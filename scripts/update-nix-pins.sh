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
#   Modern Nix fetchTarball verifies the NAR hash of the UNPACKED tree,
#   not the archive file hash — proven by the first CI run of the
#   validation workflow, where file hashes were rejected with a NAR hash
#   mismatch. This script therefore downloads the tarball, unpacks it,
#   and serializes a faithful NAR (upstream: src/libutil/archive.cc +
#   archive.hh, validated against a runner-reported NAR hash) to produce
#   exactly the value pins.nix needs. Requires python3; downloads are
#   cached in $PIN_CACHE_DIR and reused.
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

# Computes the NAR hash of the unpacked tarball tree — the only hash
# semantics modern Nix fetchTarball accepts (see the header comment).
# Faithful reimplementation of Nix's wire format, validated against a
# runner-reported NAR hash for the nixpkgs pin.
prefetch() { # <url> — prints the Nix32 NAR sha256 of the tarball at url
  local url="$1" cache_file
  command -v python3 >/dev/null 2>&1 || die "prefetch needs python3"
  cache_file="$PIN_CACHE_DIR/$(printf '%s' "$url" | sed 's|^https://github.com/||; s|/archive/|-|; s|/|-|g')"
  mkdir -p "$PIN_CACHE_DIR"
  if [ ! -s "$cache_file" ]; then
    info "downloading $(basename "$cache_file") …"
    curl -sfL --max-time 900 -o "$cache_file" "$url" || die "download failed: $url"
  fi
  info "serializing NAR of $(basename "$cache_file") …"
  python3 - "$cache_file" <<'PYEOF'
import base64
import hashlib
import os
import stat
import struct
import sys
import tarfile
import tempfile

NIX32 = "0123456789abcdfghijklmnpqrsvwxyz"


def nix32(data: bytes) -> str:
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
        out.append(NIX32[c & 0x1F])
    return "".join(out)


def pad(n: int) -> int:
    return (-n) % 8


class NarSink:
    def __init__(self) -> None:
        self.h = hashlib.sha256()

    def out(self, data: bytes) -> None:
        self.h.update(data)

    def wint(self, i: int) -> None:
        self.out(struct.pack("<q", i))

    def wstr(self, s: bytes) -> None:
        self.wint(len(s))
        self.out(s)
        self.out(b"\x00" * pad(len(s)))


def dump_node(sink: NarSink, path: str) -> None:
    st = os.lstat(path)
    sink.wstr(b"(")
    if stat.S_ISDIR(st.st_mode):
        sink.wstr(b"type")
        sink.wstr(b"directory")
        for child in sorted(os.listdir(path), key=lambda c: c.encode()):
            sink.wstr(b"entry")
            sink.wstr(b"(")
            sink.wstr(b"name")
            sink.wstr(child.encode())
            sink.wstr(b"node")
            dump_node(sink, os.path.join(path, child))
            sink.wstr(b")")
        sink.wstr(b")")
    elif stat.S_ISREG(st.st_mode):
        sink.wstr(b"type")
        sink.wstr(b"regular")
        if st.st_mode & 0o111:
            sink.wstr(b"executable")
            sink.wstr(b"")
        sink.wstr(b"contents")
        with open(path, "rb") as f:
            data = f.read()
        size = len(data)
        sink.wint(size)
        sink.out(data)
        sink.out(b"\x00" * pad(size))
        sink.wstr(b")")
    elif stat.S_ISLNK(st.st_mode):
        sink.wstr(b"type")
        sink.wstr(b"symlink")
        sink.wstr(b"target")
        sink.wstr(os.readlink(path).encode())
        sink.wstr(b")")
    else:
        raise RuntimeError(f"unsupported file type: {path}")


def nar_hash_of_tarball(tarball: str) -> bytes:
    with tempfile.TemporaryDirectory(prefix="sniffers-nar-") as tmp:
        with tarfile.open(tarball, "r:gz") as tf:
            tf.extractall(tmp, filter="data")
        entries = os.listdir(tmp)
        if len(entries) != 1:
            raise RuntimeError(f"expected a single top-level directory, got {entries}")
        sink = NarSink()
        sink.wstr(b"nix-archive-1")
        dump_node(sink, os.path.join(tmp, entries[0]))
        return sink.h.digest()


print(nix32(nar_hash_of_tarball(sys.argv[1])))
PYEOF
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
  [ -f shell-android.nix ] && targets+=("shell-android.nix")
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
