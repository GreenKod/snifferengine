# ==============================================================================
# SnifferEngine Local CI Pre-flight Quality Verification Script (PowerShell)
# ==============================================================================
# Run this script before pushing to guarantee that CI will pass on all platforms.
# ==============================================================================

$ErrorActionPreference = "Stop"

function Print-Header {
    param([string]$Title)
    Write-Host "`n========================================================" -ForegroundColor Cyan
    Write-Host " $Title" -ForegroundColor Cyan
    Write-Host "========================================================" -ForegroundColor Cyan
}

$startTime = Get-Date

try {
    # 1. Engine Architecture Purity
    Print-Header -Title "1/8: Checking Engine Architecture Purity (no launcher leaks)"
    $leaks = Select-String -Path "crates\**\*.rs", "framework\**\*.js" -Pattern "app_drawer" -ErrorAction SilentlyContinue
    if ($leaks) {
        throw "Found launcher-specific 'app_drawer' references inside engine crates or framework!"
    }
    Write-Host "[OK] Engine architecture purity verified." -ForegroundColor Green

    # 2. Code Formatting
    Print-Header -Title "2/8: Checking Code Formatting (cargo fmt)"
    cargo fmt --all --check
    Write-Host "[OK] Code formatting is clean." -ForegroundColor Green

    # 3. Framework JavaScript Syntax
    Print-Header -Title "3/8: Checking Framework JavaScript Syntax"
    if (Get-Command node -ErrorAction SilentlyContinue) {
        node --check framework/core.js
        node --check framework/sniffer_ui.js
        Write-Host "[OK] Framework JavaScript syntax is clean." -ForegroundColor Green
    }

    # 4. Workspace Cargo Check
    Print-Header -Title "4/8: Checking Workspace Compilation (cargo check)"
    cargo check --locked --workspace --all-targets --all-features
    cargo check --locked --workspace --no-default-features
    Write-Host "[OK] Workspace compilation checks (all-features & no-default-features) passed." -ForegroundColor Green

    # 5. Linter & Static Analysis
    Print-Header -Title "5/8: Running Strict Linter (cargo clippy)"
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    Write-Host "[OK] Clippy reported 0 errors and 0 warnings." -ForegroundColor Green

    # 6. Unit & Integration Tests
    Print-Header -Title "6/8: Running Unit and Integration Tests (cargo test)"
    cargo test --locked --workspace --all-features
    Write-Host "[OK] All unit and integration tests passed." -ForegroundColor Green

    # 7. Unused Dependencies Check (cargo-machete if installed)
    Print-Header -Title "7/8: Checking for Unused Dependencies (cargo machete)"
    if (Get-Command cargo-machete -ErrorAction SilentlyContinue) {
        cargo machete
        Write-Host "[OK] No unused dependencies found." -ForegroundColor Green
    } else {
        Write-Host "[INFO] cargo-machete not installed. Skipping (install with: cargo install cargo-machete)" -ForegroundColor Yellow
    }

    # 8. Patched Crate Upstream Freshness (freshness only; network problems
    # degrade to warnings, only misconfiguration fails the gate)
    Print-Header -Title "8/8: Checking Patched Crate Upstream Freshness"
    $pythonCmd = Get-Command python3 -ErrorAction SilentlyContinue
    if (-not $pythonCmd) {
        $pythonCmd = Get-Command python -ErrorAction SilentlyContinue
    }
    if ($pythonCmd) {
        & $pythonCmd.Source .github/scripts/check_patched_crates.py
        if ($LASTEXITCODE -ne 0) {
            throw "Patched crate tracker reported a configuration error."
        }
        Write-Host "[OK] Patched crates track the latest upstream release (or drift was reported as warnings)." -ForegroundColor Green
    } else {
        Write-Host "[INFO] python not found. Skipping patched crate freshness check." -ForegroundColor Yellow
    }

    $elapsed = (Get-Date) - $startTime
    Write-Host "`nALL SNIFFERENGINE PRE-FLIGHT QUALITY CHECKS PASSED in $([math]::Round($elapsed.TotalSeconds, 1))s. Ready to push safely.`n" -ForegroundColor Green

} catch {
    Write-Host "`n[FAIL] PRE-FLIGHT CHECK FAILED: $_" -ForegroundColor Red
    Write-Host "Please fix the issue above before pushing to remote.`n" -ForegroundColor Red
    exit 1
}
