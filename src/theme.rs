//! One palette, two renderers.
//!
//! The static sheet writes ANSI escapes directly and the pager goes through
//! ratatui, so the two cannot share a representation — but they must share the
//! *decisions*, which live here. They did not, once: section headings came out
//! plain in the sheet and yellow in the pager, and the yellow collided with the
//! one already meaning Ctrl.
//!
//! Only the 16 ANSI slots are used, never truecolor: the terminal's own theme
//! then picks the actual shades, so the sheet stays readable on light and dark
//! backgrounds without us guessing either.
//!
//! Every colour means exactly one thing. There is no free slot left for section
//! headings, which is why they are bold and underlined instead — nothing else in
//! the sheet is underlined, so it reads as a heading without spending a colour.

/// The ANSI colours in use, and what each one means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    /// Super / the logo key.
    Magenta,
    /// Ctrl.
    Yellow,
    /// Alt.
    Cyan,
    /// Shift.
    Green,
    /// Any other modifier, and the pager's prompt.
    Blue,
    /// Reserved for "this binding never fires".
    Red,
}

impl Colour {
    /// SGR foreground escape, for the static sheet.
    pub fn ansi(self) -> &'static str {
        match self {
            Colour::Magenta => "\x1b[35m",
            Colour::Yellow => "\x1b[33m",
            Colour::Cyan => "\x1b[36m",
            Colour::Green => "\x1b[32m",
            Colour::Blue => "\x1b[34m",
            Colour::Red => "\x1b[31m",
        }
    }

    /// The same colour, for ratatui.
    pub fn ratatui(self) -> ratatui::style::Color {
        use ratatui::style::Color;
        match self {
            Colour::Magenta => Color::Magenta,
            Colour::Yellow => Color::Yellow,
            Colour::Cyan => Color::Cyan,
            Colour::Green => Color::Green,
            Colour::Blue => Color::Blue,
            Colour::Red => Color::Red,
        }
    }
}

/// Modifier names are coloured so a chord can be taken in at a glance rather
/// than read left to right.
pub fn modifier(name: &str) -> Colour {
    match name {
        "Super" => Colour::Magenta,
        "Ctrl" => Colour::Yellow,
        "Alt" => Colour::Cyan,
        "Shift" => Colour::Green,
        _ => Colour::Blue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_modifier_gets_its_own_colour() {
        let named = ["Super", "Ctrl", "Alt", "Shift"];
        let mut seen: Vec<Colour> = named.iter().map(|m| modifier(m)).collect();
        seen.sort_by_key(|c| c.ansi());
        seen.dedup();
        assert_eq!(seen.len(), named.len(), "two modifiers share a colour");
    }

    #[test]
    fn red_is_not_spent_on_a_modifier() {
        // It has to stay available to mean "never fires".
        let used: Vec<Colour> = ["Super", "Ctrl", "Alt", "Shift", "Mod3"]
            .iter()
            .map(|m| modifier(m))
            .collect();
        assert!(!used.contains(&Colour::Red));
    }

    #[test]
    fn the_two_encodings_agree() {
        use ratatui::style::Color;
        for (colour, expected) in [
            (Colour::Magenta, Color::Magenta),
            (Colour::Yellow, Color::Yellow),
            (Colour::Cyan, Color::Cyan),
            (Colour::Green, Color::Green),
            (Colour::Blue, Color::Blue),
            (Colour::Red, Color::Red),
        ] {
            assert_eq!(colour.ratatui(), expected);
            // 3x is the SGR foreground range; anything else is a typo.
            assert!(colour.ansi().starts_with("\x1b[3"), "{colour:?}");
        }
    }
}
