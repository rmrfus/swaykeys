//! Line mechanics of the sway config format.
//!
//! This mirrors sway's own read loop (`sway/config.c` `read_config`) rather
//! than inventing a lexer, because the details are load-bearing and none of
//! them are what you would guess:
//!
//! * A trailing backslash continues the line, and the next physical line is
//!   appended *verbatim* — leading whitespace included (`getline_with_cont`).
//!   A line whose very first byte is `#` never continues.
//! * Only *whole* lines are comments. `bindsym $mod+x exec foo # note` keeps
//!   `# note` as part of the command: sway tests `line[0] == '#'` after
//!   stripping and never looks inside the line.
//! * An opening brace may sit on its own line. sway looks ahead past blank
//!   lines — but not past comments — to find it (`detect_brace`).

/// One logical line: continuations joined, whitespace stripped, never empty
/// and never a comment.
#[derive(Debug, Clone)]
pub struct LogicalLine {
    pub text: String,
    /// 1-based number of the first physical line this came from.
    pub line: u32,
    /// The contiguous run of `#` comment lines directly above, `#` and one
    /// following space removed. Empty when the line had no comment block.
    pub comment: Vec<String>,
}

/// Split config text into logical lines the way sway does.
pub fn logical_lines(content: &str) -> Vec<LogicalLine> {
    let phys: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let mut comment: Vec<String> = Vec::new();
    let mut i = 0;

    while i < phys.len() {
        let start = i;
        let mut text = phys[i].to_string();
        i += 1;

        // Backslash continuation. sway inspects the raw line, so the "not a
        // comment" test is on byte 0 — `  # x \` does continue.
        while !text.starts_with('#') && text.ends_with('\\') && i < phys.len() {
            text.pop();
            text.push_str(phys[i]);
            i += 1;
        }

        let stripped = text.trim();
        if stripped.is_empty() {
            // A blank line ends a comment block: it is no longer attached to
            // whatever comes next.
            comment.clear();
            continue;
        }
        if let Some(rest) = stripped.strip_prefix('#') {
            comment.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            continue;
        }

        let mut text = stripped.to_string();
        if !text.ends_with('{') && !text.ends_with('}') {
            if let Some(consumed) = detect_brace(&phys[i..]) {
                i += consumed;
                text.push_str(" {");
            }
        }

        out.push(LogicalLine {
            text,
            line: start as u32 + 1,
            comment: std::mem::take(&mut comment),
        });
    }
    out
}

/// Look ahead for an opening brace on its own line, as `detect_brace` does:
/// skip blank lines, and stop at the first line with any content. Returns how
/// many lines to consume, or `None` to leave the cursor untouched.
fn detect_brace(rest: &[&str]) -> Option<usize> {
    for (n, line) in rest.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        return if line == "{" { Some(n + 1) } else { None };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        logical_lines(s).into_iter().map(|l| l.text).collect()
    }

    #[test]
    fn joins_continuations_verbatim() {
        // The next line is appended as-is, so its indentation survives.
        assert_eq!(
            texts("bindsym a \\\n    exec foo"),
            ["bindsym a     exec foo"]
        );
    }

    #[test]
    fn comment_line_never_continues() {
        assert_eq!(
            texts("# note \\\nbindsym a exec foo"),
            ["bindsym a exec foo"]
        );
    }

    #[test]
    fn trailing_hash_stays_in_the_command() {
        // Not a comment as far as sway is concerned.
        assert_eq!(
            texts("bindsym a exec foo # note"),
            ["bindsym a exec foo # note"]
        );
    }

    #[test]
    fn brace_on_its_own_line_is_pulled_up() {
        assert_eq!(
            texts("mode \"resize\"\n\n{\nbindsym h resize\n}"),
            ["mode \"resize\" {", "bindsym h resize", "}",]
        );
    }

    #[test]
    fn brace_lookahead_stops_at_a_comment() {
        // A comment before the brace aborts the lookahead, so the two lines
        // stay separate — same as sway.
        let lines = texts("mode \"resize\"\n# hm\n{\n}");
        assert_eq!(lines, ["mode \"resize\"", "{", "}"]);
    }

    #[test]
    fn comment_block_attaches_to_the_next_line() {
        let lines = logical_lines("# first\n# second\nbindsym a exec foo");
        assert_eq!(lines[0].comment, ["first", "second"]);
        assert_eq!(lines[0].line, 3);
    }

    #[test]
    fn blank_line_detaches_the_comment() {
        let lines = logical_lines("# stray\n\nbindsym a exec foo");
        assert!(lines[0].comment.is_empty());
    }
}
