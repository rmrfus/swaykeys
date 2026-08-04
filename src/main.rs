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

    /// Colour. `auto` means on a terminal only; NO_COLOR always wins.
    #[arg(long, value_enum, default_value_t = When::Auto)]
    color: When,

    /// Interactive pager. `auto` opens it on a terminal when no explicit
    /// --format was asked for.
    #[arg(long, value_enum, default_value_t = When::Auto)]
    pager: When,

    /// Lay the sections out side by side.
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

    // Asking for a format is asking for text, so an explicit --format turns the
    // pager off. Otherwise it opens on a terminal and stays out of the way in a
    // pipe, where there is nothing to interact with.
    let pager = match args.pager {
        When::Always => true,
        When::Never => false,
        When::Auto => tty && args.format == Format::Auto,
    };
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
    let format = match args.format {
        Format::Auto if tty => Format::Plain,
        Format::Auto => Format::Md,
        explicit => explicit,
    };
    // NO_COLOR is honoured whatever its value, per no-color.org.
    let color = match args.color {
        When::Always => true,
        When::Never => false,
        When::Auto => tty && std::env::var_os("NO_COLOR").is_none(),
    };
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
