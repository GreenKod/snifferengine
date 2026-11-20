# Locally Patched Crates

This directory vendors crates that carry local modifications on top of an
official crates.io release. Each patched crate is wired into the workspace
through the `[patch.crates-io]` table in the root `Cargo.toml`, so cargo
resolves the local copy instead of the registry copy.

## Patched Crate Registry

| Crate            | Vendored version | Upstream release | Base commit                            | Why it is patched                                                        |
| ---------------- | ---------------- | ---------------- | -------------------------------------- | ------------------------------------------------------------------------ |
| `android-activity` | `0.6.1`          | `0.6.1`          | `b4ddf059b77be12cdb955d394e65a92e7568d936` | Wallpaper-visible transparent windows and crash-resilient JNI bootstrap |

The base commit is the upstream git SHA recorded in
`.cargo_vcs_info.json` of the unpacked release archive.

## Why `android-activity` Is Patched

Local modifications are preserved as a git-apply-able diff in
[`android-activity/local-modifications.patch`](android-activity/local-modifications.patch).
They currently change three files:

- **`src/native_activity/mod.rs`** — call
  `ANativeActivity_setWindowFormat(..., WINDOW_FORMAT_RGBA_8888)` when window
  flags are updated, so the launcher can show the system wallpaper through a
  transparent `NativeActivity` window.
- **`src/init.rs`** — replace the high-level `jni` crate helpers with raw JNI
  calls for `getApplication`, `getAssets`, and the `android_main` thread
  bootstrap. Lookups degrade gracefully instead of aborting when a JNI method
  is unavailable on exotic OEM ROMs.
- **`src/sdk.rs`** — disable the eager `jni_init` API preload that the raw-JNI
  bootstrap above makes redundant.

Additionally, the vendored copy **omits the upstream `android-games-sdk/`
directory** (only needed for the `game-activity` backend, which this engine
does not use) to keep the repository lean. Re-delete it after every update.

## Upstream Tracking

The tracker compares every patched crate against its upstream from four
angles. A scheduled GitHub Actions workflow
([`.github/workflows/patch-freshness.yml`](../.github/workflows/patch-freshness.yml))
runs the full audit weekly and keeps a labeled GitHub issue open until the
drift is resolved, so an upstream update is never missed.

1. **Freshness** — vendored version vs the latest stable release on crates.io
   (pre-releases are reported but never treated as latest).
2. **Yank status** — whether the vendored release was yanked upstream.
3. **Integrity** — downloads the sha256-verified pristine release archive and
   proves `vendored tree == pristine + local-modifications.patch`. Catches
   unrecorded edits and stale patch records. Always a hard failure: a broken
   local record means the update procedure is no longer reproducible.
4. **Upstream activity** — unreleased commits on the upstream default branch
   since the vendored base release, with a warning when they touch files that
   the local patch also modifies (an early rebase-conflict heads-up).

Workflow behavior:

- **Weekly (strict):** any drift fails the job; a `patch-freshness` labeled
  issue is opened (or commented on) and stays open as a visible reminder. It
  is closed automatically once the audit is green again.
- **Push to `master` (non-strict):** drift is reported as annotations only;
  integrity failures still fail the run.
- **Manual dispatch:** runs the full strict audit on demand.

Local commands:

```bash
# Freshness only, warnings on drift (also part of scripts/check.sh step 8/8)
python3 .github/scripts/check_patched_crates.py

# Full audit: freshness + yank + integrity + upstream activity, strict
python3 .github/scripts/check_patched_crates.py --strict --verify-integrity --track-upstream
```

Exit codes: `0` clean (or warnings in non-strict mode), `1` strict-mode drift
or an integrity failure, `2` configuration error (unreadable vendored crate,
patch table name mismatch, ...).

## Update Procedure

When the tracker reports a newer upstream release:

1. **Quarantine:** per the repository supply-chain policy, wait at least 24
   hours after the release was published before pulling it in.
2. **Replace the vendored copy** with the new release archive:

   ```bash
   VERSION=<new-version>
   curl -sL "https://static.crates.io/crates/android-activity/android-activity-${VERSION}.crate" -o /tmp/android-activity.crate
   rm -rf patches/android-activity
   mkdir -p /tmp/android-activity-new && tar xzf /tmp/android-activity.crate -C /tmp/android-activity-new
   mv /tmp/android-activity-new/android-activity-${VERSION} patches/android-activity
   rm -rf patches/android-activity/android-games-sdk
   ```

3. **Re-apply the local modifications:**

   ```bash
   cd patches/android-activity
   git apply local-modifications.patch
   ```

   Resolve conflicts manually if the patch no longer applies cleanly, then
   regenerate the patch file (see below) so it matches the final result.
4. **Verify:** `cargo check --locked --workspace` plus `bash scripts/check.sh`,
   and confirm the `.cargo_vcs_info.json` sha and version in this README's
   registry table match the release you unpacked.
5. **Commit** the refreshed crate together with the updated registry table.

## Regenerating the Patch File

After editing the vendored crate, refresh the tracked diff (paths must stay
relative to the crate root so `git apply` works from inside it):

```bash
PRISTINE=/tmp/android-activity-new/android-activity-${VERSION}   # freshly unpacked, unmodified
cd "$(dirname "$PRISTINE")"
rm -rf p1 p2 && mkdir p1 p2
cp -r "$(basename "$PRISTINE")" p1/android-activity
cp -r /path/to/snifferengine/patches/android-activity p2/android-activity
(cd p1 && diff -ru \
  --exclude='android-games-sdk' --exclude='.cargo-ok' --exclude='.cargo_vcs_info.json' \
  android-activity ../p2/android-activity \
  | sed -e 's|^--- android-activity/|--- a/|' -e 's|^+++ ../p2/android-activity/|+++ b/|') \
  > /path/to/snifferengine/patches/android-activity/local-modifications.patch
```

The regeneration is also handy as a **drift audit**: the tracked patch should
contain exactly the intended local modifications, nothing more.
