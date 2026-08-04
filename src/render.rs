//! Output formats.
//!
//! Colour uses the 16 ANSI slots only, never truecolor: the terminal's own
//! theme then decides the actual shades, so the sheet stays readable on light
//! and dark backgrounds alike without us guessing either.

use serde::Serialize;

use crate::group::{Row, Section};
use crate::model::{Binding, Bindings};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const UNDERLINE: &str = "\x1b[4m";

/// A distinct colour per modifier, so a chord is scannable at a glance rather
/// than read left to right.
fn modifier_colour(name: &str) -> &'static str {
    match name {
        "Super" => "\x1b[35m", // magenta
        "Ctrl" => "\x1b[33m",  // yellow
        "Alt" => "\x1b[36m",   // cyan
        "Shift" => "\x1b[32m", // green
        _ => "\x1b[34m",       // blue
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("{code}{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    fn chord(&self, row: &Row) -> String {
        if !self.color {
            return row.chord();
        }
        let mut parts: Vec<String> = row
            .modifiers
            .iter()
            .map(|m| self.paint(modifier_colour(m), m))
            .collect();
        parts.push(self.paint(BOLD, &row.key));
        parts.join(&self.paint(DIM, "+"))
    }
}

/// Visible width, ignoring escape sequences.
fn width(row: &Row) -> usize {
    row.chord().chars().count()
}

/// Gap between columns.
const GUTTER: usize = 3;

/// The aligned terminal sheet.
///
/// `term_width` bounds each column when there is more than one; a single
/// column is left to wrap or overflow, because truncating a command the reader
/// asked to see is worse than a long line.
pub fn plain(sections: &[Section], style: Style, columns: usize, term_width: usize) -> String {
    let pad = sections
        .iter()
        .flat_map(|s| s.rows.iter())
        .map(width)
        .max()
        .unwrap_or(0);

    // One long `exec` line would otherwise set the width of the whole column
    // and push everything to its right off the screen.
    let text_budget = (columns > 1).then(|| {
        let per_column = term_width.saturating_sub(GUTTER * (columns - 1)) / columns;
        per_column.saturating_sub(pad + 4).max(8)
    });

    let mut lines: Vec<Line> = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line {
                section: i,
                text: String::new(),
                header: false,
            });
        }
        lines.push(Line {
            section: i,
            text: heading(&section.title, style),
            header: true,
        });
        for row in &section.rows {
            lines.push(Line {
                section: i,
                text: render_row(row, style, pad, text_budget),
                header: false,
            });
        }
    }

    if columns <= 1 {
        let body: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        return body.join("\n") + "\n";
    }
    side_by_side(&lines, sections, style, columns)
}

struct Line {
    section: usize,
    text: String,
    header: bool,
}

fn heading(title: &str, style: Style) -> String {
    style.paint(&format!("{BOLD}{UNDERLINE}"), title)
}

fn render_row(row: &Row, style: Style, pad: usize, text_budget: Option<usize>) -> String {
    let gap = " ".repeat(pad - width(row) + 2);
    // Clip before colouring, so no escape sequence is ever cut in half.
    let text = match text_budget {
        Some(max) => ellipsize(&row.text, max),
        None => row.text.clone(),
    };
    let mut out = format!("  {}{gap}{}", style.chord(row), style.paint(DIM, &text));
    if let Some(origin) = &row.shadowed_by {
        out.push_str(&style.paint(DIM, &format!("  (shadowed by {origin})")));
    }
    out
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// Balance the sheet across `columns`.
///
/// Splitting only between sections would be tidier, but useless in practice:
/// "Standard" is almost the whole sheet on any real config, so the second
/// column would sit nearly empty. Sections are cut where the balance falls and
/// the heading is repeated at the top of the next column, the way a printed
/// cheat sheet does it.
fn side_by_side(lines: &[Line], sections: &[Section], style: Style, columns: usize) -> String {
    let target = lines.len().div_ceil(columns);

    let mut groups: Vec<Vec<String>> = Vec::new();
    for (n, chunk) in lines.chunks(target).enumerate() {
        let mut column: Vec<String> = Vec::new();
        // A column that starts mid-section is unreadable without its heading.
        if n > 0 {
            if let Some(first) = chunk.first().filter(|l| !l.header) {
                if let Some(section) = sections.get(first.section) {
                    column.push(heading(&format!("{} (cont.)", section.title), style));
                }
            }
        }
        column.extend(chunk.iter().map(|l| l.text.clone()));
        groups.push(column);
    }

    let widths: Vec<usize> = groups
        .iter()
        .map(|g| g.iter().map(|l| visible_len(l)).max().unwrap_or(0))
        .collect();
    let height = groups.iter().map(|g| g.len()).max().unwrap_or(0);

    let mut out = String::new();
    for i in 0..height {
        let mut line = String::new();
        for (col, group) in groups.iter().enumerate() {
            let cell = group.get(i).map(String::as_str).unwrap_or("");
            line.push_str(cell);
            if col + 1 != groups.len() {
                line.push_str(&" ".repeat(widths[col] - visible_len(cell) + 3));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// Character count with ANSI escape sequences discounted.
fn visible_len(s: &str) -> usize {
    let mut n = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            in_escape = c != 'm';
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            n += 1;
        }
    }
    n
}

/// GitHub-flavoured markdown, one table per section, columns padded so the
/// source is readable without a renderer.
pub fn markdown(sections: &[Section]) -> String {
    let mut out = String::new();
    for section in sections {
        let key_w = section
            .rows
            .iter()
            .map(|r| r.chord().chars().count() + 2)
            .chain(std::iter::once(3))
            .max()
            .unwrap_or(3);
        let act_w = section
            .rows
            .iter()
            .map(|r| r.text.chars().count())
            .chain(std::iter::once(6))
            .max()
            .unwrap_or(6);

        out.push_str(&format!("## {}\n\n", section.title));
        out.push_str(&format!("| {:key_w$} | {:act_w$} |\n", "Key", "Action"));
        out.push_str(&format!(
            "| {} | {} |\n",
            "-".repeat(key_w),
            "-".repeat(act_w)
        ));
        for row in &section.rows {
            let key = format!("`{}`", row.chord());
            out.push_str(&format!("| {key:key_w$} | {:act_w$} |\n", row.text));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(modifiers: &[&str], key: &str, text: &str) -> Row {
        Row {
            modifiers: modifiers.iter().map(|s| s.to_string()).collect(),
            key: key.into(),
            text: text.into(),
            command: text.into(),
            origin: "t:1".into(),
            shadowed_by: None,
        }
    }

    #[test]
    fn visible_len_ignores_escapes() {
        assert_eq!(visible_len("\x1b[1mabc\x1b[0m"), 3);
        assert_eq!(visible_len("abc"), 3);
    }

    #[test]
    fn colour_does_not_disturb_alignment() {
        let sections = [Section {
            title: "Standard".into(),
            rows: vec![row(&["Super"], "Return", "exec foot"), row(&[], "a", "nop")],
        }];
        let plain_out = plain(&sections, Style { color: false }, 1, 200);
        let colour_out = plain(&sections, Style { color: true }, 1, 200);
        let strip = |s: &str| s.lines().map(visible_len).collect::<Vec<_>>();
        assert_eq!(strip(&plain_out), strip(&colour_out));
    }

    #[test]
    fn two_columns_balance_and_repeat_the_heading() {
        // One oversized section: the whole point is that it does get cut.
        let sections = [Section {
            title: "Standard".into(),
            rows: vec![row(&[], "a", "one"); 20],
        }];
        let out = plain(&sections, Style { color: false }, 2, 200);

        assert!(
            out.contains("Standard (cont.)"),
            "heading not repeated:\n{out}"
        );
        // Every row still present, and the sheet is now half as tall.
        assert_eq!(out.matches("one").count(), 20);
        assert!(
            out.lines().count() < 15,
            "not balanced: {} lines",
            out.lines().count()
        );
    }

    #[test]
    fn a_column_starting_on_a_heading_does_not_repeat_it() {
        let sections = [
            Section {
                title: "A".into(),
                rows: vec![row(&[], "a", "one"); 3],
            },
            Section {
                title: "B".into(),
                rows: vec![row(&[], "b", "two"); 3],
            },
        ];
        let out = plain(&sections, Style { color: false }, 2, 200);
        assert!(!out.contains("(cont.)"), "spurious continuation:\n{out}");
    }

    #[test]
    fn markdown_pads_its_columns() {
        let sections = [Section {
            title: "Standard".into(),
            rows: vec![row(&["Super"], "Return", "exec foot"), row(&[], "a", "nop")],
        }];
        let out = markdown(&sections);
        let widths: Vec<usize> = out
            .lines()
            .filter(|l| l.starts_with('|'))
            .map(|l| l.chars().count())
            .collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged table:\n{out}"
        );
    }
}
