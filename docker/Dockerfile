# ==============================================================================
# SnifferEngine — development container (vault plan 0002 — Stage 1.2)
# ==============================================================================
# Base image and toolchain acquisition are fixed by ADR 001
# (docker/docs/adr_001_base_image_and_toolchain.md):
#   • base: digest-pinned Debian trixie (current stable; bookworm is the
#     documented fallback, digest recorded in the ADR)
#   • toolchain: version-pinned rustup consuming rust-toolchain.toml — the
#     channel is never duplicated into the image (parity by construction)
#
# Layer ordering strategy (least-frequently-changing first, so bumps rebuild
# as little as possible):
#   1. base        pinned Debian + apt system libraries      (stage 1.3)
#   2. toolchain   rustup + toml-driven toolchain + targets   (stage 1.4)
#   3. dev         entrypoint wiring + (later) Android SDK   (stages 1.5, 2.x)
#
# BuildKit decisions (stage 1.2):
#   • cache mounts (RUN --mount=type=cache) are used for apt and cargo
#     downloads — they accelerate rebuilds without entering image layers
#   • dev-time caches (cargo registry/git, target dir) are NOT in the image:
#     docker-compose.yml provides named volumes for those
#   • no floating `# syntax=` frontend directive; the builtin BuildKit
#     frontend supports everything used here (BuildKit is the default)
#
# The image stays buildable at every vault plan 0002 stage; unfinished
# layers carry explicit stage markers until their stage replaces them.
# ==============================================================================

# ------------------------------------------------------------------------------
# base — digest-pinned Debian stable + system libraries
# ------------------------------------------------------------------------------
FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2 AS base

# rustup/docker convention: shared toolchain homes on PATH for every later
# layer and every compose run (volume mounts target these paths).
ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:$PATH

# ca-certificates are installed already because every later layer needs TLS
# (rustup static archive, toolchain downloads). Stage 1.3 replaces this layer
# with the full apt closure mirroring the CI linux job (mesa/GL, the explicit
# X11 client stack, xkbcommon, wayland, alsa-lib, gtk-3, ssl, zlib, …).
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates

# ------------------------------------------------------------------------------
# toolchain — version-pinned rustup + toolchain from rust-toolchain.toml
# ------------------------------------------------------------------------------
FROM base AS toolchain

# Stage 1.4 fills this layer:
#   • rustup-init from the versioned static archive
#     (https://static.rust-lang.org/rustup/archive/<version>/…) with its
#     sha256 recorded here — never the floating sh.rustup.rs entry point
#   • COPY rust-toolchain.toml + `rustup toolchain install` (reads channel,
#     profile minimal, clippy + rustfmt straight from the repository file)
#   • `rustup target add` for the host + 4 Android targets, mirroring the
#     target set of nix/rust.nix
RUN echo "vault plan 0002 stage 1.4 lands the toml-driven toolchain here"

# ------------------------------------------------------------------------------
# dev — the interactive development environment
# ------------------------------------------------------------------------------
FROM toolchain AS dev

WORKDIR /workspace

# Stage 1.5 replaces the entrypoint stub with the real contract: the hard
# toolchain parity assertion against rust-toolchain.toml and the environment
# exports (LIBCLANG_PATH, BINDGEN_EXTRA_CLANG_ARGS_*, PKG_CONFIG_PATH, …).
# Section 2 (plan 0002) extends this stage with the pinned Android SDK/NDK.
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["bash"]
