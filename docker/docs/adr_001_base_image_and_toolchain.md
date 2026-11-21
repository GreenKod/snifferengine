# ADR 001: Digest-Pinned Base Image with Toolchain from rust-toolchain.toml

**Status:** Accepted / Specification  
**Date:** 2026-11-21  
**Area:** repository root — `docker/`, `docker-compose.yml`  
**Authors:** SnifferEngine Core Team  

---

## 1. Context and Problem Statement

Plan 0002 (vault) stages the Docker development environment. Stage 1.1 must fix the base image and the toolchain acquisition strategy before any container file is written, because every later layer and the parity contracts depend on them.

Three viable strategies exist:

1. **Debian slim + rustup driven by the repository** — a minimal Debian base, with the toolchain installed by rustup consuming the repository's own `rust-toolchain.toml`.
2. **Official `rust:<channel>` images** — the toolchain prebuilt into a Docker Hub official image, tagged per Rust release (verified: `rust:1.98.0-slim-bookworm` is active, published 2026-08-25, five days after the 1.98.0 release).
3. **Ubuntu + rustup** — maximal similarity to the GitHub-hosted CI runners (`ubuntu-latest` is 24.04; verified: `ubuntu:24.04` active).

### Constraints That Shape the Choice

1. **Toolchain parity is non-negotiable and by construction.** `rust-toolchain.toml` is the single source of truth (`channel = "1.98.0"`, `profile = "minimal"`, components `clippy`, `rustfmt`). The Nix track already encoded this principle (ADR 001, `nix/rust.nix` parses the toml). Any Docker strategy that duplicates the channel into an image tag reintroduces a drift surface that only a runtime assertion can catch.
2. **Pin culture.** The repository SHA-pins every action and hashes every patched crate; the Docker equivalent is digest pinning (`image@sha256:…`) with no floating tags. A strategy is preferred when *everything* it introduces can be pinned by version or digest.
3. **The dependency contract is the apt list, not the OS identity.** `check.yml` proves the engine on an apt-based system; Debian and Ubuntu both satisfy the list, so exact runner similarity is a convenience, not a requirement.
4. **Image hygiene.** The dev image is rebuilt rarely and layered for cache; a smaller base and long standard support keep maintenance cheap.

---

## 2. Decision: Digest-Pinned Debian Trixie + rustup from the Repository

The Docker environment is built on **`debian:trixie-slim` pinned by digest**, with the Rust toolchain installed by a **version-pinned rustup** consuming the repository's own `rust-toolchain.toml` — parity by construction, identical in spirit to the Nix track's toml-parsing module.

### 2.1 Strategy Evaluation

| Criterion | (a) Debian slim + rustup | (b) Official `rust:<channel>` | (c) Ubuntu + rustup |
| --- | --- | --- | --- |
| Toolchain parity | **By construction** — rustup reads the toml | Channel duplicated into the tag; drift possible, needs runtime assert | **By construction** |
| Base freshness & support | Trixie = current stable (2025-08); standard support to ~2028; verified active | Debian-based; track checked 2026-08-25 for 1.98.0 | 24.04 = runner match; LTS to 2029 |
| Base size | ~30 MB compressed | Toolchain prebuilt: ~250 MB+ base, but **no toolchain download at build time** | ~30 MB compressed |
| Pin surface | Base digest + rustup version — all pinnable | Base digest (toolchain implicit in tag) | Base digest + rustup version |
| CI similarity | Debian family: same apt list works | Debian family | **Identical** to hosted runners |

Option (b) is the honest speed alternative: upstream already did the toolchain download. It was rejected for the parity requirement, not for quality — see §4 for the migration trigger that would reopen it.

Evidence at decision time (Docker Hub, 2026-11-21):

| Image | Status | Digest (manifest list) |
| --- | --- | --- |
| `debian:trixie-slim` | active, pushed 2026-11-05 | `sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2` |
| `debian:bookworm-slim` (fallback) | active, pushed 2026-11-05 | `sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc` |
| `rust:1.98.0-slim-bookworm` | active, pushed 2026-08-25 | `sha256:1469a27c125cb5a3aebfa4f4e4665d935b02fb72cc093b2c974b3d740e43f157` |
| `ubuntu:24.04` | active, pushed 2026-11-20 | `sha256:9c7f200c07e3d4a03e7249d2c16bf66a51120fb876aa2242cf31451412543289` |

---

## 3. Specification

1. **Base pinning:** `docker/Dockerfile` builds `FROM debian:trixie-slim@sha256:<digest>`. The digest is rewritten only by `scripts/update-docker-pins.sh` (plan 0002, stage 4.2); the workflow's scheduled digest-drift check (stage 3.6) tracks upstream movement. Bookworm remains the documented fallback base with its own recorded digest.
2. **rustup acquisition:** rustup itself is installed from the versioned static archive (`https://static.rust-lang.org/rustup/archive/<rustup-version>/<target>/rustup-init`) with its sha256 recorded in the Dockerfile — never the floating `sh.rustup.rs` entry point.
3. **Toolchain installation:** the Dockerfile copies `rust-toolchain.toml` and runs `rustup toolchain install`, which reads the channel, profile, and components directly from the repository declaration; Android targets are added with an explicit `rustup target add` mirroring the target set of `nix/rust.nix`.
4. **Runtime parity assertion:** the container entrypoint (stage 1.5) additionally asserts `rustc --version` equals the toml channel and fails hard on mismatch — defense in depth on top of construction-level parity, matching plan 0002 rule 4.

---

## 4. Migration Trigger

Revisit this decision when any of the following holds:

- The official `rust:<channel>` images demonstrably shorten CI image builds enough to matter, **and** a runtime parity assertion is accepted as the primary drift guard instead of construction-level parity.
- A system dependency appears that cannot be satisfied on Debian trixie while working on Ubuntu 24.04, making exact runner similarity a hard requirement.

The specification in §3 is deliberately layered (base → rustup → toolchain) so a future strategy swap replaces the base and acquisition steps without touching the apt or SDK layers.

---

## 5. Consequences

### Positive
- Toolchain parity by construction: the channel is never duplicated into an image tag, and the entrypoint still asserts it at runtime.
- Smallest currently-supported Debian stable; digest pinning makes every base layer bit-identical for all contributors and CI.
- The apt layer stays cacheable and independent: swapping the base or the toolchain strategy does not rebuild the system-dependency layers.

### Negative / Trade-offs
- The image build downloads the toolchain once (~200 MB) instead of inheriting it from an official image; mitigated by layer caching and versioned, sha256-verified acquisition URLs.
- Trixie is not byte-identical to the Ubuntu 24.04 CI runner; mitigated because the dependency contract is the apt package list, which trixie satisfies, and because the validation workflow (plan 0002, Section 3) proves the gates inside the container rather than trusting runner similarity.
- Digest bumps are manual; mitigated by the update script (stage 4.2) and the scheduled drift check (stage 3.6).
