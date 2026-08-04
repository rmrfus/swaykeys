//! The interactive sheet: type to filter, sections stay put.
//!
//! Scrolling is managed here rather than by `List`/`ListState`, because the
//! one behaviour this pager exists for — section headings surviving a filter,
//! and the current heading sticking to the top once it scrolls away — needs to
//! know exactly which entries are on screen. ratatui does the layout, styling
//! and double buffering; the viewport arithmetic is ours.
//!
//! Keys follow fzf, because that is the muscle memory this replaces: typing
//! filters, arrows or Ctrl-N/Ctrl-P move, Esc leaves. `q` is *not* quit — it
//! goes into the filter, same as in fzf.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::{DefaultTerminal, Frame};

use crate::group::{Row, Section};

/// One line of the scrollable area.
enum Entry<'a> {
    Heading(&'a str),
    Row { section: usize, row: &'a Row },
}

impl Entry<'_> {
    fn is_row(&self) -> bool {
        matches!(self, Entry::Row { .. })
    }
}

pub fn run(sections: &[Section]) -> std::io::Result<()> {
    // `run` installs a panic hook and restores the terminal on the way out, so
    // a panic mid-draw does not leave the shell in raw mode.
    ratatui::run(|terminal| {
        let mut app = App::new(sections);
        app.event_loop(terminal)
    })
}

struct App<'a> {
    sections: &'a [Section],
    filter: String,
    entries: Vec<Entry<'a>>,
    /// Index into `entries`; always points at a row, never a heading.
    cursor: usize,
    /// First visible entry.
    offset: usize,
    matcher: Matcher,
    total: usize,
    /// Width of the chord column, from the data rather than a guess — real
    /// configs carry names like `XF86AudioLowerVolume`.
    key_width: usize,
}

impl<'a> App<'a> {
    fn new(sections: &'a [Section]) -> App<'a> {
        let total = sections.iter().map(|s| s.rows.len()).sum();
        let key_width = sections
            .iter()
            .flat_map(|s| s.rows.iter())
            .map(|r| r.chord().chars().count())
            .max()
            .unwrap_or(0);
        let mut app = App {
            sections,
            filter: String::new(),
            entries: Vec::new(),
            cursor: 0,
            offset: 0,
            matcher: Matcher::new(Config::DEFAULT),
            total,
            key_width,
        };
        app.refilter();
        app
    }

    /// Rebuild the visible entry list.
    ///
    /// A heading is kept only when at least one of its rows matched, which is
    /// what makes the sections survive filtering instead of collapsing into a
    /// flat list of hits.
    fn refilter(&mut self) {
        let pattern = Pattern::parse(&self.filter, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();

        self.entries.clear();
        for (i, section) in self.sections.iter().enumerate() {
            let mut hits: Vec<(u32, &Row)> = Vec::new();
            for row in &section.rows {
                // Match against everything on the line plus the section name,
                // so "touchpad" or "resize" narrows to a section by name.
                let hay = format!("{} {} {}", section.title, row.chord(), row.text);
                let score = if self.filter.is_empty() {
                    Some(0)
                } else {
                    pattern.score(Utf32Str::new(&hay, &mut buf), &mut self.matcher)
                };
                if let Some(score) = score {
                    hits.push((score, row));
                }
            }
            if hits.is_empty() {
                continue;
            }
            // Best matches first *within* a section; section order is fixed, or
            // the headings would stop meaning anything.
            if !self.filter.is_empty() {
                hits.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            }
            self.entries.push(Entry::Heading(&section.title));
            self.entries.extend(
                hits.into_iter()
                    .map(|(_, row)| Entry::Row { section: i, row }),
            );
        }

        self.cursor = self.first_row().unwrap_or(0);
        self.offset = 0;
    }

    fn first_row(&self) -> Option<usize> {
        self.entries.iter().position(Entry::is_row)
    }

    fn matches(&self) -> usize {
        self.entries.iter().filter(|e| e.is_row()).count()
    }

    /// Move the cursor by `delta` rows, skipping headings.
    fn move_cursor(&mut self, delta: isize) {
        let mut i = self.cursor as isize;
        let mut remaining = delta.abs();
        let step = delta.signum();
        while remaining > 0 {
            i += step;
            if i < 0 || i as usize >= self.entries.len() {
                return;
            }
            if self.entries[i as usize].is_row() {
                remaining -= 1;
            }
        }
        self.cursor = i as usize;
    }

    fn jump(&mut self, to_end: bool) {
        let found = if to_end {
            self.entries.iter().rposition(Entry::is_row)
        } else {
            self.first_row()
        };
        if let Some(i) = found {
            self.cursor = i;
        }
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;

            let Event::Key(key) = event::read()? else {
                continue;
            };
            // Windows sends a Release for every Press; ignore anything but the
            // press so each keystroke counts once.
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            let page = self.page_size();

            match key.code {
                KeyCode::Esc => return Ok(()),
                KeyCode::Char('c' | 'd') if ctrl => return Ok(()),
                KeyCode::Enter => return Ok(()),

                KeyCode::Char('n') if ctrl => self.move_cursor(1),
                KeyCode::Char('p') if ctrl => self.move_cursor(-1),
                KeyCode::Char('u') if ctrl => {
                    self.filter.clear();
                    self.refilter();
                }
                KeyCode::Char('w') if ctrl => {
                    let trimmed = self.filter.trim_end();
                    let cut = trimmed.rfind(' ').map_or(0, |i| i + 1);
                    self.filter.truncate(cut);
                    self.refilter();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.refilter();
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.refilter();
                }

                KeyCode::Down => self.move_cursor(1),
                KeyCode::Up => self.move_cursor(-1),
                KeyCode::PageDown => self.move_cursor(page),
                KeyCode::PageUp => self.move_cursor(-page),
                KeyCode::Home => self.jump(false),
                KeyCode::End => self.jump(true),
                _ => {}
            }
        }
    }

    /// Last known list height, for PageUp/PageDown. Set during draw.
    fn page_size(&self) -> isize {
        PAGE.with(|p| p.get()).max(1)
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [list, detail, prompt] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        // The top line is always the sticky slot, blank when the current
        // heading is still visible inline. Reserving it unconditionally keeps
        // the body height constant: making it appear and disappear would shunt
        // the whole list up and down by a row every time you cross a section,
        // and would leave `scroll_into_view` computing against the wrong height.
        let body_height = (list.height as usize).saturating_sub(1);
        PAGE.with(|p| p.set(body_height as isize));
        self.scroll_into_view(body_height);

        let mut lines: Vec<Line> = vec![match self.sticky_heading() {
            Some(title) => heading_line(&format!("{title} ↑")),
            None => Line::default(),
        }];
        for (i, entry) in self
            .entries
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(body_height)
        {
            lines.push(match entry {
                Entry::Heading(title) => heading_line(title),
                Entry::Row { row, .. } => row_line(row, i == self.cursor, self.key_width),
            });
        }
        frame.render_widget(Paragraph::new(lines), list);

        frame.render_widget(Paragraph::new(self.detail_lines()), detail);
        frame.render_widget(Paragraph::new(self.prompt_line()), prompt);
    }

    fn scroll_into_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        // Pull the heading above the cursor into view when possible, so the
        // first thing scrolled to is never a bare row.
        let want = self.cursor.saturating_sub(1);
        if want < self.offset {
            self.offset = want;
        }
        if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
        self.offset = self.offset.min(self.entries.len().saturating_sub(1));
    }

    /// The section the cursor is in, when its heading has scrolled off the top.
    fn sticky_heading(&self) -> Option<&str> {
        let Entry::Row { section, .. } = self.entries.get(self.cursor)? else {
            return None;
        };
        let heading = self.entries[..self.cursor]
            .iter()
            .rposition(|e| matches!(e, Entry::Heading(_)))?;
        (heading < self.offset).then(|| self.sections[*section].title.as_str())
    }

    fn detail_lines(&self) -> Vec<Line<'_>> {
        let Some(Entry::Row { row, .. }) = self.entries.get(self.cursor) else {
            return Vec::new();
        };
        let mut origin = vec![Span::styled(
            row.origin.clone(),
            Style::default().add_modifier(Modifier::DIM),
        )];
        if let Some(winner) = &row.shadowed_by {
            origin.push(Span::styled(
                format!("  — never fires; {winner} wins this chord"),
                Style::default().fg(Color::Red),
            ));
        }
        vec![Line::from(row.command.clone()), Line::from(origin)]
    }

    fn prompt_line(&self) -> Line<'_> {
        let matched = self.matches();
        Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(self.filter.clone()),
            Span::styled("█", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            Span::styled(
                format!("   {matched}/{}", self.total),
                Style::default().add_modifier(Modifier::DIM),
            ),
        ])
    }
}

fn heading_line(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))
}

fn row_line(row: &Row, selected: bool, key_width: usize) -> Line<'static> {
    // Under `--all` a row that never fires has to look different at a glance,
    // not just in the detail line — otherwise the list reads as a list of live
    // bindings with impostors mixed in.
    let dead = row.shadowed_by.is_some();
    let key_style = if dead {
        Style::default().add_modifier(Modifier::DIM | Modifier::CROSSED_OUT)
    } else {
        Style::default().add_modifier(Modifier::BOLD)
    };

    let mut spans = vec![Span::raw(if selected { "> " } else { "  " })];
    for m in &row.modifiers {
        let style = if dead {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(modifier_colour(m))
        };
        spans.push(Span::styled(m.clone(), style));
        spans.push(Span::styled(
            "+",
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    spans.push(Span::styled(row.key.clone(), key_style));

    // Pad against the whole chord, since the modifiers are separate spans and
    // only their combined width lines the commands up.
    let used = row.chord().chars().count();
    spans.push(Span::raw(" ".repeat(key_width.saturating_sub(used) + 2)));
    spans.push(Span::styled(
        row.text.clone(),
        Style::default().add_modifier(Modifier::DIM),
    ));

    let line = Line::from(spans);
    if selected {
        line.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn modifier_colour(name: &str) -> Color {
    match name {
        "Super" => Color::Magenta,
        "Ctrl" => Color::Yellow,
        "Alt" => Color::Cyan,
        "Shift" => Color::Green,
        _ => Color::Blue,
    }
}

thread_local! {
    /// List height from the last draw, so PageUp/PageDown move by a screen
    /// without the event loop having to know about layout.
    static PAGE: std::cell::Cell<isize> = const { std::cell::Cell::new(20) };
}
