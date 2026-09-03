//! Cross-list shadowing, which needs a real keymap.
//!
//! These skip rather than fail when libxkbcommon is unavailable: `cargo test`
//! on a bare machine should not go red for a reason that is not the code's
//! fault. CI installs the library so they do run there.
//!
//! The helpers return `Result` and the tests unwrap with `?`. That is not
//! style: a plain helper in an integration file is not a `#[test]` function, so
//! clippy's test-harness exemption does not reach it and an `.expect()` or a
//! `panic!` here would fail the build. Note that a missing keymap and a broken
//! fixture stay distinguishable — `Ok(None)` is a skip, `Err` is a failure.

use std::path::PathBuf;

use swaykeys::model::{self, Binding, Bindings, Bucket};
use swaykeys::{source, xkb};

/// `Ok(None)` when there is no usable libxkbcommon on this host — as opposed to
/// `Err`, which means the fixture itself did not load and the run should fail.
fn load(name: &str) -> Result<Option<Bindings>, String> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    let config = source::load(Some(&path))?;
    let settings = xkb::Settings::from_directives(&config.directives);
    let Some(keymap) = xkb::Keymap::new(&settings) else {
        return Ok(None);
    };

    let mut bindings = model::build(&config.directives, &keymap);
    xkb::mark_shadowed(&mut bindings.list, &keymap);
    Ok(Some(bindings))
}

/// CI greps for this exact message to prove the library really did load; see
/// the "Check the keymap tests actually ran" step.
macro_rules! keymap_or_skip {
    ($name:expr) => {
        match load($name)? {
            Some(b) => b,
            None => {
                eprintln!("skipping: libxkbcommon unavailable");
                return Ok(());
            }
        }
    };
}

fn by_command<'a>(b: &'a Bindings, command: &str) -> Result<&'a Binding, String> {
    b.list
        .iter()
        .find(|x| x.command == command)
        .ok_or_else(|| format!("no binding runs {command:?}; have {:#?}", commands(b)))
}

fn commands(b: &Bindings) -> Vec<&str> {
    b.list.iter().map(|x| x.command.as_str()).collect()
}

#[test]
fn to_code_shadows_the_plain_bindsym_it_never_replaced() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    let loser = by_command(&b, "exec default-menu")?;
    let winner = by_command(&b, "exec our-menu")?;

    // Both survive parsing — they were never in the same list…
    assert_eq!(loser.bucket, Bucket::Keysym);
    assert_eq!(winner.bucket, Bucket::Keycode);
    // …and the keycode list is queried first, so only one of them ever fires.
    assert_eq!(
        loser.shadowed_by.map(|i| b.list[i].command.as_str()),
        Some("exec our-menu")
    );
    assert_eq!(winner.shadowed_by, None);
    Ok(())
}

#[test]
fn release_bindings_are_a_separate_lookup() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec on-release")?.shadowed_by, None);
    Ok(())
}

#[test]
fn device_specific_bindings_are_never_ranked() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    // Which of these wins depends on the device the event came from. Claiming
    // either would be a guess dressed as a fact.
    assert_eq!(by_command(&b, "exec device-specific")?.shadowed_by, None);
    assert_eq!(by_command(&b, "exec generic")?.shadowed_by, None);
    Ok(())
}

#[test]
fn a_keycode_and_its_keysym_name_are_one_binding() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    // `bindcode 51` and `--to-code backslash` compare equal after translation,
    // so this is an overwrite, not two entries.
    assert!(
        !commands(&b).contains(&"exec by-raw-code"),
        "have: {:#?}",
        commands(&b)
    );
    by_command(&b, "exec by-name")?;
    Ok(())
}

#[test]
fn bindcode_is_shown_by_name() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec by-name")?.chord, "Super+backslash");
    Ok(())
}

#[test]
fn an_uncontested_binding_is_not_marked() -> Result<(), String> {
    let b = keymap_or_skip!("shadow.config");
    assert_eq!(by_command(&b, "exec lonely")?.shadowed_by, None);
    Ok(())
}

/// The case neither the ticket nor the prototype anticipated: `--to-code` can
/// *fail*, and then the binding lands in the keysym list after all — where it
/// does collide with a plain `bindsym`.
#[test]
fn to_code_that_cannot_translate_stays_a_keysym_binding() -> Result<(), String> {
    let b = keymap_or_skip!("untranslatable.config");
    // Print sits on two keycodes in the stock us layout (PRSC and I218), so
    // sway's single-match test fails and the binding never reaches the keycode
    // list. Verified independently with `xkbcli compile-keymap --layout us`.
    let print = by_command(&b, "exec ours")?;
    assert_eq!(print.bucket, Bucket::Keysym);
    // Same list as the upstream binding, so this really is an overwrite.
    assert!(
        !commands(&b).contains(&"exec theirs"),
        "have: {:#?}",
        commands(&b)
    );

    // A keysym that does resolve behaves the other way round, in the same file.
    assert_eq!(by_command(&b, "exec resolves")?.bucket, Bucket::Keycode);
    Ok(())
}
