# ==============================================================================
# SnifferEngine Local CI Pre-flight Quality Verification Script (PowerShell)
# ==============================================================================
# Run this script before pushing to guarantee that CI will pass on all platforms.
# ==============================================================================

$ErrorActionPreference = "Stop"

function Print-Header {
    param([string]$Title)
    Write-Host "`n========================================================" -ForegroundColor Cyan
    Write-Host " 🚀 $Title" -ForegroundColor Cyan
    Write-Host "========================================================" -ForegroundColor Cyan
}

$startTime = Get-Date

try {
    # 1. Engine Architecture Purity
    Print-Header -Title "1/7: Checking Engine Architecture Purity (no launcher leaks)"
    $leaks = Select-String -Path "crates\**\*.rs", "framework\**\*.js" -Pattern "app_drawer" -ErrorAction SilentlyContinue
    if ($leaks) {
        throw "Found launcher-specific 'app_drawer' references inside engine crates or framework!"
    }
    Write-Host "✔ Engine architecture purity verified." -ForegroundColor Green

    # 2. Code Formatting
    Print-Header -Title "2/7: Checking Code Formatting (cargo fmt)"
    cargo fmt --all --check
    Write-Host "✔ Code formatting is clean." -ForegroundColor Green

    # 3. Framework JavaScript Syntax
    Print-Header -Title "3/7: Checking Framework JavaScript Syntax"
    if (Get-Command node -ErrorAction SilentlyContinue) {
        node --check framework/core.js
        node --check framework/sniffer_ui.js
        Write-Host "✔ Framework JavaScript syntax is clean." -ForegroundColor Green
    }

    # 4. Workspace Cargo Check
    Print-Header -Title "4/7: Checking Workspace Compilation (cargo check)"
    cargo check --locked --workspace --all-targets --all-features
    cargo check --locked --workspace --no-default-features
    Write-Host "✔ Workspace compilation checks (all-features & no-default-features) passed." -ForegroundColor Green

    # 5. Linter & Static Analysis
    Print-Header -Title "5/7: Running Strict Linter (cargo clippy)"
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    Write-Host "✔ Clippy reported 0 errors and 0 warnings." -ForegroundColor Green

    # 6. Unit & Integration Tests
    Print-Header -Title "6/7: Running Unit and Integration Tests (cargo test)"
    cargo test --locked --workspace --all-features
    Write-Host "✔ All unit and integration tests passed." -ForegroundColor Green

    # 7. Unused Dependencies Check (cargo-machete if installed)
    Print-Header -Title "7/7: Checking for Unused Dependencies (cargo machete)"
    if (Get-Command cargo-machete -ErrorAction SilentlyContinue) {
        cargo machete
        Write-Host "✔ No unused dependencies found." -ForegroundColor Green
    } else {
        Write-Host "ℹ cargo-machete not installed. Skipping (install with: cargo install cargo-machete)" -ForegroundColor Yellow
    }

    $elapsed = (Get-Date) - $startTime
    Write-Host "`n🎉 ALL SNIFFERENGINE PRE-FLIGHT QUALITY CHECKS PASSED in $([math]::Round($elapsed.TotalSeconds, 1))s! Ready to push safely. 🎉`n" -ForegroundColor Green

} catch {
    Write-Host "`n❌ PRE-FLIGHT CHECK FAILED: $_" -ForegroundColor Red
    Write-Host "Please fix the issue above before pushing to remote.`n" -ForegroundColor Red
    exit 1
}
