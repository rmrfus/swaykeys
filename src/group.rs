//! Bindings arranged for reading: sections, descriptions, and the two things
//! that never appear as a `bind*` line but do respond to keys.

use crate::model::{Binding, Bindings, Bucket, Kind};
use crate::source::Directive;

/// A run of bindings under one heading.
pub struct Section {
    pub title: String,
    pub rows: Vec<Row>,
}

/// One line of the help sheet, already reduced to strings.
#[derive(Debug, Clone)]
pub struct Row {
    /// Modifier names, kept separate from the key so they can be coloured.
    pub modifiers: Vec<String>,
    /// The key, button, gesture or switch part.
    pub key: String,
    /// What to show: the description when asked for and available, else the
    /// raw command.
    pub text: String,
    /// Always the raw command, for the detail line in the pager.
    pub command: String,
    pub origin: String,
    /// Where the binding that beats this one is defined.
    pub shadowed_by: Option<String>,
}

impl Row {
    /// `Super+Shift+r`.
    pub fn chord(&self) -> String {
        let mut parts = self.modifiers.clone();
        parts.push(self.key.clone());
        parts.join("+")
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Opts {
    /// Include bindings that never fire.
    pub all: bool,
    /// Prefer the comment above a binding over its command.
    pub desc: bool,
}

/// Fixed section order. Modes keep the name they have in the config and appear
/// in the order they were first defined.
pub fn sections(bindings: &Bindings, directives: &[Directive], opts: Opts) -> Vec<Section> {
    let mut standard = Vec::new();
    let mut modes: Vec<(String, Vec<Row>)> = Vec::new();
    let mut mouse = Vec::new();
    let mut touchpad = Vec::new();
    let mut switches = Vec::new();

    for b in &bindings.list {
        if b.shadowed_by.is_some() && !opts.all {
            continue;
        }
        let row = row_of(b, bindings, opts);
        match (b.bucket, b.kind, b.mode.as_str()) {
            (Bucket::Gesture, _, _) => touchpad.push(row),
            (Bucket::Switch, _, _) => switches.push(row),
            (Bucket::Mouse, _, _) => mouse.push(row),
            (_, Kind::Chord, "default") => standard.push(row),
            (_, Kind::Chord, mode) => match modes.iter_mut().find(|(name, _)| name == mode) {
                Some((_, rows)) => rows.push(row),
                None => modes.push((mode.to_string(), vec![row])),
            },
            _ => standard.push(row),
        }
    }

    mouse.extend(floating_modifier_rows(directives));

    let mut out = vec![Section {
        title: "Standard".into(),
        rows: standard,
    }];
    out.extend(
        modes
            .into_iter()
            .map(|(title, rows)| Section { title, rows }),
    );
    out.push(Section {
        title: "Mouse".into(),
        rows: mouse,
    });
    out.push(Section {
        title: "Touchpad".into(),
        rows: touchpad,
    });
    out.push(Section {
        title: "Switches".into(),
        rows: switches,
    });
    out.push(Section {
        title: "Built-in".into(),
        rows: builtin_rows(),
    });
    out.retain(|s| !s.rows.is_empty());
    out
}

fn row_of(b: &Binding, all: &Bindings, opts: Opts) -> Row {
    let text = opts
        .desc
        .then(|| description(&b.comment))
        .flatten()
        .unwrap_or_else(|| b.command.clone());

    Row {
        modifiers: b.modifiers.clone(),
        // `chord` is modifiers and keys joined with `+`; strip the modifiers
        // back off rather than storing the key list twice.
        key: b
            .chord
            .splitn(b.modifiers.len() + 1, '+')
            .last()
            .unwrap_or("")
            .to_string(),
        text,
        command: b.command.clone(),
        origin: b.origin.clone(),
        shadowed_by: b.shadowed_by.map(|i| all.list[i].origin.clone()),
    }
}

/// First sentence of the comment block above a binding.
///
/// A heuristic, which is why `--desc` is opt-in: config comments are prose,
/// and prose does not promise to put the useful half first. Sentence-splitting
/// on ". " also trips over abbreviations. Wrong here costs a worse label, not a
/// wrong binding, and the raw command stays one keystroke away in the pager.
fn description(comment: &[String]) -> Option<String> {
    const MAX: usize = 72;

    let joined = comment.join(" ");
    let text = joined.trim();
    if text.is_empty() {
        return None;
    }
    let sentence = match text.find(". ") {
        Some(i) => &text[..i],
        None => text.strip_suffix('.').unwrap_or(text),
    };
    let sentence = sentence.trim();
    if sentence.is_empty() {
        return None;
    }
    if sentence.chars().count() <= MAX {
        return Some(sentence.to_string());
    }
    let cut: String = sentence.chars().take(MAX - 1).collect();
    // Prefer to break at a word boundary rather than mid-word.
    let cut = cut
        .rsplit_once(' ')
        .map_or(cut.clone(), |(head, _)| head.to_string());
    Some(format!("{cut}…"))
}

/// `floating_modifier <mod> [normal|inverse]` binds the pointer without ever
/// naming a button, so it never shows up as a `bind*` line.
fn floating_modifier_rows(directives: &[Directive]) -> Vec<Row> {
    let Some(d) = directives
        .iter()
        .filter(|d| d.blocks.is_empty())
        .rev()
        .find(|d| d.text.starts_with("floating_modifier "))
    else {
        return Vec::new();
    };

    let mut tokens = d.text.split_whitespace().skip(1);
    let Some(modifier) = tokens.next() else {
        return Vec::new();
    };
    if modifier.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    // "If inverse is specified, left click is used for resizing and right
    // click for moving" — sway(5).
    let inverse = tokens
        .next()
        .is_some_and(|m| m.eq_ignore_ascii_case("inverse"));
    let (left, right) = if inverse {
        ("resize floating window", "move floating window")
    } else {
        ("move floating window", "resize floating window")
    };

    [("left drag", left), ("right drag", right)]
        .into_iter()
        .map(|(button, action)| Row {
            modifiers: vec![pretty_modifier(modifier)],
            key: button.into(),
            text: action.into(),
            command: format!(
                "floating_modifier {}",
                d.text["floating_modifier ".len()..].trim()
            ),
            origin: d.origin(),
            shadowed_by: None,
        })
        .collect()
}

fn pretty_modifier(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "mod4" | "super" => "Super".into(),
        "mod1" | "alt" => "Alt".into(),
        "control" | "ctrl" => "Ctrl".into(),
        other => other.into(),
    }
}

/// The one binding that is compiled into sway.
///
/// `keyboard_execute_compositor_binding` (`sway/input/keyboard.c`) switches VT
/// on the `XF86Switch_VT_1..12` keysyms, which the usual layouts put on
/// Ctrl+Alt+F1..F12. It runs in the `if (!handled)` branch — *after* config
/// bindings and only when none matched — so it is a fallback, not a
/// reservation. The ticket has this backwards: a config binding on the same
/// chord does win.
fn builtin_rows() -> Vec<Row> {
    vec![Row {
        modifiers: vec!["Ctrl".into(), "Alt".into()],
        key: "F1…F12".into(),
        text: "switch to virtual terminal 1–12".into(),
        command: "compositor fallback; a config binding on the same chord wins".into(),
        origin: "built into sway".into(),
        shadowed_by: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(lines: &[&str]) -> Option<String> {
        description(&lines.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn description_takes_the_first_sentence() {
        assert_eq!(
            desc(&["Reload config on Mod+Shift+R (muscle memory). Default is Mod+Shift+C."]),
            Some("Reload config on Mod+Shift+R (muscle memory)".into())
        );
    }

    #[test]
    fn description_spans_wrapped_comment_lines() {
        assert_eq!(
            desc(&[
                "Re-home floating toggle",
                "and nothing else. Mnemonic: sibling of f."
            ]),
            Some("Re-home floating toggle and nothing else".into())
        );
    }

    #[test]
    fn description_drops_a_lone_trailing_period() {
        assert_eq!(
            desc(&["Kill the focused window."]),
            Some("Kill the focused window".into())
        );
    }

    #[test]
    fn description_truncates_at_a_word_boundary() {
        let long = "a".repeat(40) + " " + &"b".repeat(40);
        let got = desc(&[&long]).unwrap();
        assert!(got.ends_with('…'), "got {got}");
        assert!(
            got.chars().count() <= 72,
            "got {} chars",
            got.chars().count()
        );
        // Cut between the words, not inside one.
        assert!(!got.contains('b'), "got {got}");
    }

    #[test]
    fn no_comment_means_no_description() {
        assert_eq!(desc(&[]), None);
        assert_eq!(desc(&["", "  "]), None);
    }
}
