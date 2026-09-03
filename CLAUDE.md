# swaykeys

A help sheet for every sway key binding that actually fires. It re-implements
sway's own config reading — includes, `set` expansion, mode blocks, the five
binding lists — because anything less invents bindings the compositor does not
have.

## Commands

The toolchain comes from the flake, not from PATH. Run things the way CI runs
them or you get findings CI does not have and miss the ones it does:

```bash
nix develop --command cargo fmt --all --check
nix develop --command cargo clippy --all-targets --locked -- -D warnings
nix develop --command cargo test --locked
nix develop --command cargo deny check advisories
nix develop --command cargo machete
nix develop --command groff -man -Tutf8 -ww -z man/man1/swaykeys.1
nix build
```

`--locked` everywhere: the lockfile is committed, and a build that silently
updates it is not the build CI ran.

`cargo run` needs the devshell too — `LD_LIBRARY_PATH` is set there, and
without it libxkbcommon is not found and half the tool degrades silently.

## Module map

| module      | holds                                                             |
|-------------|-------------------------------------------------------------------|
| `main.rs`   | clap args, the format/colour/pager decisions, stdout              |
| `source.rs` | locating the root config, the IPC cross-check, walking `include`  |
| `lex.rs`    | line mechanics: continuations, comments, lookahead for `{`        |
| `vars.rs`   | `set $var` and substitution                                       |
| `model.rs`  | directives → the five binding lists sway keeps                    |
| `xkb.rs`    | keysym ↔ keycode via dlopen'd libxkbcommon; cross-list shadowing  |
| `group.rs`  | bindings arranged into sections and rows for display              |
| `theme.rs`  | the one palette both renderers share                              |
| `render.rs` | plain / markdown / json output                                    |
| `tui.rs`    | the interactive pager                                             |
| `lib.rs`    | exists only so `tests/` can reach the parser without a compositor |

Flat `src/*.rs`, no `mod.rs`, no subdirectories.

## Non-negotiables

Each of these was learned the expensive way. They say how they break, not just
what the rule is.

- **Bindings are five lists, never a map.** sway keeps `Keycode`, `Keysym`,
  `Mouse`, `Switch` and `Gesture` lists per mode, and a binding only ever
  collides with or is removed by something in its *own* list. A
  `map[chord] = last_one_wins` model reports one of `bindsym --to-code $mod+d`
  and `bindsym $mod+d` as overwriting the other; in reality both live, and
  which fires is decided by query order in `get_active_binding`.
- **Variable substitution is one left-to-right pass**, and scanning resumes
  *after* the inserted value. Running it to a fixpoint expands `$foo` inside a
  value that sway leaves alone, so the sheet shows a command that never runs.
- **`$var` matching is longest-name-first prefix**, with no word boundary:
  `$mod1` must win over `$mod`. Sort the table the other way and every config
  with both silently mis-expands.
- **Only a whole line is a comment.** sway tests `line[0] == '#'` after
  stripping and never looks inside, so `bindsym $mod+x exec foo # note` keeps
  `# note` as part of the command. Stripping trailing `#` truncates real
  commands.
- **A continuation appends the next physical line verbatim**, leading
  whitespace included, and a line starting with `#` never continues.
- **xkb translation happens against group 0 only.** sway's translation state is
  fresh from `xkb_state_new` and nothing changes the group, so with
  `xkb_layout us,ru` only `us` is ever consulted. Consulting both invents
  translations.
- **libxkbcommon is `dlopen`'d, never linked.** Without it the tool must still
  print a sheet — raw keycodes, no shadow claims — and never fail. That is also
  why `flake.nix` wraps the binary with `LD_LIBRARY_PATH`: on NixOS there is no
  `/usr/lib` to fall back on, and without the wrapper `bindcode` degrades to
  raw keycodes with no error anywhere.
- **FFI uses `std::ffi::c_char`, never `i8`/`u8`.** It is signed on x86_64 and
  unsigned on aarch64. Hardcoding either side compiles on one architecture and
  fails on the other, which is why CI cross-checks aarch64.
- **Colour is the 16 ANSI slots only, never truecolor**, and every colour means
  exactly one thing. The terminal's own theme then picks the shades, so the
  sheet stays readable on light and dark backgrounds without us guessing. All
  of those decisions live in `theme.rs`: the static sheet and the pager cannot
  share a representation, but they must share the decisions — they did not
  once, and headings came out plain in one and yellow in the other, colliding
  with the yellow that already meant Ctrl.
- **A flag must reach a code path.** `-2` was accepted and ignored in the
  default mode for a while: on a terminal the pager opened and never looked at
  it, in a pipe the format resolved to markdown, which has no columns. That is
  what `tests/cli.rs` is for — every option there is asserted to change the
  output.
- **Payload goes through `stdout.lock().write_all`**, with `BrokenPipe` treated
  as a clean exit. The `print!` macros unwrap the write error, so
  `swaykeys | head` panics.
- **Anything that looked like a binding but did not parse goes to stderr.** A
  help sheet that silently drops a line is worse than no help sheet.
- **`shell_expand` works on bytes, not `char`s.** Scanning by byte is safe —
  `$`, `{`, `}` and name characters are ASCII, which cannot appear inside a
  multi-byte UTF-8 sequence — but `b[i] as char` maps each byte of such a
  sequence to its own Latin-1 codepoint, so `include ~/.config/sway/конфиг`
  came back as mojibake and matched nothing. Environment values are read with
  `var_os` for the same reason: `var` drops a non-UTF-8 value entirely.
- **`XDG_CONFIG_HOME` is checked for set-and-non-empty, deliberately not for
  absolute.** The XDG spec asks for absolute; sway tests only `NULL || '\0'`
  (`get_config_path`, `sway/config.c`), and matching sway wins every time these
  two disagree. Rejecting a relative value would send this tool looking
  somewhere the compositor is not.

## Conventions

The house Rust rules apply (`rmrf-code:rust`): `anyhow` is *not* a dependency
here — errors are `Result<T, String>` built with `format!`, which is a coherent
regime; do not half-migrate it. No panics outside tests, enforced by the
`[lints.clippy]` table in `Cargo.toml`. No async. Comments state the mechanism
of failure, not the action. English only in code and comments.
