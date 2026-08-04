//! Cross-list shadowing, which needs a real keymap.
//!
//! These skip rather than fail when libxkbcommon is unavailable: `cargo test`
//! on a bare machine should not go red for a reason that is not the code's
//! fault. CI installs the library so they do run there.

use std::path::PathBuf;

use swaykeys::model::{self, Binding, Bindings, Bucket};
use swaykeys::{source, xkb};

/// `None` when there is no usable libxkbcommon on this host.
fn load(name: &str) -> Option<Bindings> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    let config = source::load(Some(&path)).expect("fixture should load");
    let settings = xkb::Settings::from_directives(&config.directives);
    let keymap = xkb::Keymap::new(&settings)?;

    let mut bindings = model::build(&config.directives, &keymap);
    xkb::mark_shadowed(&mut bindings.list, &keymap);
    Some(bindings)
}

macro_rules! keymap_or_skip {
    ($name:expr) => {
        match load($name) {
            Some(b) => b,
            None => {
                eprintln!("skipping: libxkbcommon unavailable");
                return;
            }
        }
    };
}

fn by_command<'a>(b: &'a Bindings, command: &str) -> &'a Binding {
    b.list
        .iter()
        .find(|x| x.command == command)
        .unwrap_or_else(|| panic!("no binding runs {command:?}; have {:#?}", commands(b)))
}

fn commands(b: &Bindings) -> Vec<&str> {
    b.list.iter().map(|x| x.command.as_str()).collect()
}

#[test]
fn to_code_shadows_the_plain_bindsym_it_never_replaced() {
    let b = keymap_or_skip!("shadow.config");
    let loser = by_command(&b, "exec default-menu");
    let winner = by_command(&b, "exec our-menu");

    // Both survive parsing — they were never in the same list…
    assert_eq!(loser.bucket, Bucket::Keysym);
    assert_eq!(winner.bucket, Bucket::Keycode);
    // …and the keycode list is queried first, so only one of them ever fires.
    assert_eq!(
        loser.shadowed_by.map(|i| b.list[i].command.as_str()),
        Some("exec our-menu")
    );
    assert_eq!(winner.shadowed_by, None);
}

#[test]
fn release_bindings_are_a_separate_lookup() {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec on-release").shadowed_by, None);
}

#[test]
fn device_specific_bindings_are_never_ranked() {
    let b = keymap_or_skip!("shadow.config");
    // Which of these wins depends on the device the event came from. Claiming
    // either would be a guess dressed as a fact.
    assert_eq!(by_command(&b, "exec device-specific").shadowed_by, None);
    assert_eq!(by_command(&b, "exec generic").shadowed_by, None);
}

#[test]
fn a_keycode_and_its_keysym_name_are_one_binding() {
    let b = keymap_or_skip!("shadow.config");
    // `bindcode 51` and `--to-code backslash` compare equal after translation,
    // so this is an overwrite, not two entries.
    assert!(
        !commands(&b).contains(&"exec by-raw-code"),
        "have: {:#?}",
        commands(&b)
    );
    by_command(&b, "exec by-name");
}

#[test]
fn bindcode_is_shown_by_name() {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec by-name").chord, "Super+backslash");
}

#[test]
fn an_uncontested_binding_is_not_marked() {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec lonely").shadowed_by, None);
}

/// The case neither the ticket nor the prototype anticipated: `--to-code` can
/// *fail*, and then the binding lands in the keysym list after all — where it
/// does collide with a plain `bindsym`.
#[test]
fn to_code_that_cannot_translate_stays_a_keysym_binding() {
    let b = keymap_or_skip!("untranslatable.config");
    // Print sits on two keycodes in the stock us layout (PRSC and I218), so
    // sway's single-match test fails and the binding never reaches the keycode
    // list. Verified independently with `xkbcli compile-keymap --layout us`.
    let print = by_command(&b, "exec ours");
    assert_eq!(print.bucket, Bucket::Keysym);
    // Same list as the upstream binding, so this really is an overwrite.
    assert!(
        !commands(&b).contains(&"exec theirs"),
        "have: {:#?}",
        commands(&b)
    );

    // A keysym that does resolve behaves the other way round, in the same file.
    assert_eq!(by_command(&b, "exec resolves").bucket, Bucket::Keycode);
}
