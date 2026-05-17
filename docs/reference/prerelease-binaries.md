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
  binaries as `kassiber-cli-<platform>-<arch>.tar.gz` archives. The extracted
  executable is still named `kassiber`. Linux is built on Ubuntu 22.04 to keep
  the glibc floor aligned with the AppImage build.
- Desktop previews: a universal macOS `.app` zip plus `.dmg`, Linux
  `.AppImage`, and Windows `.msi` plus NSIS setup `.exe`, published with short
  user-facing filenames. Each desktop preview includes a bundled one-file
  Kassiber CLI sidecar for its target platform; the universal macOS app bundles
  both arm64 and x86_64 CLI sidecars.

Public release filenames intentionally omit the release version because the
GitHub release tag already supplies it:

```text
kassiber-cli-linux-x64.tar.gz
kassiber-cli-macos-arm64.tar.gz
kassiber-cli-macos-x64.tar.gz
kassiber-linux-x64.AppImage
kassiber-macos-universal.app.zip
kassiber-macos-universal.dmg
kassiber-windows-x64.exe
kassiber-windows-x64.msi
SHA256SUMS.txt
```

Use `x64` for public filenames instead of `x86_64`. Bundled sidecar resource
filenames are internal to the desktop package and use Rust target triples such
as `kassiber-cli-aarch64-apple-darwin`; those raw sidecars are not release
assets.

The macOS CLI legs stay architecture-specific because PyInstaller universal2
requires a universal2 Python interpreter or extra binary stitching. The macOS
desktop leg uses GitHub's `macos-latest` runner with Tauri's
`--target universal-apple-darwin`, after installing both Rust targets
(`aarch64-apple-darwin` and `x86_64-apple-darwin`). Keep the desktop artifact
target name `macos-universal` so users only see one Mac GUI download.

### Local Apple Silicon build

For a local Apple Silicon build without Rosetta, use the arm64-only helper:

```bash
./scripts/build-macos-arm64-app.sh
# skip the DMG if you only want the .app:
BUNDLES=app ./scripts/build-macos-arm64-app.sh
```

It builds the PyInstaller sidecar as
`ui-tauri/src-tauri/binaries/kassiber-cli-aarch64-apple-darwin`, verifies that
the executable is arm64, and runs Tauri with
`--target aarch64-apple-darwin --bundles app,dmg`. The result is a full
unsigned desktop app and DMG under
`ui-tauri/src-tauri/target/aarch64-apple-darwin/release/bundle`.
The helper defaults to Python 3.11 to match the GitHub Actions prerelease
workflow; set `PYTHON_VERSION=<version>` only for intentional local debugging.

#### First launch on macOS

Tauri ad-hoc signs the binary (required for arm64 to launch at all), but
ad-hoc is not a Developer ID, so Gatekeeper will challenge the first launch.
The good news for a local build: nothing downloaded it, so there is no
`com.apple.quarantine` xattr — Gatekeeper is on its softer path.

```bash
open ui-tauri/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Kassiber.app
```

Expected dialogs and how to clear them:

- **"Kassiber.app" cannot be opened because the developer cannot be
  verified.** — the typical local-build case. Either:
  - Finder → right-click the .app → **Open** → **Open** to record a
    one-time override (macOS remembers the path), or
  - System Settings → Privacy & Security → scroll to the blocked-app
    notice → **Open Anyway**.
- **"Kassiber.app" is damaged and can't be opened.** — only if a quarantine
  xattr was attached (e.g. you zipped/unzipped the bundle through a tool
  that adds the flag). Strip it:
  ```bash
  xattr -dr com.apple.quarantine \
    ui-tauri/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Kassiber.app
  ```

Subsequent launches from the same path go through silently. If you rebuild
over the same path, macOS usually keeps the override; if it doesn't, repeat
the right-click → Open dance once.

For day-to-day frontend iteration, skip the .app bundle entirely and use
`pnpm --dir ui-tauri tauri:dev` — that runs the webview + Rust supervisor +
Python daemon with HMR, no PyInstaller step, no DMG, no Gatekeeper dialog.
Use the full build script when you specifically need a real installable
`.app` to test packaging, the bundled CLI sidecar, file associations, or
to hand a build to someone else on Apple Silicon.

Desktop artifacts are unsigned previews, but normal daemon calls use the
bundled PyInstaller CLI sidecar and do not require a separate Python checkout.
Settings -> Desktop -> Terminal command can install a user-local `kassiber`
launcher without administrator privileges. The launcher forwards to the
installed desktop executable with `--cli`, so the desktop bundle's sidecar is
used from a normal terminal. The GUI executable also forwards `--cli ...`
directly, for example:

```bash
./kassiber-linux-x64.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

If the GUI executable is symlinked with the exact stem `kassiber`, plain CLI
arguments are also forwarded. Use the Settings-managed launcher, or pass
`--cli ...` for any other executable name.

macOS `.app` bundles also include `Contents/Resources/bin/kassiber`, a stable
launcher that package managers can link directly. A future Homebrew tap should
use Homebrew's `binary` stanza against that path; see
[Homebrew Cask](homebrew-cask.md).

`KASSIBER_PYTHON` remains available as an intentional debug override for daemon
startup and installed-app CLI forwarding.

There is no standalone Windows CLI artifact yet. Windows coverage is
desktop-preview only, and the installed desktop executable can forward
`--cli ...` to the bundled CLI sidecar.

## Commit Identity

Every GitHub Actions run records the source ref and commit SHA. For tag
prereleases, the tag itself is the source-of-truth link back to the commit.

The desktop shell displays the build commit beside the version number. CLI
binaries, artifact filenames, and `SHA256SUMS.txt` entries do not currently
include or embed the source commit hash.

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
