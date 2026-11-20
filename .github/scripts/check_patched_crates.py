#!/usr/bin/env python3
"""
Patched Crate Upstream Tracker

Audits every crate wired through a `[patch.<registry>]` table with a local
`path` dependency (e.g. `patches/android-activity`) against its upstream:

1. Freshness   - vendored version vs the latest stable release on crates.io.
2. Yank status - whether the vendored release was yanked upstream.
3. Integrity   - downloads the sha256-verified pristine release archive and
                 proves that the vendored tree equals pristine + the recorded
                 `local-modifications.patch`. Catches unrecorded edits and
                 stale patch records.
4. Upstream activity - unreleased commits on the upstream default branch
                 since the vendored base, including overlap with the files
                 our local patch touches.

Usage:
    python .github/scripts/check_patched_crates.py [--strict]
        [--verify-integrity] [--track-upstream] [--manifest Cargo.toml]

Exit codes:
    0  All checks passed (or produced warnings in non-strict mode).
    1  Strict mode found drift; or an integrity audit failed (integrity is
       always a hard failure: a broken local record is a repository defect).
    2  Configuration error (missing manifest, unreadable vendored crate,
       patch table name mismatch, ...).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.request
from typing import Dict, List, Optional, Sequence, Set, Tuple

if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

USER_AGENT = "SnifferEngine-CI/1.0 (contact: greenkod.dev@gmail.com)"
CRATES_IO_API = "https://crates.io/api/v1/crates"
CRATES_DL_API = "https://static.crates.io/crates"
GITHUB_API = "https://api.github.com/repos"
RECORDED_PATCH_NAME = "local-modifications.patch"
NETWORK_TIMEOUT = 15

# Files that cargo writes while unpacking a `.crate`; never part of the diff.
UNPACK_EXCLUDES = (".cargo-ok", ".cargo_vcs_info.json")


def http_get_json(url: str) -> Dict[str, object]:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=NETWORK_TIMEOUT) as resp:
        if resp.status != 200:
            raise RuntimeError(f"GET {url} returned HTTP {resp.status}")
        return json.loads(resp.read().decode("utf-8"))


def http_download(url: str, dest_path: str) -> None:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(req, timeout=NETWORK_TIMEOUT) as resp, open(
        dest_path, "wb"
    ) as out:
        if resp.status != 200:
            raise RuntimeError(f"GET {url} returned HTTP {resp.status}")
        shutil.copyfileobj(resp, out)


def read_manifest(manifest_path: str) -> str:
    try:
        with open(manifest_path, "r", encoding="utf-8") as f:
            return f.read()
    except OSError as e:
        print(f"::error::Cannot read manifest {manifest_path}: {e}")
        raise SystemExit(2)


def parse_path_patches(manifest: str) -> List[Tuple[str, str]]:
    """
    Returns (crate_name, local_path) entries from every `[patch.<registry>]`
    table using a local `path` dependency. Git patches are ignored: they
    already pin an upstream reference themselves.
    """
    patches: List[Tuple[str, str]] = []
    in_patch_table = False
    entry_re = re.compile(r'^\s*([A-Za-z0-9_-]+)\s*=\s*\{\s*path\s*=\s*"([^"]+)"')

    for line in manifest.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_patch_table = re.fullmatch(r"\[\s*patch\.[^\]]+\s*\]", stripped) is not None
            continue
        if not in_patch_table:
            continue
        m = entry_re.match(line)
        if m:
            patches.append((m.group(1), m.group(2)))

    return patches


def parse_vendored_package(vendored_manifest_path: str) -> Dict[str, str]:
    """
    Reads `name`, `version`, and `repository` from the `[package]` section of
    a vendored crate manifest. Returns empty strings for missing fields.
    """
    info: Dict[str, str] = {"name": "", "version": "", "repository": ""}
    try:
        with open(vendored_manifest_path, "r", encoding="utf-8") as f:
            content = f.read()
    except OSError:
        return info

    in_package = False
    for line in content.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_package = stripped == "[package]"
            continue
        if not in_package:
            continue
        for field in ("name", "version", "repository"):
            m = re.match(rf'^{field}\s*=\s*"([^"]+)"', stripped)
            if m and not info[field]:
                info[field] = m.group(1)

    return info


def parse_vcs_info(vendored_dir: str) -> Optional[str]:
    """Returns the upstream git sha1 recorded by cargo, if present."""
    try:
        with open(f"{vendored_dir}/.cargo_vcs_info.json", "r", encoding="utf-8") as f:
            data = json.load(f)
        return data.get("git", {}).get("sha1")
    except (OSError, ValueError):
        return None


def version_key(version: str) -> Tuple[int, ...]:
    """
    Converts a dotted numeric version into a comparable tuple. Pre-release
    suffixes are ignored so `1.0.0-beta.1` sorts as `1.0.0`.
    """
    parts: List[int] = []
    for chunk in version.split("."):
        m = re.match(r"^(\d+)", chunk)
        if not m:
            break
        parts.append(int(m.group(1)))
    return tuple(parts)


def fetch_latest_upstream(crate_name: str) -> Dict[str, str]:
    """Latest stable release, newest pre-release, and last publish date."""
    data = http_get_json(f"{CRATES_IO_API}/{crate_name}")
    crate = data.get("crate", {})
    latest = crate.get("max_stable_version") or crate.get("newest_version") or ""
    if not latest:
        raise ValueError(f"No versions reported for crate '{crate_name}'")
    return {
        "latest": str(latest),
        "newest": str(crate.get("newest_version") or latest),
        "updated_at": str(crate.get("updated_at") or ""),
    }


def fetch_version_detail(crate_name: str, version: str) -> Dict[str, object]:
    """Yank status and sha256 checksum for one exact release."""
    data = http_get_json(f"{CRATES_IO_API}/{crate_name}/{version}")
    detail = data.get("version", {})
    return {
        "yanked": bool(detail.get("yanked", False)),
        "checksum": str(detail.get("checksum") or ""),
        "created_at": str(detail.get("created_at") or ""),
    }


def parse_github_repo(repository: str) -> Optional[Tuple[str, str]]:
    m = re.match(r"^https://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/?$", repository)
    if not m:
        return None
    return (m.group(1), m.group(2))


def fetch_upstream_compare(
    owner: str, repo: str, base_sha: str
) -> Optional[Dict[str, object]]:
    """
    Commits on the upstream default branch since the vendored base release.
    Returns None when the upstream repository is unknown or unreachable.
    """
    url = f"{GITHUB_API}/{owner}/{repo}/compare/{base_sha}...HEAD"
    try:
        data = http_get_json(url)
    except (urllib.error.URLError, RuntimeError, ValueError):
        return None
    if not isinstance(data, dict) or "ahead_by" not in data:
        return None
    return data


def normalize_patch(patch_text: str) -> Tuple[List[str], List[str], Set[str]]:
    """
    Normalizes a unified diff for stable comparison:

    - drops `diff ...` separator lines and `Only in ...` inventory lines
      (the latter are returned separately as omission entries),
    - rewrites directory prefixes to the canonical `a/` and `b/` forms,
    - strips header timestamps.

    Returns (normalized_lines, omission_lines, changed_paths).
    """
    normalized: List[str] = []
    omissions: List[str] = []
    changed: Set[str] = set()

    for raw in patch_text.splitlines():
        line = raw.rstrip("\r")
        if line.startswith("diff "):
            continue
        if line.startswith("Only in "):
            omissions.append(line)
            continue

        # Canonical headers (already in `a/`-`b/` form, possibly with a
        # tab-separated timestamp as produced by `diff`).
        m = re.match(r"^--- a/(.+?)\t", line) or re.match(r"^--- a/(.+)$", line)
        if m:
            changed.add(m.group(1))
            normalized.append(f"--- a/{m.group(1)}")
            continue
        m = re.match(r"^\+\+\+ b/(.+?)\t", line) or re.match(r"^\+\+\+ b/(.+)$", line)
        if m:
            normalized.append(f"+++ b/{m.group(1)}")
            continue

        # Raw headers pointing at same-named pristine/vendored trees.
        m = re.match(r"^--- (?:\S*?/)?android-activity/(.+?)\t", line)
        if m:
            changed.add(m.group(1))
            normalized.append(f"--- a/{m.group(1)}")
            continue
        m = re.match(r"^\+\+\+ (?:\.\./)?(?:p\d+/)?android-activity/(.+?)\t", line)
        if m:
            normalized.append(f"+++ b/{m.group(1)}")
            continue

        # Any other header: keep it, drop the timestamp.
        if line.startswith(("--- ", "+++ ")):
            normalized.append(re.split(r"\t", line)[0])
            continue
        normalized.append(line)

    while normalized and not normalized[-1].strip():
        normalized.pop()

    return normalized, omissions, changed


def patch_excludes(patch_text: str) -> List[str]:
    """Recovers the `--exclude` patterns used to record a patch file."""
    first_diff = next((l for l in patch_text.splitlines() if l.startswith("diff ")), "")
    return re.findall(r"'--exclude=([^']+)'", first_diff)


def safe_extract_crate(archive_path: str, dest_dir: str, expected_root: str) -> None:
    """Extracts a `.crate` tarball, refusing paths outside its root."""
    with tarfile.open(archive_path, "r:gz") as tar:
        for member in tar.getmembers():
            name = member.name.replace("\\", "/")
            if name.startswith("/") or ".." in name.split("/"):
                raise RuntimeError(f"Unsafe tar member refused: {member.name}")
            if not (name == expected_root or name.startswith(f"{expected_root}/")):
                raise RuntimeError(f"Unexpected tar member refused: {member.name}")
        try:
            tar.extractall(dest_dir, filter="data")  # Python 3.12+
        except TypeError:
            tar.extractall(dest_dir)


def run_candidate_diff(
    parent_dir: str, crate_dirname: str, excludes: Sequence[str]
) -> str:
    """
    Reproduces the unified diff between the pristine release tree at
    `parent_dir/p1/<crate_dirname>` and the vendored tree at
    `parent_dir/p2/<crate_dirname>`, using the same excludes that were used
    to record the patch file so the outputs are directly comparable.
    """
    cmd = ["diff", "-ru"]
    cmd += [f"--exclude={pattern}" for pattern in excludes]
    cmd += [f"p1/{crate_dirname}", f"p2/{crate_dirname}"]
    result = subprocess.run(cmd, cwd=parent_dir, capture_output=True, text=True, check=False)
    if result.returncode not in (0, 1):
        raise RuntimeError(f"diff exited with {result.returncode}: {result.stderr.strip()}")
    return result.stdout


def verify_integrity(
    declared_name: str,
    vendored_version: str,
    vendored_dir: str,
    checksum: str,
    recorded_patch_path: str,
) -> Tuple[str, List[str]]:
    """
    Proves vendored tree == pristine release + recorded patch.

    Returns (status, findings) where status is "ok", "error", or "skipped".
    Integrity failures are always errors: a broken local record means the
    documented update procedure is no longer reproducible.
    """
    findings: List[str] = []

    try:
        with open(recorded_patch_path, "r", encoding="utf-8") as f:
            recorded_text = f.read()
    except OSError:
        return (
            "error",
            [
                f"No recorded '{RECORDED_PATCH_NAME}' for patched crate "
                f"'{declared_name}'. Record the local modifications so future "
                f"updates stay reproducible (see patches/README.md)."
            ],
        )

    recorded_lines, _, recorded_changed = normalize_patch(recorded_text)
    excludes = list(
        dict.fromkeys(patch_excludes(recorded_text) + list(UNPACK_EXCLUDES))
    )

    expected_root = f"{declared_name}-{vendored_version}"
    with tempfile.TemporaryDirectory(prefix="patched-crate-audit-") as tmp:
        archive_path = f"{tmp}/{expected_root}.crate"
        pristine_root = f"{tmp}/p1/{declared_name}"
        vendored_root = f"{tmp}/p2/{declared_name}"

        try:
            http_download(
                f"{CRATES_DL_API}/{declared_name}/{expected_root}.crate", archive_path
            )
        except (urllib.error.URLError, RuntimeError) as e:
            return (
                "error",
                [f"Could not download pristine {declared_name} {vendored_version}: {e}"],
            )

        digest = hashlib.sha256()
        with open(archive_path, "rb") as f:
            for chunk in iter(lambda: f.read(65536), b""):
                digest.update(chunk)
        if checksum and digest.hexdigest() != checksum:
            return (
                "error",
                [
                    f"sha256 mismatch for pristine {declared_name} "
                    f"{vendored_version} archive; refusing to audit against it."
                ],
            )

        try:
            safe_extract_crate(archive_path, f"{tmp}/p1", expected_root)
        except (RuntimeError, tarfile.TarError) as e:
            return ("error", [f"Could not unpack pristine archive: {e}"])

        shutil.move(f"{tmp}/p1/{expected_root}", pristine_root)
        # The recorded patch itself lives inside the vendored directory; it is
        # repository bookkeeping, not part of the crate, so it stays out.
        shutil.copytree(
            vendored_dir,
            vendored_root,
            ignore=shutil.ignore_patterns(RECORDED_PATCH_NAME),
        )

        try:
            candidate_text = run_candidate_diff(tmp, declared_name, excludes)
        except RuntimeError as e:
            return ("error", [f"Integrity diff failed for {declared_name}: {e}"])

        candidate_lines, omissions, candidate_changed = normalize_patch(candidate_text)

        if omissions:
            findings.append(
                "Vendored tree and pristine release disagree about these files: "
                + "; ".join(omissions[:5])
                + (" ..." if len(omissions) > 5 else "")
            )
        if candidate_lines != recorded_lines:
            changed_files = sorted(
                (candidate_changed ^ recorded_changed)
                or (candidate_changed | recorded_changed)
            )
            findings.append(
                f"Recorded patch is out of sync with the vendored tree of "
                f"'{declared_name}'; files in play: "
                + (", ".join(changed_files[:8]) or "unknown")
                + ". Re-run the regeneration procedure in patches/README.md."
            )
        if not findings:
            return (
                "ok",
                [f"vendored tree == pristine {vendored_version} + recorded patch"],
            )
        return ("error", findings)


def render_step_summary(rows: List[Dict[str, str]]) -> None:
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if not summary_path or not rows:
        return

    lines = [
        "",
        "### Patched Crate Tracker",
        "",
        "| Crate | Vendored | Upstream latest | Yanked | Integrity | Unreleased upstream commits |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for row in rows:
        lines.append(
            f"| {row['name']} | {row['vendored']} | {row['latest']} "
            f"| {row['yanked']} | {row['integrity']} | {row['upstream']} |"
        )
    try:
        with open(summary_path, "a", encoding="utf-8") as f:
            f.write("\n".join(lines) + "\n")
    except OSError:
        pass


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Audit patched crates against their upstream releases."
    )
    parser.add_argument(
        "--manifest",
        default="Cargo.toml",
        help="Workspace manifest holding the [patch.crates-io] table (default: Cargo.toml)",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Fail when a patched crate is behind the latest upstream release",
    )
    parser.add_argument(
        "--verify-integrity",
        action="store_true",
        help=(
            "Download the pristine release and prove the vendored tree matches "
            "pristine + the recorded local-modifications.patch (hard failure)"
        ),
    )
    parser.add_argument(
        "--track-upstream",
        action="store_true",
        help="Report unreleased commits on the upstream default branch",
    )
    args = parser.parse_args()

    manifest = read_manifest(args.manifest)
    patches = parse_path_patches(manifest)

    if not patches:
        print("[OK] No local `path` patches found in the workspace manifest.")
        return 0

    print(f"[INFO] Auditing {len(patches)} locally patched crate(s):")

    behind: List[Tuple[str, str, str]] = []
    yanked_crates: List[Tuple[str, str]] = []
    integrity_failures: List[str] = []
    lookup_failures: List[str] = []
    config_errors: List[str] = []
    upstream_alerts: List[str] = []
    summary_rows: List[Dict[str, str]] = []

    for declared_name, local_path in patches:
        vendored = parse_vendored_package(f"{local_path}/Cargo.toml")
        vendored_name, vendored_version = vendored["name"], vendored["version"]

        if not vendored_version:
            config_errors.append(declared_name)
            print(
                f"::error::Patched crate '{declared_name}' at '{local_path}' has "
                f"no readable Cargo.toml with a [package] version."
            )
            continue

        # The patch table key must match the vendored package name, otherwise
        # the patch silently stops applying and cargo falls back to crates.io.
        if vendored_name and vendored_name != declared_name:
            config_errors.append(declared_name)
            print(
                f"::error::Patch table declares '{declared_name}' but "
                f"'{local_path}' packages '{vendored_name}'; the patch is not "
                f"being applied by cargo."
            )
            continue

        vcs_sha = parse_vcs_info(local_path)
        sha_suffix = f" @ {vcs_sha[:10]}" if vcs_sha else ""

        try:
            upstream = fetch_latest_upstream(declared_name)
        except Exception as e:
            print(f"::warning::Upstream lookup failed for '{declared_name}': {e}")
            lookup_failures.append(declared_name)
            continue

        try:
            detail = fetch_version_detail(declared_name, vendored_version)
        except Exception as e:
            detail = {"yanked": False, "checksum": "", "created_at": ""}
            print(f"::warning::Release detail lookup failed for '{declared_name}': {e}")

        published = upstream["updated_at"][:10] or "?"

        # --- 1. Freshness ----------------------------------------------
        if version_key(vendored_version) >= version_key(upstream["latest"]):
            print(
                f"   [OK] {declared_name} {vendored_version}{sha_suffix} "
                f"== upstream {upstream['latest']} (published {published})"
            )
        else:
            message = (
                f"Patched crate '{declared_name}' is {vendored_version} but "
                f"upstream latest is {upstream['latest']} (published {published}). "
                f"Rebase the local patches onto the new release; see patches/README.md."
            )
            print(f"   [BEHIND] {declared_name} {vendored_version} -> {upstream['latest']}")
            print(f"{'::error::' if args.strict else '::warning::'}{message}")
            behind.append((declared_name, vendored_version, upstream["latest"]))

        pre_release_note = ""
        if version_key(upstream["newest"]) > version_key(upstream["latest"]):
            pre_release_note = f" (pre-release {upstream['newest']} visible upstream)"
            print(
                f"   [INFO] {declared_name}: upstream pre-release "
                f"{upstream['newest']} is not counted as stable latest."
            )

        # --- 2. Yank status --------------------------------------------
        yanked = bool(detail["yanked"])
        if yanked:
            message = (
                f"Release {declared_name} {vendored_version} was yanked upstream; "
                f"move to a healthy release."
            )
            print(f"   [YANKED] {declared_name} {vendored_version}")
            print(f"{'::error::' if args.strict else '::warning::'}{message}")
            yanked_crates.append((declared_name, vendored_version))

        # --- 3. Integrity ----------------------------------------------
        integrity_status = "n/a"
        if args.verify_integrity:
            status, findings = verify_integrity(
                declared_name,
                vendored_version,
                local_path,
                str(detail["checksum"]),
                f"{local_path}/{RECORDED_PATCH_NAME}",
            )
            integrity_status = status
            for finding in findings:
                print(f"   [{status.upper()}] {declared_name}: {finding}")
                if status == "error":
                    print(f"::error::{declared_name}: {finding}")
            if status == "error":
                integrity_failures.append(declared_name)

        # --- 4. Upstream activity --------------------------------------
        upstream_note = "not checked"
        if args.track_upstream:
            upstream_note = "unknown"
            repo_coords = parse_github_repo(vendored["repository"])
            if not repo_coords:
                print(
                    f"   [INFO] {declared_name}: no github.com repository in the "
                    f"vendored manifest; skipping unreleased-commit tracking."
                )
                upstream_note = "no repository"
            elif not vcs_sha:
                print(
                    f"   [INFO] {declared_name}: no .cargo_vcs_info.json sha; "
                    f"skipping unreleased-commit tracking."
                )
                upstream_note = "no base sha"
            else:
                compare = fetch_upstream_compare(repo_coords[0], repo_coords[1], vcs_sha)
                if compare is None:
                    print(
                        f"   [INFO] {declared_name}: upstream compare unavailable; "
                        f"skipping unreleased-commit tracking."
                    )
                else:
                    ahead_by = int(compare.get("ahead_by", 0))
                    compare_files = {
                        str(f.get("filename"))
                        for f in (compare.get("files") or [])
                        if isinstance(f, dict)
                    }
                    if ahead_by == 0:
                        upstream_note = "0 unreleased commits"
                        print(
                            f"   [OK] {declared_name}: upstream default branch has "
                            f"no commits beyond the vendored base release."
                        )
                    else:
                        upstream_note = f"{ahead_by} unreleased commits"
                        print(
                            f"   [INFO] {declared_name}: {ahead_by} unreleased "
                            f"commits on the upstream default branch."
                        )
                        if compare_files:
                            try:
                                with open(
                                    f"{local_path}/{RECORDED_PATCH_NAME}",
                                    "r",
                                    encoding="utf-8",
                                ) as f:
                                    _, _, recorded_changed = normalize_patch(f.read())
                            except OSError:
                                recorded_changed = set()
                            overlap = sorted(compare_files & recorded_changed)
                            if overlap:
                                upstream_note += ", touching patched files"
                                message = (
                                    f"Unreleased upstream commits for "
                                    f"'{declared_name}' touch locally patched "
                                    f"files: {', '.join(overlap)}. Expect a "
                                    f"rebase conflict on the next release."
                                )
                                print(f"   [WATCH] {declared_name}: {message}")
                                print(f"::warning::{message}")
                                upstream_alerts.append(f"{declared_name}: {message}")

        summary_rows.append(
            {
                "name": declared_name,
                "vendored": vendored_version + sha_suffix,
                "latest": upstream["latest"] + pre_release_note,
                "yanked": "yes" if yanked else "no",
                "integrity": integrity_status,
                "upstream": upstream_note,
            }
        )

    render_step_summary(summary_rows)
    print()

    # --- Verdict --------------------------------------------------------
    exit_code = 0
    if config_errors:
        print("[FAIL] Configuration errors for: " + ", ".join(sorted(set(config_errors))))
        exit_code = 2
    if integrity_failures:
        print(
            "[FAIL] Integrity audit failed for "
            + ", ".join(sorted(set(integrity_failures)))
            + ". The recorded patches no longer describe the vendored trees; "
            f"regenerate them per patches/README.md."
        )
        exit_code = max(exit_code, 1)
    if yanked_crates:
        names = ", ".join(f"{n} {v}" for n, v in yanked_crates)
        if args.strict:
            print(f"[FAIL] Yanked upstream releases in use: {names}")
            exit_code = max(exit_code, 1)
        else:
            print(f"[WARN] Yanked upstream releases in use: {names}")
    if behind:
        print(
            "[INFO] Patched crates behind upstream: "
            + ", ".join(f"{n} {v} -> {l}" for n, v, l in behind)
        )
        if args.strict:
            print("[FAIL] Patched crates are out of date. Update them per patches/README.md.")
            exit_code = max(exit_code, 1)
        else:
            print("[WARN] Not failing (non-strict mode). Run with --strict in CI to enforce.")
    if lookup_failures and args.strict:
        print(
            "[FAIL] Upstream lookups failed for: "
            + ", ".join(sorted(set(lookup_failures)))
            + ". Retry later; the freshness tracker refuses to stay silent."
        )
        exit_code = max(exit_code, 1)
    if upstream_alerts:
        print("[WATCH] Upstream unreleased work touches patched files:")
        for alert in upstream_alerts:
            print(f"  - {alert}")

    if exit_code == 0 and summary_rows:
        print("[OK] All patched crates track the latest upstream release cleanly.")
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
