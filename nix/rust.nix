# ==============================================================================
# SnifferEngine — Rust toolchain module (vault plan 0001 — Stage 1.3, ADR 001)
# ==============================================================================
# Selects the rust-overlay toolchain for the EXACT channel declared in
# rust-toolchain.toml — never "latest" (ADR 001 §3.3). The channel is parsed
# from the repository declaration, so the Nix toolchain cannot drift from it;
# nix/shell-hook.nix and the validation workflow assert runtime parity on top.
#
# Consumed by shell.nix as: import ./nix/rust.nix { inherit pkgs; }
# ==============================================================================

{ pkgs }:

let
  lib = pkgs.lib;

  # ── Toolchain channel: parsed, never duplicated ────────────────────────────
  # rust-toolchain.toml is the single source of truth for the toolchain
  # version. Keeping the expected line format stable here is part of the
  # contract: a reformatted file makes this module fail loudly instead of
  # silently using a different toolchain.
  tomlLines = lib.splitString "\n" (builtins.readFile ./../rust-toolchain.toml);

  channelLine =
    lib.findFirst (line: lib.hasPrefix "channel = " line) null tomlLines;

  channel =
    if channelLine == null then
      throw "nix/rust.nix: expected a 'channel = \"…\"' line in rust-toolchain.toml"
    else
      builtins.head (builtins.match "channel = \"([^\"]+)\".*" channelLine);

in
pkgs.rust-bin.stable.${channel}.default.override {
  # Components: rust-toolchain.toml declares clippy + rustfmt (both are
  # mandatory for scripts/check.sh); rust-src + rust-analyzer extend the
  # editor experience.
  extensions = [
    "rust-src" # rust-analyzer jump-to-definition
    "rust-analyzer" # LSP server
    "clippy" # linter — scripts/check.sh step 5
    "rustfmt" # formatter — scripts/check.sh step 2
  ];

  # Host target plus every Android target proven by the CI validate-android
  # job (.github/workflows/check.yml).
  targets = [
    "x86_64-unknown-linux-gnu"
    "aarch64-linux-android"
    "armv7-linux-androideabi"
    "i686-linux-android"
    "x86_64-linux-android"
  ];
}
