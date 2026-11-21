#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine — development container entrypoint (vault plan 0002)
# ==============================================================================
# Stage 1.2 ships this pass-through stub so the dev container is usable
# immediately. Stage 1.5 replaces it with the real contract:
#
#   1. Toolchain parity assertion: `rustc --version` must equal the channel
#      declared in rust-toolchain.toml — mismatch is a hard failure
#      (defense in depth on top of the construction-level parity in the
#      Dockerfile).
#   2. Environment exports: LIBCLANG_PATH, BINDGEN_EXTRA_CLANG_ARGS_*,
#      PKG_CONFIG_PATH, OPENSSL_* — the container-side equivalents of the
#      Nix track's shell hook.
#
# The entrypoint stays pass-through by design: `docker compose run --rm dev
# ./scripts/check.sh` and interactive shells both flow through here.
# ==============================================================================
set -euo pipefail

exec "$@"
