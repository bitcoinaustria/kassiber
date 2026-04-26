# Prerelease Binaries

Kassiber is still in early development, so binary builds are deliberately
opt-in except for version-tag prereleases. Keep normal PR feedback cheap and
use packaged artifacts only when a human asks for them.

## What Runs Automatically

- Pull requests run `.github/workflows/ci.yml` only. They do not build CLI or
  desktop binaries.
- Pushes to `main` run `.github/workflows/ci.yml` only.
- Pushes of tags matching `v*` run `.github/workflows/prerelease-binaries.yml`
  and publish the resulting artifacts to a GitHub prerelease.

Do not add a `pull_request` trigger to `prerelease-binaries.yml` unless the
user explicitly asks for binary builds on every PR. For one-off PR or branch
artifacts, use a manual workflow run instead.

## Manual Runs

For a tester build from a branch or PR branch, run the workflow manually against
that ref and do not publish a release:

```bash
gh workflow run prerelease-binaries.yml \
  --repo bitcoinaustria/kassiber \
  --ref <branch-name> \
  -f publish_release=false
```

Then find and download the artifacts from the workflow run:

```bash
gh run list \
  --repo bitcoinaustria/kassiber \
  --workflow prerelease-binaries.yml \
  --branch <branch-name> \
  --limit 1

gh run download <run-id> \
  --repo bitcoinaustria/kassiber \
  --dir /tmp/kassiber-prerelease-artifacts
```

For a prerelease attached to an existing tag, first verify the tag points at
the intended commit:

```bash
git fetch --tags origin
git show --no-patch --oneline <tag-name>
```

Then either push a new `v*` tag, which publishes automatically, or manually run
the workflow with publishing enabled for an existing tag:

```bash
gh workflow run prerelease-binaries.yml \
  --repo bitcoinaustria/kassiber \
  --ref <tag-name> \
  -f publish_release=true \
  -f tag_name=<tag-name>
```

Only use `publish_release=true` for real prerelease tags. PR and branch tester
builds should stay workflow artifacts, not GitHub Releases.

## Artifact Set

The workflow currently builds:

- CLI: macOS and Linux one-file PyInstaller binaries as `.tar.gz` archives,
  each with a `.sha256` file. Linux is built on Ubuntu 22.04 to keep the glibc
  floor aligned with the AppImage build.
- Desktop previews: macOS `.app` zip plus `.dmg`, Linux `.AppImage`, and
  Windows `.msi` plus NSIS setup `.exe`, each with a `.sha256` file.

Desktop artifacts are unsigned previews and do not yet bundle the Python
sidecar. Real daemon calls require a machine where `python3 -m kassiber daemon`
works, or a launch environment that sets `KASSIBER_DAEMON_PYTHON` and
`KASSIBER_REPO_ROOT`.

There is no Windows CLI binary yet. Windows coverage is desktop-preview only.

## Commit Identity

Every GitHub Actions run records the source ref and commit SHA. For tag
prereleases, the tag itself is the source-of-truth link back to the commit.

The artifact filenames and `.sha256` sidecars do not currently include or
embed the source commit hash. Do not claim an artifact contains the latest
commit hash unless the workflow has been changed to add build metadata.

The intended next hardening step is to include a small `BUILD_INFO.json` or
`BUILD_INFO.txt` in every packaged artifact with at least:

```json
{
  "commit": "${GITHUB_SHA}",
  "ref": "${GITHUB_REF_NAME}",
  "run_id": "${GITHUB_RUN_ID}",
  "built_at": "<UTC timestamp>"
}
```

## Verification When Editing This Workflow

Before pushing changes to prerelease packaging, run:

```bash
ruby -e 'require "psych"; Psych.load_file(".github/workflows/prerelease-binaries.yml")'
git diff --check
./scripts/quality-gate.sh
```

If desktop packaging changes, also run the relevant local Tauri build when the
host OS supports it, for example on macOS:

```bash
cd ui-tauri
pnpm install --frozen-lockfile
pnpm tauri build --bundles app,dmg --ci
```
