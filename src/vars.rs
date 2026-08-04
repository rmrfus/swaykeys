//! `set $var value` and the substitution that goes with it.
//!
//! Mirrors `do_var_replacement` (`sway/config.c`) and `cmd_set`
//! (`sway/commands/set.c`). Three things there are easy to get wrong:
//!
//! * Symbols are kept sorted longest-name-first and matched as a *prefix* of
//!   the text after `$`, so `$mod1` wins over `$mod` — there is no word
//!   boundary involved.
//! * Substitution is a single left-to-right pass and scanning resumes *after*
//!   the inserted value, so a value containing `$foo` is not re-expanded.
//!   (The prototype's five fixpoint passes invent expansions sway won't do.)
//! * `set` expands its *value* against the table as it stands at that point in
//!   the file, then stores the result. Later redefinition of a referenced
//!   variable does not reach back. Order in the file is the whole story.

/// Variable table, kept in sway's longest-name-first order.
#[derive(Debug, Default, Clone)]
pub struct Vars {
    syms: Vec<(String, String)>,
}

impl Vars {
    /// Define or redefine `$name`. The value is stored as given — expand it
    /// first if it came from a config line.
    pub fn set(&mut self, name: &str, value: String) {
        if let Some(slot) = self.syms.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
            return;
        }
        self.syms.push((name.to_string(), value));
        // Longest first, so a prefix match always picks the most specific.
        self.syms
            .sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    }

    /// Substitute every `$var` in `s`.
    pub fn expand(&self, s: &str) -> String {
        let b = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;

        while i < b.len() {
            if b[i] != b'$' {
                // Push whole non-`$` runs; keeps this O(n) over UTF-8 too,
                // since `$` can never appear inside a multi-byte sequence.
                let start = i;
                while i < b.len() && b[i] != b'$' {
                    i += 1;
                }
                out.push_str(&s[start..i]);
                continue;
            }

            // `\$` is escaped — sway leaves the backslash in place and moves
            // on. `\\$` is a literal backslash, so the `$` is live again.
            let escaped = i > 0 && b[i - 1] == b'\\' && !(i > 1 && b[i - 2] == b'\\');
            if escaped {
                out.push('$');
                i += 1;
                continue;
            }
            // `$$` collapses to one literal `$` and is not a variable.
            if b.get(i + 1) == Some(&b'$') {
                out.push('$');
                i += 2;
                continue;
            }

            match self
                .syms
                .iter()
                .find(|(n, _)| s[i..].starts_with(n.as_str()))
            {
                Some((name, value)) => {
                    out.push_str(value);
                    // Resume *after* the value: substituted text is not rescanned.
                    i += name.len();
                }
                None => {
                    out.push('$');
                    i += 1;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        let mut v = Vars::default();
        for (n, val) in pairs {
            v.set(n, val.to_string());
        }
        v
    }

    #[test]
    fn substitutes_without_word_boundaries() {
        let v = vars(&[("$mod", "Mod4")]);
        assert_eq!(
            v.expand("bindsym $mod+Shift+r reload"),
            "bindsym Mod4+Shift+r reload"
        );
    }

    #[test]
    fn longest_name_wins() {
        // Insertion order deliberately puts the short name first.
        let v = vars(&[("$mod", "Mod4"), ("$mode2", "Mod1")]);
        assert_eq!(v.expand("$mode2"), "Mod1");
        assert_eq!(v.expand("$mod"), "Mod4");
    }

    #[test]
    fn does_not_rescan_substituted_text() {
        let v = vars(&[("$a", "$b"), ("$b", "boom")]);
        assert_eq!(v.expand("$a"), "$b");
    }

    #[test]
    fn redefinition_replaces_in_place() {
        let mut v = vars(&[("$term", "foot")]);
        v.set("$term", "alacritty".into());
        assert_eq!(v.expand("exec $term"), "exec alacritty");
    }

    #[test]
    fn escapes() {
        let v = vars(&[("$mod", "Mod4")]);
        assert_eq!(v.expand(r"\$mod"), r"\$mod");
        assert_eq!(v.expand("$$mod"), "$mod");
        // A literal backslash leaves the `$` live.
        assert_eq!(v.expand(r"\\$mod"), r"\\Mod4");
    }

    #[test]
    fn unknown_variable_is_left_alone() {
        let v = vars(&[("$mod", "Mod4")]);
        assert_eq!(v.expand("$menu and $mod"), "$menu and Mod4");
    }
}
