//! Getting the config in front of us: find it, cross-check it against the
//! running sway, and walk `include` the way sway walks it.
//!
//! The IPC is *not* a shortcut here, contrary to what it looks like.
//! `GET_CONFIG` returns the root config file byte-for-byte — it neither
//! expands `include` nor strips `set`. Measured on sway 1.12:
//!
//! ```text
//! diff ~/.config/sway/config <(swaymsg -t get_config -r | jq -r .config)  → identical
//! ```
//!
//! So it hands us contents but never a *path*, and a path is exactly what
//! `include` needs (sway resolves each include against the dirname of its
//! parent). We locate the root ourselves and use the IPC only to notice when
//! the running sway loaded something else.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::lex::{self, LogicalLine};
use crate::vars::Vars;

/// One command line of the config, variables already expanded.
#[derive(Debug, Clone)]
pub struct Directive {
    pub text: String,
    /// Enclosing block headers, outermost first — e.g. `["mode \"resize\""]`.
    pub blocks: Vec<String>,
    pub file: Rc<PathBuf>,
    pub line: u32,
    pub comment: Vec<String>,
}

impl Directive {
    /// `file:line`, with `$HOME` shortened, for diagnostics and `--json`.
    pub fn origin(&self) -> String {
        let path = self.file.display().to_string();
        let short = match std::env::var_os("HOME") {
            Some(home) if !home.is_empty() => {
                let home = home.to_string_lossy().into_owned();
                path.strip_prefix(&home)
                    .map(|r| format!("~{r}"))
                    .unwrap_or(path)
            }
            _ => path,
        };
        format!("{short}:{}", self.line)
    }
}

pub struct Config {
    pub directives: Vec<Directive>,
    /// Root config path. `None` when only the IPC could supply the text.
    pub root: Option<PathBuf>,
    /// Non-fatal complaints, for stderr.
    pub warnings: Vec<String>,
}

/// Load the config: explicit path, or locate + cross-check against sway.
pub fn load(explicit: Option<&Path>) -> Result<Config, String> {
    let mut warnings = Vec::new();

    let (root, text) = match explicit {
        // An explicit path is the whole truth: no search, no IPC. This is also
        // what makes the tool testable and lets you read someone else's config.
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            (Some(p.to_path_buf()), text)
        }
        None => {
            let found = search_path().into_iter().find(|p| p.is_file());
            match (found, ipc_get_config()) {
                (Some(path), Some(ipc)) => {
                    let disk = std::fs::read_to_string(&path).unwrap_or_default();
                    if disk != ipc {
                        warnings.push(format!(
                            "running sway loaded a config that differs from {}; \
                             using the running one, resolving includes from that directory",
                            path.display()
                        ));
                    }
                    (Some(path), ipc)
                }
                (Some(path), None) => {
                    let text = std::fs::read_to_string(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    (Some(path), text)
                }
                (None, Some(ipc)) => {
                    warnings.push(
                        "sway is running but no config file was found in the search path; \
                         relative includes cannot be resolved"
                            .into(),
                    );
                    (None, ipc)
                }
                (None, None) => {
                    return Err(
                        "no sway config found and sway is not running (try --config PATH)".into(),
                    );
                }
            }
        }
    };

    let base = root.clone().unwrap_or_else(|| PathBuf::from("config"));
    let mut walker = Walker {
        vars: Vars::default(),
        seen: HashSet::new(),
        blocks: Vec::new(),
        out: Vec::new(),
        warnings,
    };
    if let Ok(real) = std::fs::canonicalize(&base) {
        walker.seen.insert(real);
    }
    walker.walk(&base, &text);

    Ok(Config {
        directives: walker.out,
        root,
        warnings: walker.warnings,
    })
}

/// Config search order, straight out of `man sway(1)`.
fn search_path() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));

    let mut out = Vec::new();
    if let Some(h) = &home {
        out.push(h.join(".sway/config"));
    }
    if let Some(x) = &xdg {
        out.push(x.join("sway/config"));
    }
    if let Some(h) = &home {
        out.push(h.join(".i3/config"));
    }
    if let Some(x) = &xdg {
        out.push(x.join("i3/config"));
    }
    out.push(PathBuf::from("/etc/sway/config"));
    out.push(PathBuf::from("/etc/i3/config"));
    out
}

/// `GET_CONFIG` over the sway IPC socket. Speaking the protocol directly keeps
/// `swaymsg` off the runtime dependency list; the framing is six bytes of
/// magic plus two native-endian u32s (`man 7 sway-ipc`).
fn ipc_get_config() -> Option<String> {
    const GET_CONFIG: u32 = 9;

    let sock = std::env::var_os("SWAYSOCK").or_else(|| std::env::var_os("I3SOCK"))?;
    let mut stream = UnixStream::connect(sock).ok()?;

    let mut msg = Vec::with_capacity(14);
    msg.extend_from_slice(b"i3-ipc");
    msg.extend_from_slice(&0u32.to_ne_bytes());
    msg.extend_from_slice(&GET_CONFIG.to_ne_bytes());
    stream.write_all(&msg).ok()?;

    let mut header = [0u8; 14];
    stream.read_exact(&mut header).ok()?;
    if &header[..6] != b"i3-ipc" {
        return None;
    }
    let len = u32::from_ne_bytes(header[6..10].try_into().ok()?) as usize;
    // A config is text, not a stream; anything past a few MB is a bad frame.
    if len > 64 << 20 {
        return None;
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).ok()?;

    #[derive(serde::Deserialize)]
    struct Reply {
        config: String,
    }
    serde_json::from_slice::<Reply>(&payload)
        .ok()
        .map(|r| r.config)
}

struct Walker {
    vars: Vars,
    seen: HashSet<PathBuf>,
    blocks: Vec<String>,
    out: Vec<Directive>,
    warnings: Vec<String>,
}

impl Walker {
    fn walk(&mut self, path: &Path, text: &str) {
        let file = Rc::new(path.to_path_buf());
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        for line in lex::logical_lines(text) {
            match classify(&line.text) {
                Kind::BlockOpen(header) => {
                    // sway returns CMD_BLOCK before it does any variable
                    // replacement, so block headers stay unexpanded.
                    self.blocks.push(header);
                }
                Kind::BlockClose => {
                    if self.blocks.pop().is_none() {
                        self.warnings.push(format!(
                            "{}:{}: unmatched '}}'",
                            path.display(),
                            line.line
                        ));
                    }
                }
                Kind::Command => {
                    // `set` has to be recognised on the *raw* line. sway
                    // escapes the variable name before substituting so that
                    // `set $term alacritty` cannot expand into
                    // `set foot alacritty` — which would define nothing and
                    // silently lose the redefinition. Splitting the name off
                    // first gets the same result without the escaping dance.
                    if let Some(rest) = line.text.strip_prefix("set ") {
                        self.define(rest);
                        continue;
                    }
                    let text = self.vars.expand(&line.text);
                    if let Some(rest) = text.strip_prefix("include ") {
                        self.include(&dir, rest.trim(), path, &line);
                        continue;
                    }
                    self.out.push(Directive {
                        text,
                        blocks: self.blocks.clone(),
                        file: Rc::clone(&file),
                        line: line.line,
                        comment: line.comment,
                    });
                }
            }
        }
    }

    /// `set $name value…`. The name is never substituted; the value is expanded
    /// once, here, against the table as it stands at this point in the file.
    /// Nothing reaches back later — a variable redefined below does not change
    /// a value that already captured it.
    fn define(&mut self, rest: &str) {
        let Some((name, value)) = rest.trim().split_once(char::is_whitespace) else {
            return;
        };
        if !name.starts_with('$') {
            return;
        }
        let value = self.vars.expand(value.trim());
        self.vars.set(name, unquote(&value).to_string());
    }

    fn include(&mut self, dir: &Path, pattern: &str, parent: &Path, line: &LogicalLine) {
        let pattern = unquote(pattern);
        if pattern.contains("$(") || pattern.contains('`') {
            self.warnings.push(format!(
                "{}:{}: refusing to run command substitution in an include path: {pattern}",
                parent.display(),
                line.line
            ));
            return;
        }

        // sway chdir()s to the parent's directory and hands the path to
        // wordexp(), which gives tilde, $VAR and globbing.
        let expanded = shell_expand(pattern);
        let joined = if Path::new(&expanded).is_absolute() {
            expanded
        } else {
            dir.join(&expanded).to_string_lossy().into_owned()
        };

        let mut matched = Vec::new();
        match glob::glob(&joined) {
            Ok(paths) => matched.extend(paths.flatten()),
            // Not a valid pattern — treat it as a literal path, as wordexp would.
            Err(_) => matched.push(PathBuf::from(&joined)),
        }
        if matched.is_empty() {
            // A glob that matches nothing is silent in sway; a plain missing
            // file is worth saying out loud.
            if !joined.contains(['*', '?', '[']) {
                self.warnings.push(format!(
                    "{}:{}: include not found: {joined}",
                    parent.display(),
                    line.line
                ));
            }
            return;
        }
        matched.sort();

        for path in matched {
            if !path.is_file() {
                continue;
            }
            // realpath dedup, so a cycle of includes terminates.
            let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !self.seen.insert(real) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(text) => self.walk(&path, &text),
                Err(e) => self.warnings.push(format!("{}: {e}", path.display())),
            }
        }
    }
}

enum Kind {
    BlockOpen(String),
    BlockClose,
    Command,
}

/// Which of the three shapes a logical line has. sway decides this on the last
/// whitespace-separated token, so `mode "x"{` is *not* a block — the brace has
/// to be its own argument.
fn classify(text: &str) -> Kind {
    let mut tokens = text.split_whitespace();
    let first = tokens.next();
    let last = text.split_whitespace().next_back();

    match last {
        // `argc > 1` in sway: a lone `{` is an invalid command, not a block.
        Some("{") if first != Some("{") => {
            let header = text[..text.len() - 1].trim_end().to_string();
            Kind::BlockOpen(header)
        }
        Some("}") => Kind::BlockClose,
        _ => Kind::Command,
    }
}

/// Strip one layer of matching quotes, as sway does per argument.
fn unquote(s: &str) -> &str {
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// The parts of `wordexp()` we are willing to do: `~` and `$VAR`. Globbing is
/// left to the caller, and command substitution is refused outright.
fn shell_expand(s: &str) -> String {
    let s = match s.strip_prefix("~/") {
        Some(rest) => match std::env::var_os("HOME") {
            Some(home) => format!("{}/{rest}", home.to_string_lossy()),
            None => s.to_string(),
        },
        None => s.to_string(),
    };

    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        let start = i + 1;
        let braced = b.get(start) == Some(&b'{');
        let name_start = if braced { start + 1 } else { start };
        let mut end = name_start;
        while end < b.len() && (b[end].is_ascii_alphanumeric() || b[end] == b'_') {
            end += 1;
        }
        if end == name_start {
            out.push('$');
            i += 1;
            continue;
        }
        let name = &s[name_start..end];
        out.push_str(&std::env::var(name).unwrap_or_default());
        i = if braced && b.get(end) == Some(&b'}') {
            end + 1
        } else {
            end
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(s: &str) -> &'static str {
        match classify(s) {
            Kind::BlockOpen(_) => "open",
            Kind::BlockClose => "close",
            Kind::Command => "command",
        }
    }

    #[test]
    fn block_needs_the_brace_as_its_own_token() {
        assert_eq!(kind("mode \"resize\" {"), "open");
        assert_eq!(kind("mode \"resize\"{"), "command");
        assert_eq!(kind("}"), "close");
        assert_eq!(kind("{"), "command");
        assert_eq!(kind("bindsym $mod+a exec foo"), "command");
    }

    #[test]
    fn unquotes_one_layer() {
        assert_eq!(unquote("\"foo bar\""), "foo bar");
        assert_eq!(unquote("'foo'"), "foo");
        assert_eq!(unquote("foo"), "foo");
        assert_eq!(unquote("\"unbalanced"), "\"unbalanced");
    }
}
