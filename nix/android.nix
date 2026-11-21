# ==============================================================================
# SnifferEngine — Android SDK + NDK composition (vault plan 0001 — Stages 2.1
# and 2.2, pulled forward as the infrastructure of the validate-android job)
# ==============================================================================
# Composes the Android SDK with androidenv, pinned to the exact set the
# consolidated CI proves on the validate-android job (.github/workflows/
# check.yml): NDK 26.1.10909125, build-tools 34.0.0, platform 34. On the
# runner side several of these float (sdkmanager defaults); this module pins
# what the CI actually uses, so the Nix environment and CI prove the same
# toolchain and drift between them is impossible by construction.
#
# License acceptance is centralized in the entry points' pkgs config
# (android_sdk.accept_license); this module never accepts licenses itself.
#
# Consumed by shell-android.nix as:
#   android = import ./nix/android.nix { inherit pkgs; };
#   # packages += [ android.sdk ]; the hook exports ANDROID_* (android mode)
# ==============================================================================

{ pkgs }:

let
  # The NDK version proven by .github/workflows/check.yml (validate-android).
  ndkVersion = "26.1.10909125";

  androidComposition = pkgs.androidenv.composeAndroidPackages {
    cmdLineToolsVersion = "13.0";
    toolsVersion = "26.1.1";
    platformToolsVersion = "35.0.2";
    buildToolsVersions = [ "34.0.0" ];

    includeEmulator = false; # cross-compilation checks only, no emulator
    platformVersions = [ "34" ];
    includeSources = false;
    includeSystemImages = false;

    includeNDK = true;
    ndkVersions = [ ndkVersion ];

    cmakeVersions = [ "3.22.1" ];
  };

in
{
  # The NDK version string, exported by the shell hook as ANDROID_NDK_ROOT.
  # Single source of truth for the composition above.
  inherit ndkVersion;

  # The composed SDK derivation: platform-tools, build-tools 34.0.0,
  # platform 34, NDK 26.1.10909125, cmdline-tools 13.0, cmake 3.22.1.
  sdk = androidComposition.androidsdk;
}
