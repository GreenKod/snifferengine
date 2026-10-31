# Contributing to SnifferEngine

Thank you for your interest in contributing to SnifferEngine. SnifferEngine is a general-purpose, 120Hz embedded UI, physics, and graphics engine written in Rust with a sandboxed QuickJS (`SnifferUI`) plugin runtime and a stable C ABI native package architecture.

To maintain architectural purity, performance, and security, all contributors must adhere to the guidelines below.

---

## Architectural Purity Rule (Zero Application Leaks)

SnifferEngine is an application-agnostic engine. **Never introduce launcher-specific or host-application-specific identifiers, strings, or workflows (such as `"app_drawer"`, hardcoded plugin names, or app-specific layout hacks) into `crates/*`, `packages/*`, or `framework/*`.**

All engine mechanisms must be parameterized via declarative AST properties (`Element`, `Style`, `ScrollDismissConfig`, `TransitionSpec`) or trait boundaries (`HostPlatformBridge`). Automated CI gates (`! grep -ri "app_drawer" crates/ framework/`) enforce this boundary on every commit and pull request.

---

## Code of Conduct & Style Standards

- **Professional Tone**: Maintain a formal, technical, and objective communication style across issues, pull requests, and commit messages.
- **Zero Warnings**: Code must compile with zero warnings under strict workspace `clippy` lints (`all`, `pedantic`, `nursery`, `cargo`).
- **120Hz Hot-Path Discipline**: Avoid heap allocations (`String::clone`, `format!`, temporary `Vec`s) inside per-frame rendering (`sniffer_render`) or touch-move input loops (`sniffer_platform_android`, `sniffer_platform_desktop`). Prefer zero-copy borrows (`as_deref()`) and pre-allocated buffers.

---

## Development Setup

### Prerequisites

1. **Rust Toolchain**: Rust 1.98.0 (supporting Rust 2024 edition), automatically configured via [`rust-toolchain.toml`](rust-toolchain.toml).
2. **Desktop Dependencies (Linux)**:
   - `libx11-dev`, `libxkbcommon-dev`, `libwayland-dev`, `libgl1-mesa-dev`, `libgtk-3-dev`, `libasound2-dev`, `pkg-config`, `cmake`.
3. **Android NDK Cross-Compilation (Optional)**:
   - Android NDK r26+ and `cargo-ndk`.


### Running Tests

```bash
# Run the full local pre-flight check suite
./scripts/check.sh
```

---

## Quality Checklist

Before submitting a Pull Request, verify that all checks pass cleanly:

```bash
# 1. Verify engine architectural purity
! grep -ri "app_drawer" crates/ framework/

# 2. Run all workspace tests
cargo test --locked --workspace --all-features

# 3. Check for linter warnings (must report 0 warnings)
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# 4. Verify code formatting
cargo fmt --all --check
```

---

## Security Guidelines for Contributions

1. **Host Bridge Authorization**:
   - Every platform method in `HostPlatformBridge` requires a `caller_id: &str`.
   - Sensitive platform operations must be gated by `permission_manager` capability checks.
2. **QuickJS Sandboxing**:
   - JavaScript execution is bounded by an atomic execution deadline (`DeadlineGuard`) and per-plugin memory quota (`maxMemoryMb`).
   - All framework and plugin scripts must validate against SHA-256 digests declared in `manifest.json`.
3. **Native Dynamic Packages (`ADR 001`)**:
   - Shared libraries (`.so` / `.dll`) must adhere to the versioned C ABI contract (`"SNIF"` magic bytes, `abi_version == 1`) and pass cryptographic SHA-256 verification before loading.
