# ==============================================================================
# SnifferEngine — host build dependencies (vault plan 0001 — Stage 1.4)
# ==============================================================================
# Nix equivalents of the system packages the consolidated CI installs on the
# Linux job (.github/workflows/check.yml, validate-linux) plus the runtime
# tools scripts/check.sh needs. Ubuntu runners preinstall part of the X11
# client stack implicitly, so the apt list under-reports what winit + glutin
# actually link; this module declares the full closure explicitly instead.
#
# Consumed by shell.nix as: import ./nix/host-deps.nix { inherit pkgs; }
# ==============================================================================

{ pkgs }:

[
  # -- Graphics / windowing (winit + glutin desktop platforms) ----------------
  pkgs.mesa # libGL + libEGL + libGLESv2; DRI drivers live in "out" (apt: libgl1-mesa-dri / -dev)
  pkgs.libx11 # apt: libx11-dev (canonical by-name; xorg.* scope is deprecated upstream)
  pkgs.libxcursor
  pkgs.libxrandr
  pkgs.libxi
  pkgs.libxinerama
  pkgs.wayland # apt: libwayland-dev
  pkgs.libxkbcommon # apt: libxkbcommon-dev
  pkgs.alsa-lib # apt: libasound2-dev
  pkgs.gtk3 # apt: libgtk-3-dev

  # -- System libraries --------------------------------------------------------
  pkgs.openssl # apt: libssl-dev
  pkgs.cacert # TLS roots — SSL_CERT_FILE etc. exported by nix/shell-hook.nix
  pkgs.zlib

  # -- Build tooling (apt: build-essential, cmake; bindgen backend) -------------
  pkgs.pkg-config
  pkgs.cmake
  pkgs.ninja
  pkgs.clang
  pkgs.llvmPackages.libclang # bindgen backend — LIBCLANG_PATH (stage 2 env)

  # -- scripts/check.sh runtime --------------------------------------------------
  pkgs.nodejs # step 3 — framework JS syntax
  pkgs.python3 # step 8 — patched-crate freshness tracker
  pkgs.git # step 8 — upstream freshness queries
  pkgs.cargo-binstall
  pkgs.cargo-machete # step 7 — unused dependency check
  pkgs.cargo-ndk # stage 2 — Android cross checks (CI parity)
  pkgs.xvfb-run # apt: xvfb — headless X server wrapper for GL smoke runs
]
