# ADR 002: Android Build Interface — cargo-ndk Only

**Status:** Accepted / Decision
**Date:** 2026-11-21
**Area:** `nix/`, `shell-android.nix`, `.github/workflows/check.yml`
**Authors:** SnifferEngine Core Team

---

## 1. Context and Problem Statement

The sibling `snifferlauncher` shell hook takes a different road than the engine for Android cross builds: it **generates a `.cargo/config.toml`** (its Step 1) and wires linkers by hand — including per-target exports such as `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="…/aarch64-linux-android35-clang"`, which even hardcodes the API level into the linker name. That makes plain `cargo build --target …` work without any subcommand wrapper, at the cost of a second, hand-maintained wiring between the shell and the toolchain.

Vault plan stage 2.4 asked whether the engine's Android shell should match that pattern: auto-generate the (git-ignored) `.cargo/config.toml` so plain target builds work without `cargo-ndk`.

The engine's existing facts:

- Every Android gate the project runs is a `cargo ndk` invocation — `check.yml`'s `validate-android` job (clippy/check across four targets, plus the release `.rlib` artifact build) and the Nix `validate-android` job that mirrors it.
- The Android shell hook already exports the CI's exact `BINDGEN_EXTRA_CLANG_ARGS_*` values, which bindgen consumes regardless of who drives cargo.
- `.cargo/` is git-ignored (`.gitignore` line 37).

## 2. Decision: Defer — cargo-ndk Is the Sole Interface

The engine does **not** generate a `.cargo/config.toml` and does **not** add manual `CARGO_TARGET_*_LINKER` exports. `cargo ndk` is the only supported Android cross-build interface, identical in CI and in the Nix shell. Flakes-style future options and launcher improvements may flow back to the launcher instead.

### 2.1 Reasoning

1. **CI parity over convenience.** The engine's rule is that the Nix environment mirrors what CI proves, byte for byte. CI proves `cargo ndk` commands; a config file would wire an interface (`cargo build --target`) that no gate ever runs — a promise nobody tests.
2. **One source of linker truth.** `cargo-ndk` derives the clang, the compiler-rt builtins, and the linker arguments from `ANDROID_NDK_ROOT` and the NDK version at invocation time. A config file duplicates those paths as static strings and drifts independently — the launcher's hardcoded `aarch64-linux-android35-clang` (API level baked into a filename) is a live demonstration of how this wiring rots.
3. **Review invisibility.** `.cargo/` is git-ignored, so a hook-generated config changes build behavior without ever appearing in a diff. In a repository that reviews everything — SHA-pinned actions, hashed patched crates, a pin layer with anchors — a load-bearing untracked file is a culture violation, not just a risk.
4. **The shim incident as precedent.** The rustup-shim incident (vault plan stage 3.5) showed how expensive it is when two resolutions compete for the same build step. A config file plus `cargo-ndk`'s env injection is exactly two competing resolutions by another route.

## 3. Revisit Trigger

Revisit when an engine consumer genuinely needs plain-target builds without `cargo-ndk` — for example an IDE integration that invokes `cargo build --target` directly. Then generate the config from the same pinned Nix composition inside the shell hook, and extend `scripts/android_parity_gate.sh` so the generated linkers are asserted against the pinned NDK — never handwritten.

## 4. Consequences

### Positive
- Exactly one Android interface everywhere; `validate-android` proves the same command developers run.
- Nothing load-bearing lives in a git-ignored path.
- The linker surface stays derived (NDK-root-relative) instead of duplicated.

### Negative / Trade-offs
- Plain `cargo build --target aarch64-linux-android` does not link out of the box; the supported path is `cargo ndk …`, which both the Nix Android shell and CI provide on `PATH` with the NDK exported.
