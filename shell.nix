# ==============================================================================
# shell.nix — SnifferEngine development environment (entry point)
# ==============================================================================
# Usage:
#   nix-shell                        # enter the dev shell
#   nix-shell --run "cargo check"    # run a single command inside the shell
#
# Every external input is pinned through nix/pins.nix (ADR 001): this file
# imports no system channel. The Rust toolchain is selected by the channel
# declared in rust-toolchain.toml (nix/rust.nix parses it — single source of
# truth), and the host dependency closure mirrors the consolidated CI's Linux
# apt list plus the tools scripts/check.sh needs (nix/host-deps.nix).
#
# Module layout:
#   nix/pins.nix          pinned nixpkgs + rust-overlay (single pin source)
#   nix/rust.nix          toolchain for the rust-toolchain.toml channel
#   nix/host-deps.nix     host graphics/system libs, build + check tooling
#   nix/docs/adr_001…     the recorded style decision
#
# Stage 2 (vault plan 0001) extends this shell with the pinned Android SDK
# and the shell hook that asserts toolchain parity and exports the
# bindgen/NDK environment.
# ==============================================================================

let
  pins = import ./nix/pins.nix;

  # Central repository configuration: unfree is required by parts of the
  # Android toolchain; the SDK license acceptance is declared here once so
  # nix/android.nix (stage 2) needs no config of its own.
  pkgs = pins.importNixpkgs {
    config = {
      allowUnfree = true;
      android_sdk.accept_license = true;
    };
  };

  rustToolchain = import ./nix/rust.nix { inherit pkgs; };
  hostDeps = import ./nix/host-deps.nix { inherit pkgs; };

in
pkgs.mkShell {
  name = "snifferengine-dev";

  # Pinned Rust toolchain (rust-toolchain.toml channel, host + Android
  # targets) plus the full host dependency closure.
  packages = [ rustToolchain ] ++ hostDeps;
}
