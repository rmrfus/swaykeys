# swaykeys

[![CI](https://github.com/rmrfus/swaykeys/actions/workflows/ci.yml/badge.svg)](https://github.com/rmrfus/swaykeys/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/rmrfus/swaykeys?logo=github)](https://github.com/rmrfus/swaykeys/releases/latest)
[![License](https://img.shields.io/github/license/rmrfus/swaykeys)](LICENSE)

A help sheet for every active [sway](https://swaywm.org) key binding. Reads the
config the way sway reads it — following `include`, expanding `set` variables,
tracking `mode` blocks — and shows the bindings that actually fire, grouped
into sections.

On a terminal it opens an interactive sheet; type to filter and the section
headings stay put:

```
Standard
> Super+Return           exec foot
  Super+1                workspace number 1
  Super+2                workspace number 2
resize
  h                      resize shrink width 10px
  l                      resize grow width 10px
Touchpad
  swipe:3:left           workspace next
Switches
  lid:on                 output eDP-1 disable

exec foot
/etc/sway/config:68
> workspace█   25/105
```

Piped, it prints markdown — so `swaykeys > BINDINGS.md` is a cheat sheet you
can commit.

## Why not grep the config

Because the config does not say what fires. Three things get in the way, and
all of them are load-bearing on a real setup:

- **`include` is not expanded by the IPC.** `swaymsg -t get_config` returns the
  root config file byte for byte — no include expansion, no `set` stripping. If
  your config starts with `include /etc/sway/config`, as the recommended one
  does, then every binding in it — all 75 of the upstream defaults, the ones
  you are most likely to have forgotten — is missing from that output. You can
  check for yourself:

  ```sh
  diff ~/.config/sway/config <(swaymsg -t get_config -r | jq -r .config)
  ```

- **A chord can have two live bindings.** sway keeps bindings in five separate
  lists per mode. `bindsym --to-code $mod+d` and a plain `bindsym $mod+d` never
  collide — they are in different lists — and the first fires only because the
  keycode list is queried first. `dict[chord] = last_one_wins` merges two
  bindings sway keeps apart; listing both leaves you to guess. swaykeys works
  out which one wins, shows it, and hides the other unless you ask for `--all`.

- **`--to-code` is a request, not a guarantee.** sway only files the binding
  under keycodes if every keysym in the chord maps to exactly one keycode.
  `Print` sits on two keys in the stock `us` layout, so `bindsym --to-code
  Print` quietly stays a keysym binding — and *does* then overwrite the plain
  `bindsym Print` from the defaults.

Anything that looks like a binding but does not parse is reported on stderr
with its file and line, rather than dropped.

## Install

### As a Nix package (flake)

```sh
nix run   github:rmrfus/swaykeys    # run without installing
nix build github:rmrfus/swaykeys    # ./result/bin/swaykeys
nix profile install github:rmrfus/swaykeys
```

Pull it into a NixOS / home-manager flake as an input:

```nix
inputs.swaykeys.url = "github:rmrfus/swaykeys";
inputs.swaykeys.inputs.nixpkgs.follows = "nixpkgs";   # reuse your nixpkgs
# then, where inputs + pkgs are in scope:
home.packages = [ inputs.swaykeys.packages.${pkgs.system}.default ];
```

### With Cargo

```sh
cargo install --git https://github.com/rmrfus/swaykeys --locked
```

### Prebuilt binaries

Glibc binaries for x86_64 and aarch64 hang off each
[release](https://github.com/rmrfus/swaykeys/releases):

```sh
curl -fsSL https://github.com/rmrfus/swaykeys/releases/latest/download/swaykeys-x86_64-linux.tar.gz | tar xz
./swaykeys --version
```

Checksums are in `SHA256SUMS` on the same release. They are glibc rather than
static musl builds because libxkbcommon is loaded with `dlopen`, which a fully
static binary cannot do.

### From a local checkout

```sh
direnv allow            # cargo/rustc from the flake devShell (or: nix develop)
cargo build --release   # ./target/release/swaykeys
```

## Usage

```
swaykeys [OPTIONS]
```

| Flag                  | Default | Meaning                                                        |
|-----------------------|---------|----------------------------------------------------------------|
| `-c, --config <PATH>` | auto    | read this config instead of locating the running one           |
| `--format <FMT>`      | `auto`  | `auto`, `plain`, `md` or `json`; naming one turns the pager off |
| `--pager <WHEN>`      | `auto`  | `auto`, `always`, `never`                                      |
| `--color <WHEN>`      | `auto`  | `auto`, `always`, `never`; `NO_COLOR` always wins              |
| `-2, --two-column`    | off     | lay the sheet out side by side                                 |
| `--desc`              | off     | show the comment above a binding instead of its command        |
| `--all`               | off     | also show bindings that never fire, and what beats them        |
| `--mode <NAME>`       | all     | only this binding mode                                         |

Keys in the interactive sheet follow fzf: type to filter, `↑`/`↓` or
`Ctrl-N`/`Ctrl-P` to move, `Ctrl-U` to clear, `Esc` to leave. `q` is not quit —
it goes into the filter.

## Sections

`Standard`, then one per binding mode (named and ordered as in your config),
then `Mouse`, `Touchpad`, `Switches`, `Built-in`.

Two entries never appear as a `bind*` line but do respond to input, and are
listed anyway:

- `floating_modifier`, expanded into the drag and the resize it binds.
- Ctrl+Alt+F1..F12. sway handles these itself, but *after* config bindings and
  only if none matched — so it is a fallback, and a binding of your own on the
  same chord wins.

## Bind it to a key

```
# ~/.config/sway/config
bindsym $mod+F1 exec foot --app-id=floating swaykeys
for_window [app_id="floating"] floating enable, resize set width 60 ppt height 70 ppt
```

## Machine-readable

`--format json` emits an object with `root`, `bindings` and `unparsed`. Each
binding carries its mode, list, chord, modifiers, keys, flags, command, the
comment above it, `file:line`, and — where one exists — the index of the
binding that shadows it.

```sh
swaykeys --format json | jq '.bindings[] | select(.shadowed_by) | .chord'
```

## License

MIT — see [LICENSE](LICENSE).
