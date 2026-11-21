# ==============================================================================
# SnifferEngine — Pinned External Inputs (single source of truth)
# ==============================================================================
# ADR 001 §3.2: every nix module imports nixpkgs and the rust overlay through
# importNixpkgs below; no other module may import external inputs.
#
# Bump:   bash scripts/update-nix-pins.sh
# Verify: bash scripts/update-nix-pins.sh --verify
# Audit:  bash scripts/update-nix-pins.sh --audit
# ==============================================================================

let
  # A pinned GitHub input: a repository frozen at one branch-head commit,
  # with the NAR hash of the unpacked archive tree. Modern Nix's
  # fetchTarball verifies the NAR hash of the unpacked tree — NOT the
  # archive file hash; the first CI run of the validation workflow proved
  # file hashes are rejected with a NAR hash mismatch. The upstream branch
  # keeps moving; the (rev, sha256) pair below is what actually freezes
  # the input. The "# pin:<name>-rev" / "# pin:<name>-sha256" anchor
  # comments are machine-read by scripts/update-nix-pins.sh — keep them
  # directly above their value lines.
  pin =
    { owner, repo, branch, rev, sha256, description }:
    { inherit owner repo branch rev sha256 description; };

  ghTarball = url: sha256: builtins.fetchTarball { inherit url sha256; };

  pins = {
    # nixpkgs — NixOS stable channel branch, never the unstable one: the
    # engine environment must not silently track a moving target
    # (ADR 001 §1, constraint 5).
    nixpkgs = pin {
      owner = "NixOS";
      repo = "nixpkgs";
      branch = "nixos-26.05";
      description = "NixOS 26.05 stable channel branch";
      # pin:nixpkgs-rev
      rev = "ab5c18fdedfc8f9bbc9d773f10a540790e2787a7";
      # pin:nixpkgs-sha256
      sha256 = "0zbbcrl62rigv70w68k8r293zpvpq25izrzdidkz0qdagqvw59c8";
    };

    # rust-overlay — supplies the exact toolchain declared by
    # rust-toolchain.toml; the parity is asserted by nix/shell-hook.nix
    # and the validation workflow.
    rust-overlay = pin {
      owner = "oxalica";
      repo = "rust-overlay";
      branch = "master";
      description = "Rust toolchain overlay (pinned commit)";
      # pin:rust-overlay-rev
      rev = "52db20539ec0ba063479d66092992d9efcf456ff";
      # pin:rust-overlay-sha256
      sha256 = "10l9f3818z7nnpqdkh8b8gkrkggvazk78i4mmayk2209r21r43jn";
    };
  };
in
{
  inherit (pins) nixpkgs rust-overlay;

  # Imports the pinned nixpkgs with the pinned rust-overlay pre-applied.
  # shell.nix calls this with the repository configuration (unfree
  # packages, Android SDK license acceptance).
  importNixpkgs =
    { config ? { } }:
    import (ghTarball
      "https://github.com/${pins.nixpkgs.owner}/${pins.nixpkgs.repo}/archive/${pins.nixpkgs.rev}.tar.gz"
      pins.nixpkgs.sha256)
      {
        inherit config;
        overlays = [
          (import (ghTarball
            "https://github.com/${pins.rust-overlay.owner}/${pins.rust-overlay.repo}/archive/${pins.rust-overlay.rev}.tar.gz"
            pins.rust-overlay.sha256))
        ];
      };
}
