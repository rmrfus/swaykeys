//! End-to-end runs of the binary.
//!
//! These exist because of a bug class the unit tests cannot see: a flag that
//! reaches no code path. `-2` was silently ignored in the default mode for a
//! while — on a terminal the pager opened and never looked at it, and in a pipe
//! the format resolved to markdown, which has no columns either. Everything
//! below asserts that an option actually changes the output.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect()
}

/// Run the binary with stdout on a pipe, which is how the tests always see it.
fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swaykeys"))
        .args(["--config", fixture("layout.config").to_str().unwrap()])
        .args(args)
        // No compositor in CI, and none wanted: --config already pins the
        // input, and this keeps the IPC cross-check out of the way.
        .env_remove("SWAYSOCK")
        .env_remove("I3SOCK")
        .output()
        .expect("binary should run")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn piped_output_is_markdown_by_default() {
    let out = run(&[]);
    assert!(
        stdout(&out).contains("## Standard"),
        "got:\n{}",
        stdout(&out)
    );
}

#[test]
fn two_column_forces_the_plain_layout_even_in_a_pipe() {
    // The regression: markdown tables ignore columns, so `-2 | less` used to
    // produce a single-column markdown document and no complaint.
    let out = run(&["-2"]);
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
}

#[test]
fn two_column_is_narrower_than_one() {
    let one = stdout(&run(&["--format", "plain"])).lines().count();
    let two = stdout(&run(&["-2"])).lines().count();
    assert!(
        two < one,
        "two columns ({two} lines) not shorter than one ({one})"
    );
}

#[test]
fn two_column_says_so_when_it_cannot_apply() {
    // Silently doing nothing is the bug; saying so is the fix.
    let out = run(&["-2", "--format", "md"]);
    assert!(
        stderr(&out).contains("--two-column"),
        "silent: {:?}",
        stderr(&out)
    );
    assert!(stdout(&out).contains("## Standard"));
}

#[test]
fn desc_prefers_the_comment_over_the_command() {
    let plain = stdout(&run(&["--format", "plain"]));
    let desc = stdout(&run(&["--format", "plain", "--desc"]));
    assert!(plain.contains("exec foot"), "got:\n{plain}");
    assert!(desc.contains("Start a terminal"), "got:\n{desc}");
    assert!(!desc.contains("exec foot"), "command still shown:\n{desc}");
}

#[test]
fn mode_filter_drops_the_other_sections() {
    let out = stdout(&run(&["--format", "plain", "--mode", "resize"]));
    assert!(out.contains("resize"), "got:\n{out}");
    assert!(!out.contains("Standard"), "got:\n{out}");
}

#[test]
fn color_can_be_forced_into_a_pipe() {
    assert!(!stdout(&run(&["--format", "plain"])).contains('\x1b'));
    assert!(stdout(&run(&["--format", "plain", "--color", "always"])).contains('\x1b'));
}

/// Precedence is flag, then environment, then default: a variable from the
/// user's profile does not get to override an option they typed just now.
#[test]
fn no_color_disables_auto_but_not_an_explicit_request() {
    let with_no_color = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_swaykeys"))
            .args(["--config", fixture("layout.config").to_str().unwrap()])
            .args(args)
            // Any value counts as set, including "0" — per no-color.org.
            .env("NO_COLOR", "0")
            .env_remove("SWAYSOCK")
            .output()
            .expect("binary should run");
        stdout(&out).contains('\x1b')
    };

    assert!(!with_no_color(&["--format", "plain"]), "NO_COLOR ignored");
    assert!(
        with_no_color(&["--format", "plain", "--color", "always"]),
        "flag overridden"
    );
}

#[test]
fn a_missing_config_fails_loudly() {
    let out = Command::new(env!("CARGO_BIN_EXE_swaykeys"))
        .args(["--config", "/nonexistent/sway/config"])
        .output()
        .expect("binary should run");
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("swaykeys:"),
        "got: {:?}",
        stderr(&out)
    );
}
