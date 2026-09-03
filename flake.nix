{
  description = "swaykeys — help sheet for every active sway key binding";

  # Indirect ref: on a machine whose flake registry already has nixpkgs
  # realised (e.g. the author's), this reuses that store path. Consumers get
  # whatever the lock pins — override with inputs.swaykeys.inputs.nixpkgs.follows.
  inputs.nixpkgs.url = "flake:nixpkgs";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems
        (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forAll (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "swaykeys";
          # Read straight from Cargo.toml so the two never drift apart.
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
          src = self;
          # Cargo.lock is committed, so deps resolve straight from it — no
          # cargoHash to recompute on every dependency bump.
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.makeWrapper ];

          # libxkbcommon is dlopen'd, not linked, so nothing records a store
          # path for it and there is no global /usr/lib to fall back on here.
          # Without the wrapper `bindcode` silently degrades to raw keycodes on
          # NixOS. XKB_CONFIG_ROOT is baked into nixpkgs' libxkbcommon, so the
          # library path is the only thing missing.
          postInstall = ''
            install -Dm644 man/man1/swaykeys.1 $out/share/man/man1/swaykeys.1
            wrapProgram $out/bin/swaykeys \
              --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath [ pkgs.libxkbcommon ]}
          '';
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer

            # The tools CI drives through this shell. Left out, every check
            # below `cargo test` fails with `command not found` instead of
            # with a finding.
            groff # man page syntax check
            cargo-deny # advisory scan
            cargo-machete # unused dependency scan
          ];
          # Same reason as the wrapper above: `cargo run` has to find it too.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.libxkbcommon ];
        };
      });
    };
}
