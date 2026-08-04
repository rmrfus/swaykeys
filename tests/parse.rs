//! Fixture-driven tests for the config reader.
//!
//! Each fixture in `tests/fixtures/` isolates one thing sway does that a naive
//! line-grepping parser gets wrong. They run without a compositor.

use std::path::PathBuf;

use swaykeys::model::{self, Binding, Bindings, Bucket, Optimistic};
use swaykeys::source;

fn load(name: &str) -> Bindings {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    let config = source::load(Some(&path)).expect("fixture should load");
    model::build(&config.directives, &Optimistic)
}

/// Commands in config order — the shape most assertions want.
fn commands(b: &Bindings) -> Vec<&str> {
    b.list.iter().map(|x| x.command.as_str()).collect()
}

fn find<'a>(b: &'a Bindings, chord: &str) -> Vec<&'a Binding> {
    b.list.iter().filter(|x| x.chord == chord).collect()
}

#[test]
fn includes_are_followed_and_cycles_terminate() {
    let b = load("include.config");
    let mut got = commands(&b);
    got.sort_unstable();
    assert_eq!(
        got,
        [
            "exec from-glob-a",
            "exec from-glob-b",
            "exec from-nested",
            "exec from-root"
        ]
    );
}

#[test]
fn variables_carry_across_include_boundaries() {
    let b = load("include.config");
    // $mod is set in the parent, used in the child.
    assert_eq!(find(&b, "Super+n")[0].command, "exec from-nested");
}

#[test]
fn longest_variable_name_wins() {
    let b = load("vars.config");
    // $mod2 must not be read as $mod followed by a literal "2".
    assert_eq!(find(&b, "Alt+t")[0].command, "exec foot");
}

#[test]
fn values_are_captured_at_definition_time() {
    let b = load("vars.config");
    // `set $indirect $term` captured "foot". Redefining $term below does not
    // reach back into it — sway expands once, there is no fixpoint pass.
    assert_eq!(find(&b, "Super+i")[0].command, "exec foot");
}

#[test]
fn redefinition_applies_from_that_point_on() {
    let b = load("vars.config");
    assert_eq!(find(&b, "Super+t")[0].command, "exec alacritty");
}

#[test]
fn unknown_variables_are_left_alone() {
    let b = load("vars.config");
    assert_eq!(find(&b, "Super+u")[0].command, "exec $nosuchvar");
}

#[test]
fn double_dollar_is_a_literal() {
    let b = load("vars.config");
    assert_eq!(find(&b, "Super+d")[0].command, "exec echo $mod");
}

#[test]
fn a_nested_block_does_not_end_the_mode() {
    let b = load("modes.config");
    let escape = find(&b, "Escape");
    // `Escape` sits after an `input { … }` block inside `mode "resize"`.
    // Brace counting that resets on every `}` puts it in "default".
    assert_eq!(escape[0].mode, "resize");
}

#[test]
fn brace_on_its_own_line_opens_the_mode() {
    let b = load("modes.config");
    assert_eq!(find(&b, "Super+Pause")[0].mode, "passthrough");
}

#[test]
fn bar_bindings_are_not_key_bindings() {
    let b = load("modes.config");
    assert!(find(&b, "button2").is_empty());
}

#[test]
fn mode_ends_at_its_closing_brace() {
    let b = load("modes.config");
    assert_eq!(find(&b, "Super+q")[0].mode, "default");
}

#[test]
fn continuations_keep_the_whole_command() {
    let b = load("continuation.config");
    let exit = find(&b, "Super+e");
    assert!(
        exit[0].command.ends_with("really exit?'"),
        "got: {}",
        exit[0].command
    );
    assert!(exit[0].command.contains("-t warning"));
}

#[test]
fn a_comment_never_continues() {
    let b = load("continuation.config");
    assert_eq!(find(&b, "Super+c")[0].command, "exec after-comment");
}

#[test]
fn unbind_only_reaches_its_own_list() {
    let b = load("unbind.config");
    // $mod+space was a plain bindsym, so unbindsym removes it…
    assert!(find(&b, "Super+space").is_empty());
    // …the keycode binding is untouched…
    assert_eq!(find(&b, "65")[0].command, "exec by-keycode");
    // …and `unbindsym $mod+f` misses, because --to-code filed that one under
    // keycodes.
    let f = find(&b, "Super+f");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].bucket, Bucket::Keycode);
}

#[test]
fn broken_bindings_are_reported_not_dropped() {
    let b = load("garbage.config");
    assert_eq!(commands(&b), ["exec good", "exec also-good"]);
    assert_eq!(b.unparsed.len(), 4, "got: {:#?}", b.unparsed);
    // The report has to say where, or it is useless.
    assert!(b
        .unparsed
        .iter()
        .all(|u| u.starts_with("UNPARSED ") && u.contains("garbage.config:")));
}
