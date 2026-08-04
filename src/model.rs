//! Turning config directives into the set of bindings sway would actually act
//! on.
//!
//! The trap this module exists to avoid is `map[chord] = last_one_wins`. sway
//! keeps bindings in *five separate lists* per mode, and a binding only ever
//! collides with, or is removed by, something in its own list
//! (`sway/commands/bind.c`, `binding_upsert` / `binding_remove`):
//!
//! | list       | holds                                                    |
//! |------------|----------------------------------------------------------|
//! | `Keycode`  | `bindcode`, and `bindsym --to-code` that resolved cleanly |
//! | `Keysym`   | plain `bindsym`, and `--to-code` that did not resolve     |
//! | `Mouse`    | anything whose first key is a button                      |
//! | `Switch`   | `bindswitch`                                             |
//! | `Gesture`  | `bindgesture`                                            |
//!
//! Concretely: our `bindsym --to-code $mod+d exec wofi` and the upstream
//! `bindsym $mod+d exec $menu` land in *different* lists. Neither overwrites
//! the other and sway logs nothing at INFO. Ours wins only because the keycode
//! list is queried first (`sway/input/keyboard.c`). Working out which one that
//! is needs xkb, so it happens in `xkb.rs`; here we only get the lists right.

use serde::Serialize;

use crate::source::Directive;

/// What the model needs from xkb, and the only thing it needs it for: sway
/// puts a `--to-code` binding in the keycode list *only* when its keysym maps
/// to exactly one keycode, and falls back to the keysym list otherwise
/// (`translate_binding`, `sway/commands/bind.c`).
///
/// Kept behind a trait so the parser is testable without libxkbcommon, and so
/// a host that cannot load it degrades to [`Optimistic`] instead of lying in
/// the other direction — claiming a `--to-code` binding collides with a plain
/// `bindsym` would merge two bindings sway keeps apart.
pub trait Resolver {
    /// Does this keysym name translate to exactly one keycode?
    fn resolves_to_single_keycode(&self, sym: &str) -> bool;

    /// *Which* keycode, when there is exactly one. Separate from the question
    /// above because [`Optimistic`] can answer one and not the other.
    fn keycode_for(&self, sym: &str) -> Option<u32>;

    /// Name of the keysym a raw keycode produces, for displaying `bindcode`.
    fn keycode_name(&self, code: u32) -> Option<String>;
}

/// Assumes every keysym translates cleanly, and cannot name keycodes. True for
/// essentially every chord on a normal layout; [`crate::xkb::Keymap`] replaces
/// it when libxkbcommon is available.
///
/// Erring optimistic is deliberate. Assuming translation *fails* would file
/// `--to-code` bindings next to plain `bindsym` ones, where they would
/// overwrite each other — merging two bindings sway keeps apart. Erring the
/// other way only costs an unresolved shadow claim, which we simply do not make.
pub struct Optimistic;

impl Resolver for Optimistic {
    fn resolves_to_single_keycode(&self, _sym: &str) -> bool {
        true
    }

    fn keycode_for(&self, _sym: &str) -> Option<u32> {
        None
    }

    fn keycode_name(&self, _code: u32) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    Keycode,
    Keysym,
    Mouse,
    Switch,
    Gesture,
}

/// One key in a chord. `Code` is a raw keycode from `bindcode`; `Sym` is a
/// keysym name as written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(untagged)]
pub enum Key {
    Code(u32),
    Sym(String),
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::Code(c) => write!(f, "{c}"),
            Key::Sym(s) => f.write_str(s),
        }
    }
}

/// Modifier mask. Names are aliases for bits, not distinct modifiers — sway
/// resolves `Ctrl` and `Control` to the same bit, so comparing masks (rather
/// than the strings people wrote) is the only correct way to match chords.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mods(pub u32);

impl Mods {
    // Values follow wlroots' WLR_MODIFIER_*; only their consistency matters.
    const SHIFT: u32 = 1;
    const CAPS: u32 = 2;
    const CTRL: u32 = 4;
    const ALT: u32 = 8;
    const MOD2: u32 = 16;
    const MOD3: u32 = 32;
    const LOGO: u32 = 64;
    const MOD5: u32 = 128;

    /// sway's modifier table (`sway/input/keyboard.c`), matched case-insensitively.
    fn from_name(name: &str) -> Option<u32> {
        Some(match name.to_ascii_lowercase().as_str() {
            "shift" => Self::SHIFT,
            "lock" => Self::CAPS,
            "control" | "ctrl" => Self::CTRL,
            "mod1" | "alt" => Self::ALT,
            "mod2" => Self::MOD2,
            "mod3" => Self::MOD3,
            "mod4" | "super" => Self::LOGO,
            "mod5" => Self::MOD5,
            _ => return None,
        })
    }

    /// Display names, in a fixed order so the same chord always reads the same.
    pub fn names(self) -> Vec<&'static str> {
        const ORDER: [(u32, &str); 8] = [
            (Mods::LOGO, "Super"),
            (Mods::CTRL, "Ctrl"),
            (Mods::ALT, "Alt"),
            (Mods::SHIFT, "Shift"),
            (Mods::CAPS, "Lock"),
            (Mods::MOD2, "Mod2"),
            (Mods::MOD3, "Mod3"),
            (Mods::MOD5, "Mod5"),
        ];
        ORDER
            .iter()
            .filter(|(b, _)| self.0 & b != 0)
            .map(|(_, n)| *n)
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Flags {
    pub release: bool,
    pub locked: bool,
    pub inhibited: bool,
    pub no_repeat: bool,
    pub to_code: bool,
    pub border: bool,
    pub contents: bool,
    pub titlebar: bool,
    /// `bindswitch --reload`: also run the binding when the config reloads.
    pub reload: bool,
    /// `bindgesture --exact`.
    pub exact: bool,
    /// `--input-device=`; `*` means any, which is the default.
    pub input: String,
    /// `GroupN` / `Mode_switch` in the chord, stored 0-based as sway does.
    pub group: Option<u8>,
}

impl Flags {
    /// The subset `binding_key_compare` treats as conflict-generating. Two
    /// bindings differing in any of these do not collide.
    fn conflict_bits(&self) -> u32 {
        (self.release as u32)
            | (self.locked as u32) << 1
            | (self.inhibited as u32) << 2
            | (self.border as u32) << 3
            | (self.contents as u32) << 4
            | (self.titlebar as u32) << 5
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Keys and mouse buttons.
    Chord,
    /// `bindswitch lid:on`.
    Switch,
    /// `bindgesture swipe:3:left`.
    Gesture,
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    pub mode: String,
    pub kind: Kind,
    pub bucket: Bucket,
    /// Display form: `Super+Shift+r`, `lid:on`, `swipe:3:left`.
    pub chord: String,
    /// Modifier names in display order; empty for switches and gestures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modifiers: Vec<String>,
    /// Keys as written, so a consumer sees `51` for `bindcode 51` even when
    /// `chord` renders it as `backslash`.
    pub keys: Vec<Key>,
    pub flags: Flags,
    pub command: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub comment: Vec<String>,
    pub origin: String,
    /// Index of the binding that wins this chord, when this one never fires.
    /// Filled in by `xkb.rs`; `None` means "not shadowed as far as we know".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadowed_by: Option<usize>,

    /// Modifier mask, for comparison rather than display.
    #[serde(skip)]
    pub mods: Mods,
    /// Keys in sway's canonical sorted order — the collision key, not the
    /// display form.
    #[serde(skip)]
    pub sorted_keys: Vec<Key>,
}

impl Binding {
    /// Exactly `binding_key_compare`: input device, list, conflict-generating
    /// flags, layout group, modifier mask, and the sorted key list.
    fn collision_key(&self) -> (&str, Bucket, u32, Option<u8>, u32, &[Key]) {
        (
            &self.flags.input,
            self.bucket,
            self.flags.conflict_bits(),
            self.flags.group,
            self.mods.0,
            &self.sorted_keys,
        )
    }
}

pub struct Bindings {
    pub list: Vec<Binding>,
    /// Lines that looked like bindings but could not be understood. A help
    /// sheet that quietly drops one is worse than no help sheet.
    pub unparsed: Vec<String>,
}

/// Build the binding set from config directives, applying collisions and
/// `unbind*` the way sway does.
pub fn build(directives: &[Directive], resolver: &dyn Resolver) -> Bindings {
    let mut list: Vec<Binding> = Vec::new();
    let mut unparsed = Vec::new();

    for d in directives {
        let Some(verb) = d.text.split_whitespace().next() else {
            continue;
        };
        if !verb.starts_with("bind") && !verb.starts_with("unbind") {
            continue;
        }
        // `bar { bindsym button1 … }` binds a bar button, not a key. Not ours,
        // and not an error either.
        if d.blocks
            .iter()
            .any(|b| b.split_whitespace().next() == Some("bar"))
        {
            continue;
        }

        match parse(d, resolver) {
            Some((binding, unbind)) => apply(&mut list, binding, unbind),
            None => unparsed.push(format!("UNPARSED {}: {}", d.origin(), d.text)),
        }
    }

    Bindings { list, unparsed }
}

/// Insert, replace or remove, honouring the per-list semantics.
fn apply(list: &mut Vec<Binding>, binding: Binding, unbind: bool) {
    let key = binding.collision_key();
    let existing = list.iter().position(|b| b.collision_key() == key);

    match (unbind, existing) {
        // `binding_remove`: only ever reaches its own list.
        (true, Some(i)) => {
            list.remove(i);
        }
        (true, None) => {}
        // `binding_upsert`: replace in place, so config order survives.
        (false, Some(i)) => list[i] = binding,
        (false, None) => list.push(binding),
    }
}

/// Parse one `bind*` / `unbind*` directive. `None` means "looked like a
/// binding but wasn't one we understand" — the caller reports it.
fn parse(d: &Directive, resolver: &dyn Resolver) -> Option<(Binding, bool)> {
    let mut tokens = d.text.split_whitespace();
    let verb = tokens.next()?;

    let (kind, unbind) = match verb {
        "bindsym" => (Verb::Sym, false),
        "bindcode" => (Verb::Code, false),
        "unbindsym" => (Verb::Sym, true),
        "unbindcode" => (Verb::Code, true),
        "bindswitch" => (Verb::Switch, false),
        "unbindswitch" => (Verb::Switch, true),
        "bindgesture" => (Verb::Gesture, false),
        "unbindgesture" => (Verb::Gesture, true),
        _ => return None,
    };

    let rest: Vec<&str> = tokens.collect();
    let (mut flags, rest) = parse_flags(&rest, &kind);
    let (combo, command) = rest.split_first()?;
    // `bind*` needs a command; `unbind*` must not have one.
    if unbind != command.is_empty() {
        return None;
    }
    let command = command.join(" ");

    let binding = match kind {
        // Switches and gestures have their own lists and no chord structure —
        // the token is the whole trigger.
        Verb::Switch | Verb::Gesture => {
            let (kind, bucket) = match kind {
                Verb::Switch => (Kind::Switch, Bucket::Switch),
                _ => (Kind::Gesture, Bucket::Gesture),
            };
            Binding {
                mode: mode_of(d),
                kind,
                bucket,
                chord: combo.to_string(),
                modifiers: Vec::new(),
                keys: vec![Key::Sym(combo.to_string())],
                sorted_keys: vec![Key::Sym(combo.to_string())],
                mods: Mods::default(),
                flags,
                command,
                comment: d.comment.clone(),
                origin: d.origin(),
                shadowed_by: None,
            }
        }
        Verb::Sym | Verb::Code => {
            let bindcode = matches!(kind, Verb::Code);
            let (mods, keys, group) = parse_chord(combo, bindcode)?;
            flags.group = group;
            let bucket = bucket_for(&keys, &flags, bindcode, resolver);
            flags = fixup_mouse_flags(flags, bucket);

            // `bindcode 51` means nothing to a human — render the keysym it
            // produces on the configured layout instead, where xkb can say.
            let shown: Vec<String> = keys
                .iter()
                .map(|k| match k {
                    Key::Code(c) => resolver.keycode_name(*c).unwrap_or_else(|| c.to_string()),
                    Key::Sym(s) => s.clone(),
                })
                .collect();
            let modifiers: Vec<String> = mods.names().iter().map(|s| s.to_string()).collect();
            let chord = modifiers
                .iter()
                .chain(shown.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("+");

            // sway compares the key list *after* translation, so for a keycode
            // binding the comparison is between keycodes, not the names people
            // typed. Without that, `--to-code a` and `--to-code A` look like
            // two bindings when sway sees one.
            let mut sorted_keys: Vec<Key> = keys
                .iter()
                .map(|k| match k {
                    Key::Sym(s) if bucket == Bucket::Keycode => {
                        resolver.keycode_for(s).map_or_else(|| k.clone(), Key::Code)
                    }
                    _ => k.clone(),
                })
                .collect();
            sorted_keys.sort();

            Binding {
                mode: mode_of(d),
                kind: Kind::Chord,
                bucket,
                chord,
                modifiers,
                keys,
                sorted_keys,
                mods,
                flags,
                command,
                comment: d.comment.clone(),
                origin: d.origin(),
                shadowed_by: None,
            }
        }
    };

    Some((binding, unbind))
}

enum Verb {
    Sym,
    Code,
    Switch,
    Gesture,
}

/// Consume leading `--flags`.
///
/// The three verbs take *different* flag sets, and getting this wrong is not a
/// cosmetic bug: sway stops at the first token it does not recognise and treats
/// it as the chord. Feed `bindswitch --reload lid:on …` through the `bindsym`
/// flag list and the trigger silently becomes `--reload`, so every `bindswitch`
/// in the file collapses into one entry.
fn parse_flags<'a>(tokens: &[&'a str], verb: &Verb) -> (Flags, Vec<&'a str>) {
    let mut flags = Flags {
        input: "*".into(),
        ..Default::default()
    };
    let mut exclude_titlebar = false;
    let mut i = 0;

    while i < tokens.len() {
        let t = tokens[i];
        let known = match verb {
            // sway/commands/bind.c, cmd_bindsym_or_bindcode
            Verb::Sym | Verb::Code => match t {
                "--release" => set(&mut flags.release),
                "--locked" => set(&mut flags.locked),
                "--inhibited" => set(&mut flags.inhibited),
                "--no-repeat" => set(&mut flags.no_repeat),
                "--no-warn" => true,
                // Ignored on bindcode: the keys are already codes.
                "--to-code" => {
                    flags.to_code = matches!(verb, Verb::Sym);
                    true
                }
                "--border" => set(&mut flags.border),
                "--whole-window" => {
                    flags.border = true;
                    flags.contents = true;
                    flags.titlebar = true;
                    true
                }
                "--exclude-titlebar" => set(&mut exclude_titlebar),
                _ => input_device(t, &mut flags),
            },
            // sway/commands/bind.c, cmd_bind_or_unbind_switch
            Verb::Switch => match t {
                "--locked" => set(&mut flags.locked),
                "--reload" => set(&mut flags.reload),
                "--no-warn" => true,
                _ => false,
            },
            // sway/commands/gesture.c
            Verb::Gesture => match t {
                "--exact" => set(&mut flags.exact),
                "--no-warn" => true,
                _ => input_device(t, &mut flags),
            },
        };
        if !known {
            break;
        }
        i += 1;
    }

    if exclude_titlebar {
        flags.titlebar = false;
    }
    (flags, tokens[i..].to_vec())
}

fn set(field: &mut bool) -> bool {
    *field = true;
    true
}

fn input_device(token: &str, flags: &mut Flags) -> bool {
    match token.strip_prefix("--input-device=") {
        Some(dev) => {
            flags.input = dev.trim_matches(['"', '\'']).to_string();
            true
        }
        None => false,
    }
}

/// A mouse binding gets `titlebar` implicitly unless `--exclude-titlebar` was
/// given — and `--exclude-titlebar` already cleared it in `parse_flags`, so
/// only add it back when the flag was absent and some region flag was set.
fn fixup_mouse_flags(mut flags: Flags, bucket: Bucket) -> Flags {
    if bucket == Bucket::Mouse && (flags.border || flags.contents) {
        flags.titlebar = true;
    }
    flags
}

/// Split a chord into modifiers, keys and an optional layout group.
#[allow(clippy::type_complexity)]
fn parse_chord(combo: &str, bindcode: bool) -> Option<(Mods, Vec<Key>, Option<u8>)> {
    let mut mods = Mods::default();
    let mut keys = Vec::new();
    let mut group = None;

    for part in combo.split('+') {
        if part.is_empty() {
            return None;
        }
        if let Some(bit) = Mods::from_name(part) {
            mods.0 |= bit;
            continue;
        }
        // `Mode_switch` is i3's alias for Group2.
        if part == "Mode_switch" {
            group = Some(1);
            continue;
        }
        if let Some(n) = part.strip_prefix("Group") {
            let n: u8 = n.parse().ok()?;
            if !(1..=4).contains(&n) {
                return None;
            }
            group = Some(n - 1);
            continue;
        }
        if bindcode {
            keys.push(Key::Code(part.parse().ok()?));
        } else {
            keys.push(Key::Sym(part.to_string()));
        }
    }

    if keys.is_empty() {
        return None;
    }
    Some((mods, keys, group))
}

/// Which of sway's lists this binding lands in.
fn bucket_for(keys: &[Key], flags: &Flags, bindcode: bool, resolver: &dyn Resolver) -> Bucket {
    // Region flags make it a mouse binding outright…
    if flags.border || flags.contents || flags.titlebar {
        return Bucket::Mouse;
    }
    // …and so does a first key that names a button.
    if keys.first().is_some_and(is_button) {
        return Bucket::Mouse;
    }
    if bindcode {
        return Bucket::Keycode;
    }
    // `--to-code` reaches the keycode list only if *every* keysym translates to
    // exactly one keycode; one failure sends the whole binding back
    // (`translate_binding` bails to `error:` on the first `count != 1`).
    let translated = flags.to_code
        && keys.iter().all(|k| match k {
            Key::Sym(s) => resolver.resolves_to_single_keycode(s),
            Key::Code(_) => true,
        });
    if translated {
        Bucket::Keycode
    } else {
        Bucket::Keysym
    }
}

/// `get_mouse_bindsym` / `get_mouse_bindcode`: X11 `buttonN` or a libinput
/// `BTN_*` name.
fn is_button(key: &Key) -> bool {
    match key {
        Key::Code(_) => false,
        Key::Sym(s) => {
            s.starts_with("BTN_")
                || s.strip_prefix("button")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        }
    }
}

/// The binding mode a directive sits in: the innermost enclosing `mode` block.
fn mode_of(d: &Directive) -> String {
    for block in d.blocks.iter().rev() {
        let mut tokens = block.split_whitespace();
        if tokens.next() != Some("mode") {
            continue;
        }
        // Skip `mode --pango_markup "resize"` style flags.
        let name = tokens.find(|t| !t.starts_with("--"));
        if let Some(name) = name {
            return name.trim_matches(['"', '\'']).to_string();
        }
    }
    "default".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Directive;
    use std::path::PathBuf;
    use std::rc::Rc;

    fn dir(text: &str) -> Directive {
        Directive {
            text: text.into(),
            blocks: Vec::new(),
            file: Rc::new(PathBuf::from("t")),
            line: 1,
            comment: Vec::new(),
        }
    }

    fn build_from(lines: &[&str]) -> Bindings {
        let dirs: Vec<Directive> = lines.iter().map(|l| dir(l)).collect();
        build(&dirs, &Optimistic)
    }

    #[test]
    fn modifier_aliases_collide() {
        // Ctrl and Control are the same bit, so the second overwrites.
        let b = build_from(&["bindsym Ctrl+a exec one", "bindsym Control+a exec two"]);
        assert_eq!(b.list.len(), 1);
        assert_eq!(b.list[0].command, "exec two");
    }

    #[test]
    fn to_code_and_plain_bindsym_do_not_collide() {
        // Different lists — this is the whole point.
        let b = build_from(&[
            "bindsym Mod4+d exec wmenu-run",
            "bindsym --to-code Mod4+d exec wofi",
        ]);
        assert_eq!(b.list.len(), 2);
    }

    #[test]
    fn release_flag_splits_the_collision() {
        let b = build_from(&[
            "bindsym Mod4+a exec one",
            "bindsym --release Mod4+a exec two",
        ]);
        assert_eq!(b.list.len(), 2);
    }

    #[test]
    fn unbind_only_hits_its_own_list() {
        let b = build_from(&[
            "bindsym Mod4+space exec plain",
            "bindcode 65 exec by-code",
            "unbindsym Mod4+space",
        ]);
        assert_eq!(b.list.len(), 1);
        assert_eq!(b.list[0].command, "exec by-code");
    }

    #[test]
    fn upsert_keeps_position() {
        let b = build_from(&[
            "bindsym Mod4+a exec first",
            "bindsym Mod4+b exec second",
            "bindsym Mod4+a exec replaced",
        ]);
        assert_eq!(b.list.len(), 2);
        assert_eq!(b.list[0].command, "exec replaced");
        assert_eq!(b.list[1].command, "exec second");
    }

    #[test]
    fn chord_order_does_not_matter() {
        let b = build_from(&[
            "bindsym Shift+Mod4+a exec one",
            "bindsym Mod4+Shift+a exec two",
        ]);
        assert_eq!(b.list.len(), 1);
    }

    #[test]
    fn buttons_land_in_the_mouse_list() {
        let b = build_from(&["bindsym button2 kill", "bindsym BTN_LEFT nop"]);
        assert!(b.list.iter().all(|x| x.bucket == Bucket::Mouse));
    }

    #[test]
    fn switch_and_gesture_are_separate() {
        let b = build_from(&[
            "bindswitch --reload --locked lid:on output eDP-1 disable",
            "bindgesture swipe:3:left workspace next",
        ]);
        assert_eq!(b.list.len(), 2);
        assert_eq!(b.list[0].bucket, Bucket::Switch);
        assert_eq!(b.list[1].bucket, Bucket::Gesture);
    }

    #[test]
    fn bar_block_bindings_are_not_ours() {
        let mut d = dir("bindsym button2 kill");
        d.blocks = vec!["bar".into()];
        assert!(build(&[d], &Optimistic).list.is_empty());
    }

    #[test]
    fn mode_comes_from_the_innermost_mode_block() {
        let mut d = dir("bindsym h resize shrink width 10px");
        d.blocks = vec!["mode \"resize\"".into()];
        assert_eq!(build(&[d], &Optimistic).list[0].mode, "resize");
    }

    #[test]
    fn broken_binding_is_reported_not_dropped() {
        let b = build_from(&["bindsym", "bindsym Mod4+ exec x", "unbindsym Mod4+a exec x"]);
        assert!(b.list.is_empty());
        assert_eq!(b.unparsed.len(), 3);
    }

    #[test]
    fn group_is_part_of_the_key() {
        let b = build_from(&["bindsym Group2+a exec one", "bindsym a exec two"]);
        assert_eq!(b.list.len(), 2);
        assert_eq!(b.list[0].flags.group, Some(1));
    }
}
