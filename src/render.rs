//! Output formats. M3 grows sections, markdown, colour and two columns onto
//! this; for now it is the plain aligned sheet and the JSON dump.

use serde::Serialize;

use crate::model::{Binding, Bindings};

/// Aligned two-column plain text, one binding per line.
///
/// `all` also lists bindings that never fire because another list wins the same
/// chord, annotated with what beats them. Hiding them is the default: a help
/// sheet answers "what does this key do", and a binding that cannot happen
/// answers it wrongly.
pub fn plain(bindings: &Bindings, all: bool) -> String {
    let shown: Vec<&Binding> = bindings
        .list
        .iter()
        .filter(|b| all || b.shadowed_by.is_none())
        .collect();
    let width = shown
        .iter()
        .map(|b| b.chord.chars().count())
        .max()
        .unwrap_or(0);

    let mut out = String::new();
    for b in shown {
        let pad = width - b.chord.chars().count();
        out.push_str(&b.chord);
        out.push_str(&" ".repeat(pad + 2));
        out.push_str(&b.command);
        if let Some(i) = b.shadowed_by {
            out.push_str("    (shadowed by ");
            out.push_str(&bindings.list[i].origin);
            out.push(')');
        }
        out.push('\n');
    }
    out
}

/// The `--json` document.
///
/// An object rather than a bare array on purpose: this is a published format,
/// and wrapping it now means a later field costs a line instead of breaking
/// every consumer that indexes `[0]`.
#[derive(Serialize)]
struct Document<'a> {
    /// Config the bindings were read from; absent when only the IPC had it.
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    bindings: &'a [Binding],
    /// Lines that looked like bindings but could not be parsed — the same text
    /// that went to stderr, so a machine consumer can see them too.
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    unparsed: &'a [String],
}

pub fn json(bindings: &Bindings, root: Option<&std::path::Path>) -> String {
    let doc = Document {
        root: root.map(|p| p.display().to_string()),
        bindings: &bindings.list,
        unparsed: &bindings.unparsed,
    };
    serde_json::to_string_pretty(&doc).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
