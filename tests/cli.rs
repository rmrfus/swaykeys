//! End-to-end runs of the binary.
//!
//! These exist because of a bug class the unit tests cannot see: a flag that
//! reaches no code path. `-2` was silently ignored in the default mode for a
//! while — on a terminal the pager opened and never looked at it, and in a pipe
//! the format resolved to markdown, which has no columns either. Everything
//! below asserts that an option actually changes the output.
//!
//! The helpers return `io::Result` and the tests unwrap with `?`. That is not
//! style: a plain helper in an integration file is not a `#[test]` function, so
//! clippy's test-harness exemption does not reach it and an `.expect()` here
//! would fail the build.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

/// The binary, pointed at a fixture and cut off from any running compositor:
/// `--config` already pins the input, and this keeps the IPC cross-check out of
/// the way.
fn swaykeys(config: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_swaykeys"));
    // Passed as a path, not a &str: `to_str().unwrap()` would be a panic on any
    // checkout under a non-UTF-8 path, which is legal on Unix.
    cmd.arg("--config").arg(fixture(config));
    cmd.env_remove("SWAYSOCK").env_remove("I3SOCK");
    cmd
}

/// Run against the standard fixture with stdout on a pipe, which is how the
/// tests always see it.
fn run(args: &[&str]) -> io::Result<Output> {
    swaykeys("layout.config").args(args).output()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn piped_output_is_markdown_by_default() -> io::Result<()> {
    let out = run(&[])?;
    assert!(
        stdout(&out).contains("## Standard"),
        "got:\n{}",
        stdout(&out)
    );
    Ok(())
}

#[test]
fn two_column_forces_the_plain_layout_even_in_a_pipe() -> io::Result<()> {
    // The regression: markdown tables ignore columns, so `-2 | less` used to
    // produce a single-column markdown document and no complaint.
    let out = run(&["-2"])?;
    let text = stdout(&out);
    assert!(!text.contains("## Standard"), "still markdown:\n{text}");
    assert!(text.contains("(cont.)"), "not laid out in columns:\n{text}");
    // Specifically: no complaint *about -2*. Asserting stderr is empty would
    // be wrong — unrelated diagnostics belong there, and the keymap warning
    // legitimately fires wherever libxkbcommon is absent, such as the Nix
    // build sandbox.
    assert!(
        !stderr(&out).contains("--two-column"),
        "complained: {:?}",
        stderr(&out)
    );
    Ok(())
}

#[test]
fn two_column_is_narrower_than_one() -> io::Result<()> {
    let one = stdout(&run(&["--format", "plain"])?).lines().count();
    let two = stdout(&run(&["-2"])?).lines().count();
    assert!(
        two < one,
        "two columns ({two} lines) not shorter than one ({one})"
    );
    Ok(())
}

#[test]
fn two_column_says_so_when_it_cannot_apply() -> io::Result<()> {
    // Silently doing nothing is the bug; saying so is the fix.
    let out = run(&["-2", "--format", "md"])?;
    assert!(
        stderr(&out).contains("--two-column"),
        "silent: {:?}",
        stderr(&out)
    );
    assert!(stdout(&out).contains("## Standard"));
    Ok(())
}

#[test]
fn desc_prefers_the_comment_over_the_command() -> io::Result<()> {
    let plain = stdout(&run(&["--format", "plain"])?);
    let desc = stdout(&run(&["--format", "plain", "--desc"])?);
    assert!(plain.contains("exec foot"), "got:\n{plain}");
    assert!(desc.contains("Start a terminal"), "got:\n{desc}");
    assert!(!desc.contains("exec foot"), "command still shown:\n{desc}");
    Ok(())
}

#[test]
fn mode_filter_drops_the_other_sections() -> io::Result<()> {
    let out = stdout(&run(&["--format", "plain", "--mode", "resize"])?);
    assert!(out.contains("resize"), "got:\n{out}");
    assert!(!out.contains("Standard"), "got:\n{out}");
    Ok(())
}

#[test]
fn color_can_be_forced_into_a_pipe() -> io::Result<()> {
    assert!(!stdout(&run(&["--format", "plain"])?).contains('\x1b'));
    assert!(stdout(&run(&["--format", "plain", "--color", "always"])?).contains('\x1b'));
    Ok(())
}

/// Precedence is flag, then environment: a variable from the user's profile
/// does not get to override an option they typed just now.
///
/// Only this half is testable here. The other half — NO_COLOR silencing `auto`
/// — needs a terminal, and piped output is never coloured in `auto` mode
/// whatever the environment says, so asserting it from here would pass for the
/// wrong reason. `want_color` in `main.rs` carries that test.
#[test]
fn no_color_does_not_override_an_explicit_color_flag() -> io::Result<()> {
    let out = swaykeys("layout.config")
        .args(["--format", "plain", "--color", "always"])
        // Any non-empty value counts as set, "0" included — per no-color.org.
        .env("NO_COLOR", "0")
        .output()?;
    assert!(stdout(&out).contains('\x1b'), "flag overridden");
    Ok(())
}

#[test]
fn a_missing_config_fails_loudly() -> io::Result<()> {
    let out = Command::new(env!("CARGO_BIN_EXE_swaykeys"))
        .args(["--config", "/nonexistent/sway/config"])
        .output()?;
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("swaykeys:"),
        "got: {:?}",
        stderr(&out)
    );
    Ok(())
}
