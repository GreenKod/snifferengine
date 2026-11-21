#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine Android Environment Parity Gate (vault plan 0001 — Stage 2.3)
# ==============================================================================
# Proves that the pinned Nix Android environment and the runner-side CI
# validate-android job declare the SAME toolchain surface, failing loudly
# on drift in either direction:
#
#   nix/android.nix      ↔  .github/workflows/check.yml (validate-android)
#     ndkVersion            sdkmanager "ndk;…" pin + ANDROID_* env exports
#     buildToolsVersions    sdkmanager "build-tools;…"
#     platformVersions      sdkmanager "platforms;android-…"
#   nix/rust.nix         ↔  dtolnay/rust-toolchain targets:
#     android targets       android cross-compile targets
#   nix/shell-hook.nix   ↔  per-step BINDGEN_EXTRA_CLANG_ARGS_*
#   rust-toolchain.toml  ↔  android job toolchain:
#
# Deliberately NOT asserted: platform-tools, cmdline-tools and cmake — the
# runner-side job floats those (preinstalled runner image / sdkmanager
# defaults), so no pinned CI value exists to drift against. The Nix shell
# pins them for reproducibility, not for parity.
#
# Runs locally (bash scripts/android_parity_gate.sh) and in the nix.yml
# skeleton job; needs only bash + coreutils (no nix, no network).
# ==============================================================================
set -euo pipefail
# Regex ranges under the tr_TR (and other) collations silently exclude
# letters such as "i", which would corrupt every [a-z] match below.
# Force deterministic byte semantics for the whole script.
export LC_ALL=C

# Relocate to the repository root so relative paths resolve from anywhere.
cd "$(dirname "$0")/.."

ANDROID_NIX="nix/android.nix"
RUST_NIX="nix/rust.nix"
HOOK_NIX="nix/shell-hook.nix"
TOOLCHAIN_TOML="rust-toolchain.toml"
CHECK_YML=".github/workflows/check.yml"

C_RESET="\033[0m"
C_GREEN="\033[32m"
C_RED="\033[31m"

ok()   { printf "  ${C_GREEN}✓${C_RESET} %s\n" "$*"; }
fail() { printf "  ${C_RED}✗${C_RESET} %s\n" "$*"; failures=$((failures + 1)); }

failures=0

for f in "$ANDROID_NIX" "$RUST_NIX" "$HOOK_NIX" "$TOOLCHAIN_TOML" "$CHECK_YML"; do
    if [ ! -f "$f" ]; then
        echo "FATAL: input file missing: $f" >&2
        exit 1
    fi
done

# The runner-side android job: a 2-space job key, 4+-space body, bounded by
# the next job key at the same indent.
android_block=$(
    awk '/^  validate-android:/{flag=1}
         flag && /^  [A-Za-z0-9_-]+:$/ && !/^  validate-android:/{exit}
         flag' "$CHECK_YML"
)
if [ -z "$android_block" ]; then
    echo "FATAL: validate-android job not found in $CHECK_YML" >&2
    exit 1
fi

echo "SnifferEngine android environment parity (nix ↔ check.yml)"
echo "-----------------------------------------------------------"

# -- 1. NDK version ------------------------------------------------------------
nix_ndk=$(sed -n 's/^[[:space:]]*ndkVersion = "\([^"]*\)";/\1/p' "$ANDROID_NIX" | head -n 1)
ci_ndk_pkg=$({ grep -o 'ndk;[^" ]*' <<<"$android_block" || true; } | sed 's/^ndk;//' | sort -u)
ci_ndk_env=$({ grep -o 'ndk/[0-9.]*' <<<"$android_block" || true; } | sed 's|^ndk/||' | sort -u)

if [ -n "$nix_ndk" ] && [ "$ci_ndk_pkg" = "$nix_ndk" ] && [ "$ci_ndk_env" = "$nix_ndk" ]; then
    ok "NDK ${nix_ndk} (composition = sdkmanager pin = ANDROID_* exports)"
else
    fail "NDK drift: nix='${nix_ndk:-<missing>}' sdkmanager='${ci_ndk_pkg:-<none>}' env='${ci_ndk_env:-<none>}'"
fi

# -- 2. Build-tools versions ---------------------------------------------------
nix_build_tools=$(
    { sed -n 's/.*buildToolsVersions = \[\(.*\)\];/\1/p' "$ANDROID_NIX" || true; } \
        | { grep -o '"[^"]*"' || true; } | tr -d '"' | sort -u
)
ci_build_tools=$({ grep -o 'build-tools;[^" ]*' <<<"$android_block" || true; } | sed 's/^build-tools;//' | sort -u)

if [ -n "$nix_build_tools" ] && [ "$nix_build_tools" = "$ci_build_tools" ]; then
    ok "build-tools: $(echo "$nix_build_tools" | tr '\n' ' ')"
else
    fail "build-tools drift: nix='$(echo "$nix_build_tools" | tr '\n' ' ')' ci='$(echo "$ci_build_tools" | tr '\n' ' ')'"
fi

# -- 3. Platform versions ------------------------------------------------------
nix_platforms=$(
    { sed -n 's/.*platformVersions = \[\(.*\)\];/\1/p' "$ANDROID_NIX" || true; } \
        | { grep -o '"[^"]*"' || true; } | tr -d '"' | sort -u
)
ci_platforms=$({ grep -o 'platforms;android-[0-9]*' <<<"$android_block" || true; } | sed 's/^platforms;android-//' | sort -u)

if [ -n "$nix_platforms" ] && [ "$nix_platforms" = "$ci_platforms" ]; then
    ok "platforms: android-$(echo "$nix_platforms" | tr '\n' ' ')"
else
    fail "platform drift: nix='$(echo "$nix_platforms" | tr '\n' ' ')' ci='$(echo "$ci_platforms" | tr '\n' ' ')'"
fi

# -- 4. Rust android cross targets ---------------------------------------------
nix_android_targets=$(
    { sed -n '/targets = \[/,/\];/p' "$RUST_NIX" || true; } \
        | { grep -o '"[^"]*"' || true; } | tr -d '"' \
        | { grep -E 'android(eabi)?$' || true; } | sort -u
)
ci_android_targets=$(
    { grep -o 'targets: .*' <<<"$android_block" || true; } | head -n 1 \
        | sed 's/^targets: *//' | tr ',' '\n' | sed 's/^ *//;s/ *$//' | sort -u
)

if [ -n "$nix_android_targets" ] && [ "$nix_android_targets" = "$ci_android_targets" ]; then
    ok "rust android targets: $(echo "$nix_android_targets" | tr '\n' ' ')"
else
    fail "rust android target drift: nix='$(echo "$nix_android_targets" | tr '\n' ' ')' ci='$(echo "$ci_android_targets" | tr '\n' ' ')'"
fi

# -- 5. Toolchain channel ------------------------------------------------------
repo_channel=$(sed -n 's/^channel = "\([^"]*\)".*/\1/p' "$TOOLCHAIN_TOML" | head -n 1)
ci_channel=$({ grep -o 'toolchain: [^[:space:]]*' <<<"$android_block" || true; } | head -n 1 | sed 's/^toolchain: *//')

if [ -n "$repo_channel" ] && [ "$repo_channel" = "$ci_channel" ]; then
    ok "toolchain ${repo_channel} (rust-toolchain.toml = CI android job)"
else
    fail "toolchain drift: repo='${repo_channel:-<missing>}' ci='${ci_channel:-<none>}'"
fi

# -- 6. BINDGEN per-target clang args ------------------------------------------
nix_bindgen=$(
    { sed -n 's/^[[:space:]]*export BINDGEN_EXTRA_CLANG_ARGS_\([a-z0-9_]*android[a-z0-9_]*\)="\([^"]*\)".*/\1|\2/p' "$HOOK_NIX" || true; } \
        | sort -u
)
ci_bindgen=$(
    { grep -o 'BINDGEN_EXTRA_CLANG_ARGS_[a-z0-9_]*="[^"]*"' <<<"$android_block" || true; } \
        | sed 's/^BINDGEN_EXTRA_CLANG_ARGS_\([a-z0-9_]*\)="\([^"]*\)"/\1|\2/' | sort -u
)

if [ -n "$nix_bindgen" ] && [ "$nix_bindgen" = "$ci_bindgen" ]; then
    ok "BINDGEN_EXTRA_CLANG_ARGS identical for $(echo "$nix_bindgen" | wc -l) android target(s)"
else
    fail "BINDGEN args drift (target|args):"
    printf '%s\n' '    nix :' "$nix_bindgen" '    ci  :' "$ci_bindgen" | sed 's/^/      /'
fi

# -- Verdict -------------------------------------------------------------------
echo "  (not asserted, CI floats: platform-tools, cmdline-tools, cmake)"
echo

if [ "$failures" -ne 0 ]; then
    echo "Android environment parity: ${failures} check(s) FAILED."
    echo "The Nix environment and the CI android job must move together:"
    echo "  nix/android.nix, nix/rust.nix, nix/shell-hook.nix  ↔  .github/workflows/check.yml"
    exit 1
fi

echo "Android environment parity verified: nix and the CI android job declare the same toolchain."
