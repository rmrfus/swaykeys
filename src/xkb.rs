//! The keysym ↔ keycode half, and the shadow detection it makes possible.
//!
//! Two questions need a real keymap, and neither can be guessed:
//!
//! 1. **Which list does `bindsym --to-code` land in?** sway translates each
//!    keysym to a keycode and keeps the binding in the keycode list only if
//!    *every* one resolves to exactly one keycode; a single failure sends the
//!    whole binding back to the keysym list (`translate_binding`).
//! 2. **What does `bindcode 51` mean?** Nothing, to a human, until the keymap
//!    says it is `backslash`.
//!
//! ## Which layout
//!
//! sway starts with an empty `xkb_rule_names` and rebuilds the translation
//! state from **the first input config that sets `xkb_layout` or `xkb_file`**,
//! then re-translates every binding (`retranslate_keysyms`,
//! `sway/input/input-manager.c`; `translate_keysyms`, `sway/config.c`).
//! Identifier configs (`input *`, `input "1:1:x"`) are checked before type
//! configs (`input type:keyboard`).
//!
//! With `xkb_layout us,ru` the translation state is fresh from
//! `xkb_state_new`, so group 0 is active and nothing ever changes it:
//! translation happens against the **first** layout only. That is the answer to
//! the open question in the ticket — no guessing, and no divergence to flag.
//!
//! libxkbcommon is opened with `dlopen`, so a host without it still gets a
//! working help sheet: raw keycodes instead of names, and no shadow claims.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

use xkbcommon_dl as xkb;

use crate::model::{Binding, Bucket, Key, Resolver};
use crate::source::Directive;

/// `xkb_*` settings lifted out of the config's `input` blocks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Settings {
    pub rules: Option<String>,
    pub model: Option<String>,
    pub layout: Option<String>,
    pub variant: Option<String>,
    pub options: Option<String>,
}

impl Settings {
    /// Pick the input config sway would translate against: the first one
    /// setting `xkb_layout`, identifier blocks before `type:` blocks.
    pub fn from_directives(directives: &[Directive]) -> Settings {
        let mut by_identifier: Option<Settings> = None;
        let mut by_type: Option<Settings> = None;

        for header in input_blocks(directives) {
            let settings = collect(directives, &header);
            if settings.layout.is_none() {
                continue;
            }
            let is_type = header
                .split_whitespace()
                .nth(1)
                .is_some_and(|id| id.trim_matches('"').starts_with("type:"));
            let slot = if is_type {
                &mut by_type
            } else {
                &mut by_identifier
            };
            if slot.is_none() {
                *slot = Some(settings);
            }
        }
        by_identifier.or(by_type).unwrap_or_default()
    }
}

/// Distinct `input …` block headers, in config order.
fn input_blocks(directives: &[Directive]) -> Vec<String> {
    let mut seen = Vec::new();
    for d in directives {
        for block in &d.blocks {
            if block.split_whitespace().next() == Some("input") && !seen.contains(block) {
                seen.push(block.clone());
            }
        }
    }
    seen
}

fn collect(directives: &[Directive], header: &str) -> Settings {
    let mut s = Settings::default();
    for d in directives
        .iter()
        .filter(|d| d.blocks.last().is_some_and(|b| b == header))
    {
        let Some((key, value)) = d.text.split_once(char::is_whitespace) else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_string();
        match key {
            "xkb_rules" => s.rules = Some(value),
            "xkb_model" => s.model = Some(value),
            "xkb_layout" => s.layout = Some(value),
            "xkb_variant" => s.variant = Some(value),
            "xkb_options" => s.options = Some(value),
            _ => {}
        }
    }
    s
}

/// A compiled keymap plus the two lookup tables we need from it.
pub struct Keymap {
    /// keycode → the single keysym it produces in group 0.
    name_of: HashMap<u32, String>,
    /// keysym → how many keycodes produce it, and the last one seen. sway's
    /// `--to-code` succeeds only at a count of exactly one.
    codes_for: HashMap<u32, (u32, usize)>,
}

impl Keymap {
    /// Compile the keymap. `None` when libxkbcommon is absent or the rules do
    /// not compile — the caller falls back to [`crate::model::Optimistic`].
    pub fn new(settings: &Settings) -> Option<Keymap> {
        let lib = xkb::xkbcommon_option()?;

        // Held until after xkb_keymap_new_from_names has copied them out.
        let cstr = |v: &Option<String>| v.as_deref().map(|s| CString::new(s).unwrap_or_default());
        let (rules, model, layout, variant, options) = (
            cstr(&settings.rules),
            cstr(&settings.model),
            cstr(&settings.layout),
            cstr(&settings.variant),
            cstr(&settings.options),
        );
        let ptr = |c: &Option<CString>| c.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
        let names = xkb::xkb_rule_names {
            rules: ptr(&rules),
            model: ptr(&model),
            layout: ptr(&layout),
            variant: ptr(&variant),
            options: ptr(&options),
        };

        // SAFETY: every pointer below is either freshly returned by
        // libxkbcommon and null-checked, or a CString alive for this scope.
        unsafe {
            let context = (lib.xkb_context_new)(xkb::xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if context.is_null() {
                return None;
            }
            let keymap = (lib.xkb_keymap_new_from_names)(
                context,
                &names,
                xkb::xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            );
            if keymap.is_null() {
                (lib.xkb_context_unref)(context);
                return None;
            }
            let state = (lib.xkb_state_new)(keymap);
            if state.is_null() {
                (lib.xkb_keymap_unref)(keymap);
                (lib.xkb_context_unref)(context);
                return None;
            }

            let mut name_of = HashMap::new();
            let mut codes_for: HashMap<u32, (u32, usize)> = HashMap::new();
            let min = (lib.xkb_keymap_min_keycode)(keymap);
            let max = (lib.xkb_keymap_max_keycode)(keymap);
            let mut buf = [0i8; 64];

            for code in min..=max {
                // Group 0: the state is fresh, so this is the first layout —
                // exactly what sway translates against.
                let sym = (lib.xkb_state_key_get_one_sym)(state, code);
                if sym == 0 {
                    continue; // XKB_KEY_NoSymbol — `find_keycode` skips these
                }
                let entry = codes_for.entry(sym).or_insert((code, 0));
                entry.0 = code;
                entry.1 += 1;

                let n = (lib.xkb_keysym_get_name)(sym, buf.as_mut_ptr(), buf.len());
                if n > 0 {
                    if let Ok(name) = CStr::from_ptr(buf.as_ptr()).to_str() {
                        name_of.insert(code, name.to_string());
                    }
                }
            }

            (lib.xkb_state_unref)(state);
            (lib.xkb_keymap_unref)(keymap);
            (lib.xkb_context_unref)(context);

            Some(Keymap { name_of, codes_for })
        }
    }

    /// Parse a keysym name the way sway does — case-insensitively.
    fn keysym(&self, name: &str) -> Option<u32> {
        let lib = xkb::xkbcommon_option()?;
        let c = CString::new(name).ok()?;
        // SAFETY: `c` outlives the call; the function only reads the string.
        let sym = unsafe {
            (lib.xkb_keysym_from_name)(
                c.as_ptr(),
                xkb::xkb_keysym_flags::XKB_KEYSYM_CASE_INSENSITIVE,
            )
        };
        (sym != 0).then_some(sym)
    }
}

impl Resolver for Keymap {
    fn resolves_to_single_keycode(&self, sym: &str) -> bool {
        self.keycode_for(sym).is_some()
    }

    fn keycode_for(&self, sym: &str) -> Option<u32> {
        let sym = self.keysym(sym)?;
        match self.codes_for.get(&sym) {
            Some(&(code, 1)) => Some(code),
            // Zero or several matches: `translate_binding` bails out.
            _ => None,
        }
    }

    fn keycode_name(&self, code: u32) -> Option<String> {
        self.name_of.get(&code).cloned()
    }
}

/// Mark bindings that never fire because another list is queried first.
///
/// `get_active_binding` (`sway/input/keyboard.c`) walks the keycode list, then
/// the keysym list twice, keeping the *first* match whenever specificity ties.
/// So an equally-applicable keycode binding always beats a keysym one.
///
/// "Equally applicable" is the load-bearing part: sway prefers a binding whose
/// input device, layout group, lock state or inhibit state matches the actual
/// event, and we cannot know those without the runtime device list. When the
/// two candidates differ in any of them, no claim is made — an unmarked
/// binding means "not known to be shadowed", never "definitely fires".
pub fn mark_shadowed(bindings: &mut [Binding], resolver: &dyn Resolver) {
    let codes: Vec<Option<Vec<u32>>> = bindings.iter().map(|b| keycodes_of(b, resolver)).collect();

    for i in 0..bindings.len() {
        if bindings[i].bucket != Bucket::Keysym {
            continue;
        }
        let Some(loser) = &codes[i] else { continue };

        let winner = (0..bindings.len()).find(|&j| {
            bindings[j].bucket == Bucket::Keycode
                && codes[j].as_ref() == Some(loser)
                && equally_applicable(&bindings[i], &bindings[j])
        });
        bindings[i].shadowed_by = winner;
    }
}

/// The keycodes a binding's chord resolves to, or `None` when any key cannot
/// be pinned down — in which case we say nothing rather than guess.
fn keycodes_of(binding: &Binding, resolver: &dyn Resolver) -> Option<Vec<u32>> {
    if binding.kind != crate::model::Kind::Chord {
        return None;
    }
    let mut out: Vec<u32> = binding
        .keys
        .iter()
        .map(|k| match k {
            Key::Code(c) => Some(*c),
            Key::Sym(s) => resolver.keycode_for(s),
        })
        .collect::<Option<Vec<u32>>>()?;
    out.sort_unstable();
    Some(out)
}

/// Would both bindings be candidates for the very same key event?
fn equally_applicable(a: &Binding, b: &Binding) -> bool {
    a.mode == b.mode
        && a.mods == b.mods
        // A device-specific binding outranks `*`, but only for events from that
        // device — which we cannot see. Refuse to rank them.
        && a.flags.input == "*"
        && b.flags.input == "*"
        && a.flags.group == b.flags.group
        // press and release bindings are looked up in separate passes
        && a.flags.release == b.flags.release
        && a.flags.locked == b.flags.locked
        && a.flags.inhibited == b.flags.inhibited
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::rc::Rc;

    fn dir(text: &str, blocks: &[&str]) -> Directive {
        Directive {
            text: text.into(),
            blocks: blocks.iter().map(|s| s.to_string()).collect(),
            file: Rc::new(PathBuf::from("t")),
            line: 1,
            comment: Vec::new(),
        }
    }

    #[test]
    fn takes_the_first_input_block_that_sets_a_layout() {
        let dirs = [
            // No layout here, so it is not the one sway would use.
            dir("accel_profile adaptive", &["input \"type:touchpad\""]),
            dir("xkb_layout us,ru", &["input *"]),
            dir("xkb_variant altgr-intl,ruu", &["input *"]),
            dir("xkb_layout de", &["input \"1:1:other\""]),
        ];
        let s = Settings::from_directives(&dirs);
        assert_eq!(s.layout.as_deref(), Some("us,ru"));
        assert_eq!(s.variant.as_deref(), Some("altgr-intl,ruu"));
    }

    #[test]
    fn identifier_blocks_outrank_type_blocks() {
        let dirs = [
            dir("xkb_layout de", &["input \"type:keyboard\""]),
            dir("xkb_layout us", &["input *"]),
        ];
        assert_eq!(
            Settings::from_directives(&dirs).layout.as_deref(),
            Some("us")
        );
    }

    #[test]
    fn falls_back_to_a_type_block_when_that_is_all_there_is() {
        let dirs = [dir("xkb_layout de", &["input \"type:keyboard\""])];
        assert_eq!(
            Settings::from_directives(&dirs).layout.as_deref(),
            Some("de")
        );
    }
}
