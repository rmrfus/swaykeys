use std::ffi::OsStr;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use swaykeys::model::Resolver;
use swaykeys::{group, model, render, source, tui, xkb};

/// Help sheet for every active sway key binding.
///
/// Parses the sway config the way sway does — following includes, expanding
/// variables, tracking mode blocks — and lists the bindings that actually fire.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Read this config instead of locating the running one.
    #[arg(short, long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Output format. `auto` is the aligned sheet on a terminal, markdown when
    /// piped.
    #[arg(long, value_enum, default_value_t = Format::Auto)]
    format: Format,

    /// Colour. `auto` means on a terminal only, and NO_COLOR disables it —
    /// but an explicit `always` wins over both.
    #[arg(long, value_enum, default_value_t = When::Auto)]
    color: When,

    /// Interactive pager. `auto` opens it on a terminal when no explicit
    /// --format was asked for.
    #[arg(long, value_enum, default_value_t = When::Auto)]
    pager: When,

    /// Lay the sections out side by side. Implies --pager never, since the
    /// pager scrolls instead of packing the sheet onto one screen.
    #[arg(short = '2', long)]
    two_column: bool,

    /// Show the comment above a binding instead of its command.
    #[arg(long)]
    desc: bool,

    /// Also show bindings that never fire because another one wins the chord.
    #[arg(long)]
    all: bool,

    /// Only this binding mode.
    #[arg(long, value_name = "NAME")]
    mode: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Auto,
    Plain,
    Md,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum When {
    Auto,
    Always,
    Never,
}

/// Colour on: asked for explicitly, or a terminal that NO_COLOR has not
/// silenced.
///
/// no-color.org asks for *present and non-empty*, not merely present. A bare
/// `is_none()` would also disagree with `anstream`, which clap pulls in for
/// `--help`: under `NO_COLOR=` our sheet would go monochrome while clap's help
/// stayed coloured.
///
/// Split out because the terminal half is untestable from `tests/` — piped
/// output is never coloured in `auto` mode whatever the environment says, so an
/// end-to-end assertion about NO_COLOR passes for the wrong reason.
fn want_color(when: When, tty: bool, no_color: Option<&OsStr>) -> bool {
    match when {
        When::Always => true,
        When::Never => false,
        When::Auto => tty && !matches!(no_color, Some(v) if !v.is_empty()),
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let tty = std::io::stdout().is_terminal();

    let config = match source::load(args.config.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("swaykeys: {e}");
            return ExitCode::FAILURE;
        }
    };
    for w in &config.warnings {
        eprintln!("swaykeys: {w}");
    }

    // Compile the keymap the config asks for. Without libxkbcommon we still
    // produce a help sheet — raw keycodes, and no shadow claims.
    let settings = xkb::Settings::from_directives(&config.directives);
    let keymap = xkb::Keymap::new(&settings);
    if keymap.is_none() {
        eprintln!(
            "swaykeys: could not compile an xkb keymap (layout {}); \
             showing raw keycodes and skipping shadow detection",
            settings.layout.as_deref().unwrap_or("default")
        );
    }
    let resolver: &dyn Resolver = match &keymap {
        Some(k) => k,
        None => &model::Optimistic,
    };

    let mut bindings = model::build(&config.directives, resolver);
    if keymap.is_some() {
        xkb::mark_shadowed(&mut bindings.list, resolver);
    }
    if let Some(mode) = &args.mode {
        bindings.list.retain(|b| &b.mode == mode);
    }

    // Anything that looked like a binding but did not parse goes to stderr.
    // A help sheet that silently drops a line is worse than none at all.
    for line in &bindings.unparsed {
        eprintln!("{line}");
    }

    let opts = group::Opts {
        all: args.all,
        desc: args.desc,
    };
    let sections = group::sections(&bindings, &config.directives, opts);

    // Asking for a particular layout is asking for text, so an explicit
    // --format or -2 turns the pager off. Otherwise it opens on a terminal and
    // stays out of the way in a pipe, where there is nothing to interact with.
    //
    // -2 counts because two columns are a way to fit a whole sheet on one
    // screen, which is a static-output problem; the pager answers it by
    // scrolling and filtering instead. Leaving -2 out of this rule made it a
    // flag that did nothing at all in the default mode.
    let pager = match args.pager {
        When::Always => true,
        When::Never => false,
        When::Auto => tty && args.format == Format::Auto && !args.two_column,
    };
    if pager && args.two_column {
        eprintln!("swaykeys: --two-column does not apply to the pager; ignoring it");
    }
    if pager {
        return match tui::run(&sections) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("swaykeys: {e}");
                ExitCode::FAILURE
            }
        };
    }

    // Piped output is for reading elsewhere — a README, an issue — so markdown
    // is the useful default there, and the aligned sheet is for the terminal.
    // -2 overrides that: it is a request for a particular text layout, and
    // markdown tables have no use for columns, so `swaykeys -2 | less` should
    // still give you two columns.
    let format = match args.format {
        Format::Auto if tty || args.two_column => Format::Plain,
        Format::Auto => Format::Md,
        explicit => explicit,
    };
    if args.two_column && format != Format::Plain {
        eprintln!("swaykeys: --two-column only applies to the plain format; ignoring it");
    }
    let color = want_color(args.color, tty, std::env::var_os("NO_COLOR").as_deref());
    let columns = if args.two_column { 2 } else { 1 };

    let out = match format {
        Format::Plain | Format::Auto => {
            // 100 when there is no terminal to ask, which only matters for the
            // column budget — a single column never truncates.
            let term_width = ratatui::crossterm::terminal::size().map_or(100, |(w, _)| w as usize);
            render::plain(&sections, render::Style { color }, columns, term_width)
        }
        Format::Md => render::markdown(&sections),
        Format::Json => render::json(&bindings, config.root.as_deref()),
    };

    // Write directly so a closed pipe (`swaykeys | head`) exits cleanly instead
    // of panicking the way the print! macros do.
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(out.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("swaykeys: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{When, want_color};
    use std::ffi::OsStr;

    #[test]
    fn no_color_needs_a_value_to_mean_anything() {
        assert!(want_color(When::Auto, true, None));
        // Set but empty is not set: the spec says present *and non-empty*, and
        // anstream — which clap uses for --help — reads it that way too.
        assert!(want_color(When::Auto, true, Some(OsStr::new(""))));
        assert!(!want_color(When::Auto, true, Some(OsStr::new("1"))));
        // Any non-empty value counts, "0" included.
        assert!(!want_color(When::Auto, true, Some(OsStr::new("0"))));
    }

    /// Precedence is flag, then environment, then default: a variable from the
    /// user's profile does not get to override an option they typed just now.
    #[test]
    fn an_explicit_color_flag_outranks_both_the_terminal_and_no_color() {
        assert!(want_color(When::Always, false, Some(OsStr::new("1"))));
        assert!(!want_color(When::Never, true, None));
    }

    #[test]
    fn auto_stays_off_when_there_is_no_terminal() {
        assert!(!want_color(When::Auto, false, None));
    }
}
