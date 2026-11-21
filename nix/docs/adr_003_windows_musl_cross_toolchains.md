# ADR 003: No Windows/musl Cross Toolchains in the Engine Environment

**Status:** Accepted / Decision
**Date:** 2026-11-21
**Area:** `nix/rust.nix`, `nix/host-deps.nix`
**Authors:** SnifferEngine Core Team

---

## 1. Context and Problem Statement

The sibling `snifferlauncher` environment deliberately ships cross toolchains for its whole distribution matrix: Windows via MinGW-w64 (`pkgsCross.mingwW64`), static Linux via musl (`pkgsCross.musl64`, target `x86_64-unknown-linux-musl`), plus Android — because the launcher's packaging story needs those artifacts built from a developer machine.

Vault plan stage 2.5 asked whether the engine's Nix environment should match: add MinGW and musl cross toolchains so the shell can cross-build the engine the way the launcher shell can.

The engine's existing facts:

- Windows is proven by `check.yml`'s `validate-windows` job on a **native `windows-latest` MSVC runner** (`dtolnay/rust-toolchain` with `toolchain: 1.98.0`; clippy, check, tests, and a release build all run there).
- The engine's Rust target matrix is exactly the host (`x86_64-unknown-linux-gnu`) plus the four Android targets — there is no musl target, gate, or artifact anywhere in the project.
- The standing rule from ADR 001 and the parity gates: **the environment mirrors what CI proves — nothing more.**

## 2. Decision: Defer — No MinGW, No musl

The engine environment ships **no** Windows or musl cross toolchains. The launcher divergence is deliberate and stays.

### 2.1 Reasoning

1. **MinGW is not the Windows oracle.** A green MinGW build would prove a binary the project never ships: different CRT, different link semantics, different codegen than the MSVC toolchain that `validate-windows` actually runs. That is a false-confidence target, and this repository has already paid for a false green once (the rustup-shim incident, vault plan stage 3.5) — the policy is to never wire a proof that answers a question nobody asks.
2. **musl has no consumer.** The engine has no musl target in `rust-toolchain`-adjacent surfaces, no CI gate, and no distributed musl artifact. A musl cross toolchain in the shell would be an unvalidated target in a repo whose whole validation philosophy is target-for-target parity with CI.
3. **The launcher's divergence is justified by its own story.** The launcher is an end-product with a distribution matrix (self-contained desktop binaries); the engine is a library whose contract is defined by the CI matrix. Module-layout parity with the launcher (ADR 001) does not require target-matrix parity.
4. **Closure cost.** MinGW and musl toolchains add gigabytes to the host shell closure — a download and eval tax on every `validate-shell`/`validate-gates` run, paid for a proof nobody consumes.

## 3. Revisit Trigger

Revisit when the engine's CI matrix itself gains a musl or a MinGW target — a musl artifact for an embedded consumer, or a windows-gnu build step. The environment then follows the matrix, per the standing rule; `nix/rust.nix` gains the target and the parity gates extend with it.

## 4. Consequences

### Positive
- The environment stays exactly as broad as its proof: host Linux plus Android.
- The host shell closure stays minimal; no cross-toolchain download tax on CI.
- Launcher parity remains honest: same modules and style, with the target difference recorded as an intentional decision rather than an oversight.

### Negative / Trade-offs
- A developer cannot cross-build the engine for Windows or musl from the Nix shell. Windows remains reachable only through the CI oracle (`validate-windows`), which is the intended contract.
