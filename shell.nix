# ==============================================================================
# shell.nix — SnifferEngine development environment (entry point)
# ==============================================================================
#
# Usage:
#   nix-shell                        # enter the dev shell
#   nix-shell --run "cargo build"    # run a single command inside
#   nix-shell --pure                 # hermetic: only the pinned closure
#   direnv allow                     # auto-activate (direnv + nix-direnv)
#
# Every external input is pinned in nix/pins.nix (ADR 001 §3.2): this entry
# point evaluates against the pinned nixpkgs + rust-overlay and imports
# nothing floating. The Rust toolchain is selected by the channel parsed
# from rust-toolchain.toml (nix/rust.nix), and the shell hook asserts the
# same channel at runtime.
# ==============================================================================

let
  pins = import ./nix/pins.nix;

  pkgs = pins.importNixpkgs {
    config = {
      allowUnfree = true; # Android SDK terms (stage 2 composition)
      android_sdk.accept_license = true;
    };
  };

  # Attrset: toolchain derivation + the channel parsed from
  # rust-toolchain.toml (consumed by the shell hook parity assertion).
  rust = import ./nix/rust.nix { inherit pkgs; };

  hostDeps = import ./nix/host-deps.nix { inherit pkgs; };

in
pkgs.mkShell {
  name = "snifferengine-dev";

  packages = [ rust.toolchain ] ++ hostDeps;

  # Banner, hard toolchain parity assertion (rust-toolchain.toml), host
  # environment exports, and quick command aliases (vault plan stage 1.5).
  shellHook = import ./nix/shell-hook.nix {
    inherit pkgs;
    expectedChannel = rust.channel;
    nixpkgsRev = pins.nixpkgs.rev;
  };
}
