#!/usr/bin/env bash
# ==============================================================================
# SnifferEngine Nix Pin Maintenance (vault plan 0001 — Stage 1.2, ADR 001 §3.2)
# ==============================================================================
# Verifies, audits, or bumps the pinned inputs in nix/pins.nix.
#
# Usage (from the repository root, or from anywhere — the script relocates):
#   bash scripts/update-nix-pins.sh            # bump every pin to its branch head
#   bash scripts/update-nix-pins.sh --verify    # verify every (rev, sha256) pair
#   bash scripts/update-nix-pins.sh --audit     # fail on floating nix references
#
# Hashing strategy:
#   1. nix-prefetch-url when a Nix installation is available.
#   2. curl + sha256sum + an embedded reimplementation of Nix's nix32
#      encoding (upstream: src/libutil/base-nix-32.cc) otherwise, so the
#      script also works on machines without Nix. The fallback needs
#      python3; downloads are cached in $PIN_CACHE_DIR and reused.
#
# Non-goals: this script never edits anything outside nix/pins.nix.
# ==============================================================================
set -euo pipefail
# Regex ranges under the tr_TR (and other) collations silently exclude
# letters such as "i", which would corrupt every sha256 matching and
# rewrite below. Force deterministic byte semantics for the whole script.
export LC_ALL=C

PINS_NIX="nix/pins.nix"
PINS="nixpkgs rust-overlay"
PIN_CACHE_DIR="${PIN_CACHE_DIR:-${TMPDIR:-/tmp}/sniffers-nix-pins}"

C_RESET="\033[0m"
C_RED="\033[31m"
C_GREEN="\033[32m"
C_YELLOW="\033[33m"
C_CYAN="\033[36m"

info() { printf "  ${C_CYAN}▶ %s${C_RESET}\n" "$*" >&2; }
ok()   { printf "  ${C_GREEN}✓ %s${C_RESET}\n" "$*" >&2; }
warn() { printf "  ${C_YELLOW}⚠ %s${C_RESET}\n" "$*" >&2; }
fail() { printf "  ${C_RED}✗ %s${C_RESET}\n" "$*" >&2; }
die()  { fail "$*"; exit 1; }

RE_REV='^[0-9a-f]{40}$'
# Nix32 alphabet: digits plus letters, with e/o/u/t omitted.
RE_SHA='^[0-9abcdfghijklmnpqrsvwxyz]{52}$'

# ------------------------------------------------------------------------------
# pins.nix parsing (single source of truth: everything is read from the file)
# ------------------------------------------------------------------------------
pin_field() { # <pin-name> <field> — prints the field value
  awk -v pin="$1" -v field="$2" '
    $0 ~ "^[[:space:]]*" pin " = pin" { inblock = 1; next }
    inblock && index($0, "};") { inblock = 0; next }
    inblock {
      line = $0
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", line)
      if (index(line, field " = \"") == 1) {
        sub("^" field " = \"", "", line)
        sub("\".*$", "", line)
        print line
        exit
      }
    }
  ' "$PINS_NIX"
}

# ------------------------------------------------------------------------------
# Network + hashing helpers
# ------------------------------------------------------------------------------
resolve_head() { # <owner> <repo> <branch> — prints the branch-head commit sha
  local url="https://api.github.com/repos/$1/$2/branches/$3" json rev
  json="$(curl -sf --max-time 30 "$url")" || die "GitHub API request failed: $url"
  rev="$(printf '%s' "$json" | jq -r '.commit.sha // empty')"
  [ -n "$rev" ] || die "GitHub API returned no commit sha for $1/$2@$3 (branch missing or rate limited)"
  printf '%s' "$rev"
}

nix32_from_hex() { # <hex-sha256> — prints the Nix32 (base32) representation
  python3 - "$1" <<'PYEOF'
import sys

ALPHABET = "0123456789abcdfghijklmnpqrsvwxyz"


def encode(data: bytes) -> str:
    size = len(data)
    length = (size * 8 - 1) // 5 + 1
    out = []
    for n in range(length - 1, -1, -1):
        b = n * 5
        i = b // 8
        j = b % 8
        c = data[i] >> j
        if i < size - 1:
            c |= data[i + 1] << (8 - j)
        out.append(ALPHABET[c & 0x1F])
    return "".join(out)


print(encode(bytes.fromhex(sys.argv[1])))
PYEOF
}

prefetch() { # <url> — prints the Nix32 sha256 of the tarball at url
  local url="$1" cache_file hex
  if command -v nix-prefetch-url >/dev/null 2>&1; then
    nix-prefetch-url --type sha256 "$url" 2>/dev/null
    return
  fi
  command -v python3 >/dev/null 2>&1 || die "prefetch needs nix-prefetch-url (Nix) or python3 (fallback)"
  cache_file="$PIN_CACHE_DIR/$(printf '%s' "$url" | sed 's|^https://github.com/||; s|/archive/|-|; s|/|-|g')"
  mkdir -p "$PIN_CACHE_DIR"
  if [ ! -s "$cache_file" ]; then
    info "downloading $(basename "$cache_file") …"
    curl -sfL --max-time 900 -o "$cache_file" "$url" || die "download failed: $url"
  fi
  hex="$(sha256sum "$cache_file" | cut -d' ' -f1)"
  nix32_from_hex "$hex"
}

# ------------------------------------------------------------------------------
# Modes
# ------------------------------------------------------------------------------
verify() {
  local pin rev sha url actual failed=0
  for pin in $PINS; do
    rev="$(pin_field "$pin" rev)"
    sha="$(pin_field "$pin" sha256)"
    url="https://github.com/$(pin_field "$pin" owner)/$(pin_field "$pin" repo)/archive/${rev}.tar.gz"

    [[ "$rev" =~ $RE_REV ]] || { fail "$pin: rev is not a 40-hex commit sha"; failed=1; continue; }
    [[ "$sha" =~ $RE_SHA ]] || { fail "$pin: sha256 is not a 52-char Nix32 hash"; failed=1; continue; }

    info "verifying $pin @ ${rev:0:12}…"
    actual="$(prefetch "$url")"
    if [ "$actual" = "$sha" ]; then
      ok "$pin: hash verified"
    else
      fail "$pin: hash mismatch — pinned $sha, actual $actual"
      failed=1
    fi
  done
  [ "$failed" -eq 0 ] || die "pin verification FAILED"
  ok "all pins verified"
}

audit() {
  local failed=0 targets=() pattern
  # Every current and future nix module belongs to the audit surface.
  [ -f shell.nix ] && targets+=("shell.nix")
  for f in nix/*.nix; do
    [ -e "$f" ] && targets+=("$f")
  done
  [ "${#targets[@]}" -gt 0 ] || die "no nix files to audit"

  for pattern in '<nixpkgs>' 'builtins\.fetchGit' 'stable\.latest'; do
    if grep -rnE -- "$pattern" "${targets[@]}" >/dev/null 2>&1; then
      fail "floating reference found ($pattern):"
      grep -rnE -- "$pattern" "${targets[@]}" >&2 || true
      failed=1
    fi
  done
  [ "$failed" -eq 0 ] || die "floating-reference audit FAILED"
  ok "no floating references in ${targets[*]}"
}

update() {
  local pin owner repo branch rev head newsha changed=0
  for pin in $PINS; do
    owner="$(pin_field "$pin" owner)"
    repo="$(pin_field "$pin" repo)"
    branch="$(pin_field "$pin" branch)"
    rev="$(pin_field "$pin" rev)"

    head="$(resolve_head "$owner" "$repo" "$branch")"
    if [ "$head" = "$rev" ]; then
      ok "$pin already at $branch head ${rev:0:12}…"
      continue
    fi

    info "bumping $pin: ${rev:0:12}… → ${head:0:12}…"
    newsha="$(prefetch "https://github.com/$owner/$repo/archive/$head.tar.gz")"
    sed -i "/# pin:$pin-rev\$/{n;s|rev = \"[0123456789abcdef]*\"|rev = \"$head\"|}" "$PINS_NIX"
    sed -i "/# pin:$pin-sha256\$/{n;s|sha256 = \"[0123456789abcdfghijklmnpqrsvwxyz]*\"|sha256 = \"$newsha\"|}" "$PINS_NIX"
    ok "$pin bumped (sha256 $newsha)"
    changed=1
  done

  verify
  if [ "$changed" -eq 1 ]; then
    warn "pins.nix updated — re-run the host gates and commit:"
    warn "  git add nix/pins.nix && git commit -m 'build(nix): bump pins to branch heads'"
  else
    ok "no pin changes needed"
  fi
}

# ------------------------------------------------------------------------------
# Entry point
# ------------------------------------------------------------------------------
cd "$(dirname "$0")/.."
[ -f "$PINS_NIX" ] || die "$PINS_NIX not found — run from the repository root"

case "${1:-}" in
  --verify) verify ;;
  --audit)  audit ;;
  "")       update ;;
  *)        die "usage: $0 [--verify | --audit]" ;;
esac
