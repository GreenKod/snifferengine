#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine — development container entrypoint (vault plan 0002 — Stage 1.5)
# ==============================================================================
# Contract (ADR 001 §3.4, plan rule 4):
#
#   1. Hard toolchain parity assertion — three-way agreement is enforced:
#      the repository's rust-toolchain.toml, the toolchain record baked into
#      the image at build time, and the active rustc. Any mismatch is a hard
#      failure with rebuild guidance. RUSTUP_AUTO_INSTALL=0 in the image
#      forbids rustup from silently downloading a toolchain the image does
#      not ship: the only way to change the toolchain is to change
#      rust-toolchain.toml and rebuild the image (docker compose build).
#
#   2. Environment — Debian's standard paths keep the exports thin: the Nix
#      track's shell hook needs PKG_CONFIG_PATH / OPENSSL_* because Nix store
#      paths are nonstandard; inside Debian they would be no-ops. What the
#      container does need is LIBCLANG_PATH, resolved explicitly so bindgen
#      never guesses. Android target env (BINDGEN_EXTRA_CLANG_ARGS_*,
#      ANDROID_*) lands in stage 2.2 together with the SDK.
#
# Testability: WORKSPACE, IMAGE_TOOLCHAIN_RECORD, and CLANG_SEARCH_ROOT are
# environment-overridable so the assertion logic can be exercised on any
# machine with rustup — the container only ever uses the defaults.
# ==============================================================================
set -euo pipefail

# Regex ranges under the tr_TR collation silently exclude letters such as
# "i" (the lesson recorded in scripts/update-nix-pins.sh); force byte
# semantics for the whole script.
export LC_ALL=C

WORKSPACE="${WORKSPACE:-/workspace}"
IMAGE_TOOLCHAIN_RECORD="${IMAGE_TOOLCHAIN_RECORD:-/usr/local/rustup/image-toolchain-version}"
CLANG_SEARCH_ROOT="${CLANG_SEARCH_ROOT:-/usr/lib}"

fail() { printf "entrypoint: \033[31m✗ %s\033[0m\n" "$*" >&2; exit 1; }
info() { printf "entrypoint: \033[32m✓ %s\033[0m\n" "$*" >&2; }

# ── 1. Toolchain parity (three-way) ──────────────────────────────────────────
toml="$WORKSPACE/rust-toolchain.toml"
[ -f "$toml" ] || fail "rust-toolchain.toml not found at $toml — is the repository mounted at $WORKSPACE?"

declared="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$toml" | head -n1)"
[ -n "$declared" ] || fail "no 'channel = \"…\"' line found in $toml — the expected format is: channel = \"1.98.0\""

[ -f "$IMAGE_TOOLCHAIN_RECORD" ] || fail "image toolchain record missing ($IMAGE_TOOLCHAIN_RECORD) — rebuild the image (docker compose build)"
recorded="$(tr -d '[:space:]' < "$IMAGE_TOOLCHAIN_RECORD")"

active="$(rustc --version 2>/dev/null | awk '{print $2}' || true)"
[ -n "$active" ] || fail "rust-toolchain.toml declares $declared but that toolchain is not installed in this image — rebuild the image (docker compose build)"
[ "$active" = "$recorded" ] || fail "active rustc $active does not match the image toolchain $recorded — the toolchain layer is corrupt; rebuild the image (docker compose build)"
[ "$declared" = "$recorded" ] || fail "rust-toolchain.toml declares $declared but this image ships $recorded — the repository moved ahead of the image; rebuild it (docker compose build)"

info "toolchain parity verified: toml $declared == image $recorded == rustc $active"

# ── 2. Environment exports ───────────────────────────────────────────────────
_clang_dir="$(find "$CLANG_SEARCH_ROOT" -maxdepth 3 \( -name 'libclang.so*' -o -name 'libclang-*.so*' \) 2>/dev/null | head -n1 | xargs -r dirname)"
[ -n "$_clang_dir" ] || fail "libclang not found under $CLANG_SEARCH_ROOT — is libclang-dev installed in the image?"
export LIBCLANG_PATH="$_clang_dir"
info "LIBCLANG_PATH=$_clang_dir"

# PKG_CONFIG_PATH / OPENSSL_*: deliberately NOT exported — Debian's standard
# paths already satisfy pkg-config and openssl-sys; the Nix shell hook needs
# them only because Nix store paths are nonstandard.

exec "$@"
