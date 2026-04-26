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

The publish job intentionally waits for every CLI and desktop matrix leg. A
failed macOS, Linux, or Windows leg blocks the prerelease instead of shipping a
partial artifact set. Re-run the failed workflow/job after fixing runner or
packaging failures; do not create a partial release unless the user explicitly
asks for one.

## Artifact Set

The workflow currently builds:

- CLI: macOS arm64, macOS x86_64, and Linux x86_64 one-file PyInstaller
  binaries as `kassiber-cli-<target>.tar.gz` archives, each with a `.sha256`
  file. The extracted executable is still named `kassiber`. Linux is built on
  Ubuntu 22.04 to keep the glibc floor aligned with the AppImage build.
- Desktop previews: a universal macOS `.app` zip plus `.dmg`, Linux
  `.AppImage`, and Windows `.msi` plus NSIS setup `.exe`, each emitted with a
  `kassiber-desktop-<target>-...` filename and a `.sha256` file. Each desktop
  preview includes a bundled one-file Kassiber CLI sidecar for its target
  platform; the universal macOS app bundles both arm64 and x86_64 CLI sidecars.

The macOS CLI legs stay architecture-specific because PyInstaller universal2
requires a universal2 Python interpreter or extra binary stitching. The macOS
desktop leg uses GitHub's `macos-latest` runner with Tauri's
`--target universal-apple-darwin`, after installing both Rust targets
(`aarch64-apple-darwin` and `x86_64-apple-darwin`). Keep the desktop artifact
target name `macos-universal` so users only see one Mac GUI download.

Desktop artifacts are unsigned previews, but normal daemon calls use the
bundled PyInstaller CLI sidecar and do not require a separate Python checkout.
The GUI executable also forwards `--cli ...` to that sidecar so an installed
desktop app can still be used from a terminal, for example:

```bash
Kassiber.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

`KASSIBER_DAEMON_PYTHON` remains available as an intentional debug override.

There is no standalone Windows CLI artifact yet. Windows coverage is
desktop-preview only, and the installed desktop executable can forward
`--cli ...` to the bundled CLI sidecar.

## Commit Identity

Every GitHub Actions run records the source ref and commit SHA. For tag
prereleases, the tag itself is the source-of-truth link back to the commit.

The desktop shell displays the build commit beside the version number. CLI
binaries, artifact filenames, and `.sha256` sidecars do not currently include
or embed the source commit hash.

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
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin --bundles app,dmg --ci
```
