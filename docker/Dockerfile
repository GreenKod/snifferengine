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

# System dependency closure (stage 1.3): the consolidated CI's Linux apt list
# (.github/workflows/check.yml, validate-linux) plus the explicit X11 client
# stack — a container cannot rely on the implicit preinstalls of GitHub's
# ubuntu runners, the same lesson encoded in nix/host-deps.nix. GL uses the
# CI names (libgl1-mesa-*) plus the explicit libegl/libgles split for the
# wayland/EGL path. Every package name below is verified against the Debian
# trixie package index (binary-amd64, 2026-11-21). Debian's xvfb package
# ships the xvfb-run wrapper.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      xz-utils \
      build-essential \
      pkg-config \
      cmake \
      ninja-build \
      clang \
      libclang-dev \
      libgl1-mesa-dri \
      libgl1-mesa-dev \
      libegl-dev \
      libgles-dev \
      mesa-utils \
      libx11-dev \
      libxcursor-dev \
      libxrandr-dev \
      libxi-dev \
      libxinerama-dev \
      libxkbcommon-dev \
      libxkbcommon-x11-dev \
      libwayland-dev \
      libasound2-dev \
      libgtk-3-dev \
      libssl-dev \
      zlib1g-dev \
      nodejs \
      python3 \
      git \
      xvfb

# ------------------------------------------------------------------------------
# toolchain — version-pinned rustup + toolchain from rust-toolchain.toml
# ------------------------------------------------------------------------------
FROM base AS toolchain

# rustup acquisition: versioned static archive with the sha256 recorded and
# verified at build time — never the floating sh.rustup.rs entry point
# (ADR 001 §3.2).
ARG RUSTUP_VERSION=1.29.2
ARG RUSTUP_SHA256=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4

RUN curl -fsSL \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
      -o /tmp/rustup-init && \
    echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y --no-modify-path --profile minimal --default-toolchain none && \
    rm /tmp/rustup-init

# Toolchain: installed FROM the repository's rust-toolchain.toml (channel,
# minimal profile, clippy + rustfmt). The channel is never duplicated into
# this file — parity by construction (ADR 001 §3.3). The argument-less
# `rustup toolchain install` and the component/target commands below all
# resolve the same toml (verified empirically against rustup 1.29.x).
COPY rust-toolchain.toml /tmp/toolchain-toml/rust-toolchain.toml
WORKDIR /tmp/toolchain-toml

RUN --mount=type=cache,target=/usr/local/rustup/downloads \
    rustup toolchain install && \
    rustup component add rust-src && \
    rustup target add \
      x86_64-unknown-linux-gnu \
      aarch64-linux-android \
      armv7-linux-androideabi \
      i686-linux-android \
      x86_64-linux-android

# Record the image's toolchain for the entrypoint's runtime parity assertion
# (stage 1.5): written under the toml context, so it captures exactly what
# this image ships. RUSTUP_HOME is image-internal (never volume-mounted in
# docker-compose.yml), so the record cannot drift.
RUN rustc --version | awk '{print $2}' > /usr/local/rustup/image-toolchain-version

# scripts/check.sh runtime tools: explicitly pinned crate versions
# (crates.io, 2026-11-21). Neither crate ships a Cargo.lock, so --locked is
# impossible; free transitive resolution is the accepted trade-off for
# source installs of version-pinned tools.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo install --version 0.10.1 cargo-machete && \
    cargo install --version 4.1.2 cargo-ndk

WORKDIR /workspace

# ------------------------------------------------------------------------------
# android — pinned SDK + NDK composition (vault plan 0002 — stages 2.1/2.2)
# ------------------------------------------------------------------------------
FROM toolchain AS android

# Java is consumed ONLY by sdkmanager while building this layer. Debian
# trixie no longer ships openjdk-17, so Java 21 from apt serves the builder;
# the consolidated CI installs temurin 17 for the same purpose, and no
# engine artifact depends on Java (cargo ndk links through the NDK's clang).
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends openjdk-21-jdk-headless unzip

ENV ANDROID_HOME=/opt/android-sdk
ENV JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
ENV PATH=${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:$PATH

# cmdline-tools 13.0 from the static Google archive, sha256 verified at build
# time (CI parity: setup-android provides the same tool to check.yml).
ARG CMDLINE_TOOLS_SHA256=7ec965280a073311c339e571cd5de778b9975026cfcbe79f2b1cdcb1e15317ee

RUN curl -fsSL \
      "https://dl.google.com/android/repository/commandlinetools-linux-13114758_latest.zip" \
      -o /tmp/cmdline-tools.zip && \
    echo "${CMDLINE_TOOLS_SHA256}  /tmp/cmdline-tools.zip" | sha256sum -c - && \
    mkdir -p "${ANDROID_HOME}/cmdline-tools" && \
    unzip -q /tmp/cmdline-tools.zip -d /tmp/cmdline-tools-extract && \
    mv /tmp/cmdline-tools-extract/cmdline-tools "${ANDROID_HOME}/cmdline-tools/latest" && \
    rm -rf /tmp/cmdline-tools.zip /tmp/cmdline-tools-extract && \
    yes | sdkmanager --licenses > /dev/null && \
    sdkmanager "platform-tools" "ndk;26.1.10909125" "build-tools;34.0.0" "platforms;android-34"

# The NDK/SDK versions are single-sourced against check.yml:
# docker/android-parity.sh fails CI loudly on drift.
ENV ANDROID_NDK_ROOT=${ANDROID_HOME}/ndk/26.1.10909125
ENV PATH=${ANDROID_NDK_ROOT}/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH

# CI-parity bindgen targets (check.yml validate-android values): the aarch64
# triple pins API 30, the others use the NDK default API level.
ENV BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--target=aarch64-linux-android30"
ENV BINDGEN_EXTRA_CLANG_ARGS_armv7_linux_androideabi="--target=armv7-linux-androideabi"
ENV BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android="--target=x86_64-linux-android"
ENV BINDGEN_EXTRA_CLANG_ARGS_i686_linux_android="--target=i686-linux-android"

# ------------------------------------------------------------------------------
# dev — the interactive development environment
# ------------------------------------------------------------------------------
FROM android AS dev

WORKDIR /workspace

# Runtime parity contract (stage 1.5): forbid rustup from auto-installing a
# toolchain the image does not ship — the only way to change the toolchain
# is to change rust-toolchain.toml and rebuild the image. The entrypoint
# then asserts toml == image record == active rustc on every start.
ENV RUSTUP_AUTO_INSTALL=0

# The repository arrives via a bind mount owned by the host user; without
# this, git refuses to operate inside /workspace ("dubious ownership").
RUN git config --global --add safe.directory /workspace

# The entrypoint (docker/entrypoint.sh) asserts three-way toolchain parity
# and exports the container environment before handing over. Section 2
# (plan 0002) extends this stage with the pinned Android SDK/NDK.
COPY docker/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["bash"]
