#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine Local CI Pre-flight Quality Verification Script (Bash)
# ==============================================================================
set -euo pipefail

echo -e "\n\033[1;36m========================================================\033[0m"
echo -e "\033[1;36m 🚀 SnifferEngine Local CI Pre-flight Quality Check\033[0m"
echo -e "\033[1;36m========================================================\033[0m\n"

echo -e "\033[1;34m[1/7] Checking Engine Architecture Purity (no launcher leaks)...\033[0m"
if grep -ri "app_drawer" crates/ framework/; then
    echo -e "\033[1;31m❌ Found launcher-specific 'app_drawer' references inside engine crates or framework!\033[0m"
    exit 1
fi

echo -e "\033[1;34m[2/7] Checking Code Formatting (cargo fmt)...\033[0m"
cargo fmt --all --check

echo -e "\033[1;34m[3/7] Checking Framework JavaScript Syntax...\033[0m"
if command -v node &> /dev/null; then
    node --check framework/core.js
    node --check framework/sniffer_ui.js
fi

echo -e "\033[1;34m[4/7] Checking Workspace Compilation (cargo check)...\033[0m"
cargo check --locked --workspace --all-targets --all-features
cargo check --locked --workspace --no-default-features

echo -e "\033[1;34m[5/7] Running Strict Linter (cargo clippy)...\033[0m"
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

echo -e "\033[1;34m[6/7] Running Unit & Integration Tests (cargo test)...\033[0m"
cargo test --locked --workspace --all-features

echo -e "\033[1;34m[7/7] Checking for Unused Dependencies (cargo machete)...\033[0m"
if command -v cargo-machete &> /dev/null; then
    cargo machete
else
    echo -e "\033[0;33mℹ cargo-machete not found, skipping.\033[0m"
fi

echo -e "\n\033[1;32m🎉 ALL SNIFFERENGINE QUALITY GATES PASSED! Ready to push safely. 🎉\033[0m\n"
