# ADR 001: Classic Pinned Nix Environment Style

**Status:** Accepted / Specification  
**Date:** 2026-11-21  
**Area:** repository root — `nix/`, `shell.nix`  
**Authors:** SnifferEngine Core Team  

---

## 1. Context and Problem Statement

SnifferEngine needs a Nix-managed development environment that satisfies `scripts/check.sh` (8 quality gates) end-to-end with zero manual setup, and a continuous validation workflow that keeps proving the environment is still valid (vault plan `0001-nix-infrastructure-plan.md`).

Two viable styles exist for expressing this environment:

1. **Classic Nix** — `shell.nix` + `nix/` expression modules, evaluated by `nix-shell`. This is the layout already used by the sibling `snifferlauncher` repository (`shell.nix` + `nix/{android,rust,shell-hook,vscode-extensions}.nix`).
2. **Flakes** — `flake.nix` + committed `flake.lock`, evaluated by `nix develop` / `nix flake check`.

### Constraints That Shape the Choice

1. **Launcher reference quality:** The launcher implementation is module-compatible but completely unpinned — floating `<nixpkgs>` channel, `fetchGit ref = "master"` for rust-overlay, and `rust-bin.stable.latest` that ignores `rust-toolchain.toml`. Whatever style the engine adopts, it must add strict pinning and toolchain parity on top; neither style is exempt.
2. **CI runner requirements:** The validation workflow installs Nix on GitHub-hosted runners. Both styles work there, but classic `nix-shell --pure` needs no additional configuration, while flakes require `experimental-features` opt-in on stable Nix releases — an extra failure surface on ephemeral runners.
3. **Toolchain parity is non-negotiable:** The engine pins Rust `1.98.0` in `rust-toolchain.toml` and asserts parity automatically. Both styles can express this; the decision only changes where the pin lives.
4. **Contributor reach:** The environment must work for any contributor with plain Nix installed, including distributions and managed setups where flakes are disabled entirely.
5. **Review culture:** This repository reviews everything as explicit, pinned, auditable changes (SHA-pinned actions, hashed patched crates, dependency age gates).

---

## 2. Decision: Classic Pinned Nix

The engine adopts **classic pinned Nix**: a `shell.nix` entry point importing launcher-compatible modules from `nix/`, with every external input pinned by revision and hash in a single `nix/pins.nix`, and `nix-shell --pure --run "…"` as the canonical validation primitive for both local verification and the continuous validation workflow.

### 2.1 Style Evaluation

| Criterion | Classic Pinned | Flakes |
| --- | --- | --- |
| Contributor compatibility | Works with plain stable Nix, no flags | Requires `experimental-features` opt-in; disabled in some managed setups |
| CI runner setup | No extra installer configuration | Extra experimental-features configuration and drift risk |
| Launcher parity | Module layout is directly comparable and improvable | New style for the same author to maintain in parallel |
| Pin representation | One reviewable `nix/pins.nix` (rev + sha256) | `flake.lock` (diffable, but only with flakes enabled to consume) |
| Validation primitive | `nix-shell --pure --run "…"` — long-established, scriptable | `nix develop -c` / `nix flake check` — equally scriptable, younger tooling |
| Update workflow | Helper script re-prefetches and rewrites pins | `nix flake update` (smoother, but flakes-only) |

Flakes remain a **documented future option** (see §4), not a rejected alternative.

---

## 3. Specification

1. **Layout:** `shell.nix` (entry point) importing `nix/` modules — the same module names and responsibilities as snifferlauncher (`rust.nix`, `host-deps.nix`, `android.nix`, `shell-hook.nix`) plus the new `pins.nix`.
2. **Single pin source:** `nix/pins.nix` imports nixpkgs as a pinned release tarball and rust-overlay as a pinned revision, each with an explicit hash. Every other module imports inputs from `pins.nix` and nowhere else. Hard rule: no `<nixpkgs>`, no `fetchGit ref = "master"`, no `rust-bin.stable.latest`.
3. **Toolchain parity:** `nix/rust.nix` selects the toolchain by the exact `rust-toolchain.toml` channel; the shell hook and the validation workflow both assert `rustc --version` equals the declared channel and fail hard on mismatch.
4. **Pin maintenance:** `scripts/update-nix-pins.sh` performs the prefetch → rewrite → verify cycle reproducibly; pin bumps are ordinary reviewed commits.
5. **Validation primitive:** `nix-shell --pure --run "…"` is the only command the validation workflow needs to prove the environment, because `--pure` guarantees the proof happens against the Nix-provided dependency closure, not the host machine.

---

## 4. Migration Trigger

Revisit this decision when any of the following holds:

- Upstream Nix ships flakes as a stable, flag-free default across the supported contributor base.
- A flake-based shared binary cache (e.g., Cachix/devenv integrations) becomes valuable enough to justify the migration cost.
- snifferlauncher migrates to flakes, restoring cross-repo style parity in the flakes direction.

The module layout in §3 is deliberately kept expression-shaped so a future `flake.nix` can wrap the same modules instead of replacing them.

---

## 5. Consequences

### Positive
- Any contributor with plain Nix gets a fully working environment with one command; no experimental configuration anywhere.
- Launcher module parity: the same author reviews the same structure in both repositories, and pinning/parity improvements made here can flow back to the launcher.
- `nix-shell --pure` gives the continuous validation workflow a proof that is independent of whatever happens to be installed on the runner.
- The pin surface is one small, fully reviewed file instead of a generated lockfile.

### Negative / Trade-offs
- Pin updates require an explicit prefetch step instead of a single `nix flake update`; mitigated by `scripts/update-nix-pins.sh`.
- No lockfile-style diff view of dependency moves; mitigated by keeping every pin in one reviewable file with comments.
- Classic `nix-shell` evaluation is slower than the flake evaluation cache; acceptable because the environment is entered infrequently and CI runs are not latency-critical.
