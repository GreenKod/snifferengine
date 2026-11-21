# ==============================================================================
# shell-android.nix — SnifferEngine Android cross-compilation environment
# ==============================================================================
#
# The Android twin of shell.nix: same pinned inputs, same toolchain, same
# host dependencies, plus the pinned Android SDK/NDK composition
# (nix/android.nix) and the android-mode shell hook (ANDROID_* exports).
#
# shell.nix stays host-only so host validations never download the ~GBs
# Android SDK; cross-compilation work and the validate-android CI job use
# this entry point instead.
#
# Usage:
#   nix-shell shell-android.nix                  # enter the android shell
#   nix-shell shell-android.nix --pure --run \
#     "cargo ndk --target aarch64-linux-android check --locked --workspace"
# ==============================================================================

let
  pins = import ./nix/pins.nix;

  pkgs = pins.importNixpkgs {
    config = {
      allowUnfree = true; # Android SDK terms
      android_sdk.accept_license = true;
    };
  };

  rust = import ./nix/rust.nix { inherit pkgs; };

  hostDeps = import ./nix/host-deps.nix { inherit pkgs; };

  android = import ./nix/android.nix { inherit pkgs; };

in
pkgs.mkShell {
  name = "snifferengine-android-dev";

  packages = [
    rust.toolchain
    android.sdk
  ] ++ hostDeps;

  # Host environment exports, hard toolchain parity, and the android-mode
  # ANDROID_HOME / NDK / BINDGEN exports (vault plan stages 2.1-2.2, 3.5).
  shellHook = import ./nix/shell-hook.nix {
    inherit pkgs;
    expectedChannel = rust.channel;
    nixpkgsRev = pins.nixpkgs.rev;
    androidSdk = android.sdk;
    ndkVersion = android.ndkVersion;
  };
}
