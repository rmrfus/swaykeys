use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};
use swaykeys::model::Resolver;
use swaykeys::{model, render, source, xkb};

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

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Plain)]
    format: Format,

    /// Also show bindings that never fire because another one wins the chord.
    #[arg(long)]
    all: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Plain,
    Json,
}

fn main() -> ExitCode {
    let args = Args::parse();

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
    // Anything that looked like a binding but did not parse goes to stderr.
    // A help sheet that silently drops a line is worse than none at all.
    for line in &bindings.unparsed {
        eprintln!("{line}");
    }

    let out = match args.format {
        Format::Plain => render::plain(&bindings, args.all),
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
