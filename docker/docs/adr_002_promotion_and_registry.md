# ADR 002: Workflow Promotion and Registry Policy

**Status:** Accepted / Policy  
**Date:** 2026-11-21  
**Area:** repository root — `.github/workflows/docker.yml` (decisions only; no wiring)  
**Authors:** SnifferEngine Core Team  

---

## 1. Context and Problem Statement

Vault plan 0002, stage 3.7 closes Section 3 with two decisions that must be recorded before any further infrastructure work:

1. **Required-check promotion:** whether `docker.yml` should gate merges as a required branch-protection check.
2. **Registry publication:** whether the built dev image should be published to a container registry (GHCR) for contributors who skip local builds.

Both decisions operate under the workflow-separation rule (plan rule 7): `check.yml` and `.github/paths.yaml` remain byte-identical — verified by diff against `master` on 2026-11-21 (empty diff, both files). This ADR changes no workflow files.

Current state of `docker.yml` (stages 3.1-3.6): every commit runs `validate-gates` (full in-container `scripts/check.sh`) and `validate-android` (SDK/NDK parity + aarch64/x86_64 cross clippy) — always blocking for the author; a weekly scheduled run opens or updates the `docker-environment-drift` and `docker-digest-drift` tracking issues and closes them on green; base-image digest drift is non-fatal by design.

---

## 2. Decision A — Required-Check Promotion: DEFERRED, Evidence-Gated

The workflow stays **independent and advisory** until evidence accumulates:

- **Promotion trigger:** after **four consecutive green weekly scheduled runs** (roughly one month) **and zero red author-gating runs** in the same period, promote the two validator jobs — `Build Image and Validate Quality Gates` and `Build Image and Validate Android Cross-Checks` — to required status checks in the repository's branch protection for `master`.
- **Promotion procedure (settings only, no code change):** Settings → Branches → Branch protection rule for `master` → Require status checks → select the two validator jobs. The tracker and digest jobs (`drift-tracker`, `Base Image Pin Freshness`) stay **advisory forever**: they are lifecycle signals, not merge gates, and the digest check is non-fatal by design.
- **Rationale:** premature promotion would block merges on infrastructure that has not yet proven one live green cycle. Every-commit runs already gate the author directly; the required-check status only matters for merge-time enforcement once the scheduled lifecycle has demonstrated stability.

## 3. Decision B — Registry Publication (GHCR): NO

The image is **not published** to a registry. Rationale:

1. **The Dockerfile is the artifact.** Every input is pinned (digest-pinned base, sha256-verified rustup and cmdline-tools, toml-driven toolchain), so the image is reproducible from the repository alone, and BuildKit's gha layer cache makes rebuilds incremental for CI and any contributor.
2. **Size.** The Android SDK/NDK layer makes the image multi-gigabyte; registry storage, retention, and pull time add operational burden with no corresponding gain — a cached rebuild is in the same minutes class as pulling the image.
3. **Staleness channel.** A published image invites use of a copy that can lag the repository — exactly the drift the entrypoint's three-way parity assertion polices locally. Keeping a single distribution channel (the repository) keeps the freshness contract unconditional.

**Revisit triggers:** material contributor demand for prebuilt images; CI image-build time becoming a demonstrated bottleneck; any need to share images outside the git context (e.g. external tooling).

---

## 4. Consequences

### Positive
- The bake-in period is objective and short (four weekly greens), and the promotion itself is a settings action — no code change, no workflow edit.
- Refusing the registry keeps the maintenance surface at exactly one artifact (the repository) and one cache (gha), with no staleness channel to police.

### Negative / Trade-offs
- Until promotion, scheduled failures are advisory (a labeled issue) rather than merge-blocking; mitigated by the every-commit author gating, which is blocking from day one.
- Contributors always build locally; mitigated by the incremental gha cache and the one-command compose flow.
