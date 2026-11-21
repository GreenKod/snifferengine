#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine — Android version parity assert (vault plan 0002 — stage 2.3)
# ==============================================================================
# Fails hard when the image's installed Android pieces drift from the pins
# declared in .github/workflows/check.yml (NDK, build-tools, platform).
# Single source of truth: check.yml — the image must never disagree silently.
#
# Runs inside the container (CI parity: the validate-android job runs this
# before the cross-checks) and works locally too: ANDROID_HOME, ANDROID_NDK_ROOT
# and CHECK_YML are environment-overridable for testing.
# ==============================================================================
set -euo pipefail

# Regex ranges under the tr_TR collation silently exclude letters such as
# "i" (the lesson recorded in scripts/update-nix-pins.sh); force byte
# semantics for the whole script.
export LC_ALL=C

ANDROID_HOME="${ANDROID_HOME:-/opt/android-sdk}"
CHECK_YML="${CHECK_YML:-/workspace/.github/workflows/check.yml}"

fail() { printf "android-parity: \033[31m✗ %s\033[0m\n" "$*" >&2; exit 1; }
ok()   { printf "android-parity: \033[32m✓ %s\033[0m\n" "$*" >&2; }

[ -f "$CHECK_YML" ] || fail "check.yml not found at $CHECK_YML"

# ── NDK ───────────────────────────────────────────────────────────────────────
expected_ndk="$(grep -o 'ndk;[0-9.]*' "$CHECK_YML" | head -n1 | cut -d';' -f2)"
[ -n "$expected_ndk" ] || fail "no 'ndk;…' pin found in $CHECK_YML — expected sdkmanager-style pins"
installed_ndk="$(basename "${ANDROID_NDK_ROOT:-}")"
[ -n "$installed_ndk" ] || fail "ANDROID_NDK_ROOT is not set — is this the dev image?"
[ "$installed_ndk" = "$expected_ndk" ] || \
  fail "NDK drift: image ships $installed_ndk, check.yml pins $expected_ndk — rebuild the image against the CI pins"
ok "ndk $installed_ndk matches check.yml"

# ── build-tools ───────────────────────────────────────────────────────────────
expected_bt="$(grep -o 'build-tools;[0-9.]*' "$CHECK_YML" | head -n1 | cut -d';' -f2)"
[ -n "$expected_bt" ] || fail "no 'build-tools;…' pin found in $CHECK_YML"
[ -d "$ANDROID_HOME/build-tools/$expected_bt" ] || \
  fail "build-tools $expected_bt (check.yml pin) is not installed in the image"
ok "build-tools $expected_bt matches check.yml"

# ── platform ─────────────────────────────────────────────────────────────────
expected_platform="$(grep -o 'platforms;android-[0-9]*' "$CHECK_YML" | head -n1 | cut -d';' -f2 | sed 's/android-//')"
[ -n "$expected_platform" ] || fail "no 'platforms;android-…' pin found in $CHECK_YML"
[ -d "$ANDROID_HOME/platforms/android-$expected_platform" ] || \
  fail "platform android-$expected_platform (check.yml pin) is not installed in the image"
ok "platform android-$expected_platform matches check.yml"
