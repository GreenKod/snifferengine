# ==============================================================================
# SnifferEngine — shell hook (vault plan 0001 — Stage 1.5, ADR 001 §3.3)
# ==============================================================================
# Returns the bash string spliced into mkShell.shellHook. All Nix values are
# interpolated here; the result is pure bash. House style follows the sibling
# launcher's nix/shell-hook.nix (colour helpers, banner, environment summary).
#
# The engine-specific addition is the hard toolchain parity assertion: the
# channel comes from the SAME parse rust.nix performs on rust-toolchain.toml,
# and a mismatched rustc on PATH (e.g. a host rustup shim shadowing the shell
# toolchain) fails shell entry loudly instead of building with the wrong
# toolchain.
#
# Consumed by shell.nix as:
#   import ./nix/shell-hook.nix {
#     inherit pkgs;
#     expectedChannel = <channel parsed by nix/rust.nix>;
#     nixpkgsRev = <nixpkgs rev from nix/pins.nix>;
#   }
# ==============================================================================

{ pkgs, expectedChannel, nixpkgsRev, androidSdk ? null, ndkVersion ? null }:

let
  # Android environment exports — only generated when an SDK composition is
  # passed (shell-android.nix). Mirrors the runner-side env of the CI
  # validate-android job: NDK location on PATH for cargo-ndk, and the exact
  # BINDGEN_EXTRA_CLANG_ARGS the CI sets per target.
  androidSection =
    if androidSdk == null then "" else
    ''
      # -- Android NDK environment (android mode only) ------------------------
      _gh_group "Set up Android SDK / NDK"

      export ANDROID_HOME="${androidSdk}/libexec/android-sdk"
      export ANDROID_NDK_ROOT="''${ANDROID_HOME}/ndk/${ndkVersion}"
      export ANDROID_NDK_HOME="''${ANDROID_NDK_ROOT}"
      export PATH="''${ANDROID_NDK_ROOT}/toolchains/llvm/prebuilt/linux-x86_64/bin:''${PATH}"

      if [ ! -x "''${ANDROID_NDK_ROOT}/toolchains/llvm/prebuilt/linux-x86_64/bin/clang" ]; then
        _gh_error "NDK layout missing at ''${ANDROID_NDK_ROOT}"
        exit 1
      fi

      export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--target=aarch64-linux-android30"
      export BINDGEN_EXTRA_CLANG_ARGS_armv7_linux_androideabi="--target=armv7-linux-androideabi"
      export BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android="--target=x86_64-linux-android"
      export BINDGEN_EXTRA_CLANG_ARGS_i686_linux_android="--target=i686-linux-android"

      _gh_kv "ANDROID_HOME" "''${ANDROID_HOME}"
      _gh_kv "NDK" "''${ANDROID_NDK_ROOT}"
      _gh_ok "Android environment ready"
      _gh_endgroup
    '';
in

''
  # -- ANSI colour helpers (GitHub Actions palette) ----------------------------
  _C_RESET="\033[0m"
  _C_BOLD="\033[1m"
  _C_DIM="\033[2m"
  _C_BLUE="\033[34m"
  _C_CYAN="\033[36m"
  _C_GREEN="\033[32m"
  _C_RED="\033[31m"
  _C_WHITE="\033[97m"

  _gh_group()    { printf "\n''${_C_BOLD}''${_C_BLUE}##[group]%s''${_C_RESET}\n" "$*"; }
  _gh_endgroup() { printf "''${_C_DIM}##[endgroup]''${_C_RESET}\n"; }
  _gh_step()     { printf "  ''${_C_BOLD}''${_C_CYAN}▶ %s''${_C_RESET}\n" "$*"; }
  _gh_ok()       { printf "  ''${_C_GREEN}✓''${_C_RESET}  %s\n" "$*"; }
  _gh_error()    { printf "  ''${_C_RED}✗  Error: %s''${_C_RESET}\n" "$*"; }
  _gh_kv()       { printf "  ''${_C_DIM}%-18s''${_C_RESET} ''${_C_WHITE}%s''${_C_RESET}\n" "$1" "$2"; }

  # -- Banner -------------------------------------------------------------------
  printf "\n''${_C_BOLD}''${_C_BLUE}"
  echo   "╔══════════════════════════════════════════════════════════════════╗"
  echo   "║  🦀  SnifferEngine · Nix development shell                        ║"
  echo   "╚══════════════════════════════════════════════════════════════════╝"
  printf "''${_C_RESET}\n"

  # -- Step 1 — Toolchain parity assertion (ADR 001 §3.3) -----------------------
  _gh_group "Toolchain parity (rust-toolchain.toml)"

  _expected_channel="${expectedChannel}"

  if command -v rustc >/dev/null 2>&1; then
    _actual="$(rustc --version 2>/dev/null)"
    case "''${_actual}" in
      "rustc ''${_expected_channel} "*)
        _gh_ok "rustc ''${_expected_channel} — parity verified"
        ;;
      *)
        _gh_error "Toolchain parity FAILED"
        printf "       expected : rustc ''${_expected_channel} (rust-toolchain.toml)\n"
        printf "       on PATH  : ''${_actual:-none}\n"
        printf "       The shell toolchain must equal the repository declaration.\n"
        printf "       A host rustup shim is shadowing the Nix toolchain.\n"
        exit 1
        ;;
    esac
  else
    _gh_error "rustc not found in the shell environment"
    exit 1
  fi

  _gh_kv "expected" "''${_expected_channel} (rust-toolchain.toml)"
  _gh_kv "actual"   "''${_actual}"
  _gh_endgroup

  # -- Step 2 — Environment exports ---------------------------------------------
  _gh_group "Set up host environment"

  export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
  export BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_linux_gnu="-isystem ${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.llvmPackages.libclang.version}/include -isystem ${pkgs.glibc.dev}/include"

  export PKG_CONFIG_PATH="${pkgs.openssl.dev}/lib/pkgconfig:${pkgs.zlib.dev}/lib/pkgconfig:''${PKG_CONFIG_PATH:-}"
  export LD_LIBRARY_PATH="${pkgs.openssl.out}/lib:${pkgs.zlib.out}/lib:''${LD_LIBRARY_PATH:-}"

  export OPENSSL_DIR="${pkgs.openssl.dev}"
  export OPENSSL_LIB_DIR="${pkgs.openssl.out}/lib"
  export OPENSSL_INCLUDE_DIR="${pkgs.openssl.dev}/include"

  _gh_ok "LIBCLANG_PATH, BINDGEN args, PKG_CONFIG_PATH, LD_LIBRARY_PATH, OPENSSL_*"
  _gh_endgroup
${androidSection}
  # -- Step 3 — Quick commands --------------------------------------------------
  _gh_group "Quick commands"

  alias check="bash scripts/check.sh"
  alias fmt="cargo fmt --all"
  alias lint-linux="cargo clippy --locked --workspace --all-targets --all-features -- -D warnings"
  alias test="cargo test --locked --workspace --all-features"
  alias pins-verify="bash scripts/update-nix-pins.sh --verify"
  alias pins-audit="bash scripts/update-nix-pins.sh --audit"
  alias pins-update="bash scripts/update-nix-pins.sh"

  # -- Step 4 — Environment summary ---------------------------------------------
  _gh_group "Environment summary"

  _gh_step "Toolchain versions"
  _gh_kv "rustc"   "$(rustc --version 2>/dev/null || echo not found)"
  _gh_kv "cargo"   "$(cargo --version 2>/dev/null || echo not found)"
  _gh_kv "node"     "$(node --version 2>/dev/null || echo not found)"
  _gh_kv "python3"  "$(python3 --version 2>/dev/null || echo not found)"
  _gh_kv "nixpkgs" "${nixpkgsRev}"

  _gh_step "Available commands"
  _gh_kv "check"        "run the full local quality gate (scripts/check.sh)"
  _gh_kv "lint-linux"   "strict clippy across the workspace"
  _gh_kv "pins-verify"  "verify pinned nix inputs (rev + sha256)"
  _gh_kv "pins-update"  "bump pinned nix inputs to branch heads"

  _gh_endgroup
  echo ""
''
