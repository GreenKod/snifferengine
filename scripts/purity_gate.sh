#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine Architecture Purity Gate
# ==============================================================================
# Fails when launcher-specific concepts leak into the engine. This script is
# the single source of truth for engine purity rules: scripts/check.sh and the
# CI pre-flight job both execute it, so local and CI gates never drift apart.
# ==============================================================================
set -uo pipefail

failures=0

report_violation() {
    echo "[FAIL] $1"
    failures=$((failures + 1))
}

# -----------------------------------------------------------------------------
# Group A: launcher identities and launcher plugin IPC channel names. These
# tokens must never appear anywhere in engine code, platform crates included.
# -----------------------------------------------------------------------------
for token in 'app_drawer' 'dock\.' 'dock_app' 'app_item_'; do
    matches=$(grep -rniE -- "$token" crates/ framework/ || true)
    if [ -n "$matches" ]; then
        echo "$matches"
        report_violation \
            "Launcher-specific token '$token' leaked into the engine (see matches above)."
    fi
done

# -----------------------------------------------------------------------------
# Group B: concepts the platform hosts may legitimately own (for example their
# app-icon scheme registration) but the generic engine layers must not name.
# Scoped to source directories so tests can intentionally reference retired
# magic names when proving they are no longer special.
# -----------------------------------------------------------------------------
generic_layers=(
    crates/sniffer_core/src
    crates/sniffer_render/src
    crates/sniffer_plugin/src
    crates/sniffer_pkg/src
    crates/sniffer_engine/src
    packages/*/src
    framework
)
for token in 'page_indicator_container' 'app-icon'; do
    matches=$(grep -rniE -- "$token" "${generic_layers[@]}" 2>/dev/null || true)
    if [ -n "$matches" ]; then
        echo "$matches"
        report_violation \
            "Token '$token' belongs to the embedding host, not the generic engine (see matches above)."
    fi
done

if [ "$failures" -ne 0 ]; then
    echo "Engine purity gate: $failures rule group(s) violated."
    exit 1
fi

echo "Engine purity verified: no launcher-specific references in engine layers."
