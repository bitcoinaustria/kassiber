# Prerelease Binaries

Kassiber is still in early development. Version tags publish prerelease
artifacts, while pull requests that touch packaging inputs run the same build
matrix without publishing.

## What Runs Automatically

- Pull requests run `.github/workflows/ci.yml`. Pull requests that touch the
  release workflow, packaging scripts/metadata, frozen CLI inputs, or Tauri
  bundle inputs also run `.github/workflows/prerelease-binaries.yml` without
  publishing.
- Pushes to `main` run `.github/workflows/ci.yml` only.
- Pushes of tags matching `v*` run `.github/workflows/prerelease-binaries.yml`
  and publish the resulting artifacts to a GitHub prerelease.

Keep the packaging workflow's pull-request path filter narrow. For one-off
branch artifacts outside those paths, use a manual workflow run instead.

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
the intended commit in protected `main` history. Repository rules must restrict
creation, update, and deletion of `v*` tags to the release maintainers; release
publication also checks the tag commit is an ancestor of `origin/main` before
the publish job executes the tag's release policy or publishing helpers:

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

After the permanent OpenPGP release key is published, production releases use
the same command with `-f draft_release=true`. Download and sign the generated
manifest offline, upload its `.asc` file to the draft, and have a second
operator verify it. The second operator then runs
`.github/workflows/finalize-signed-release.yml` against `main`. That workflow
downloads the existing draft assets, authenticates the signature and exact
artifact set against the code-reviewed release policy, renders Homebrew hashes
from that authenticated manifest, and publishes the existing draft without
rebuilding or replacing any asset. Once `packaging/release/signing-policy.json`
is enabled, tag pushes and non-draft build runs fail closed instead of taking an
unsigned publishing path.

Only use `publish_release=true` for real prerelease tags. PR and branch tester
builds should stay workflow artifacts, not GitHub Releases.

The publish job intentionally waits for every CLI and desktop matrix leg. A
failed macOS, Linux, or Windows leg blocks the prerelease instead of shipping a
partial artifact set. Re-run the failed workflow/job after fixing runner or
packaging failures; do not create a partial release unless the user explicitly
asks for one.

## Artifact Set

The workflow currently builds:

- CLI-only releases: macOS arm64 and Linux x86_64 one-file PyInstaller
  binaries as `.tar.gz` archives, Windows x86_64 as a `.zip`, and Linux
  x86_64 additionally as GUI-free `kassiber-cli` `.deb` and binary `.rpm`
  packages. The extracted
  executable is named `kassiber` (`kassiber.exe` on Windows). These artifacts
  do not require the desktop app. Linux is built on Ubuntu 22.04 to keep the
  glibc floor aligned with the AppImage build.
- Desktop previews: a macOS arm64 `.app` zip plus `.dmg`, Linux `.AppImage`,
  `.deb`, and binary `.rpm`, and Windows `.msi` plus
  NSIS setup `.exe`, published with short user-facing filenames. Each desktop
  preview includes the exact one-file
  Kassiber CLI executable produced by the matching CLI-only matrix leg.

macOS is Apple Silicon only. Intel macOS builds were dropped deliberately:
they could not ship the pinned `lwk` wheel (no macOS x86_64 wheel exists), so
they silently downgraded Liquid observation to the compatibility route, and
half-capable builds are worse than an explicit platform floor. Intel Mac users
can run from source.

Tag pushes keep the existing safe default and publish as prereleases. A manual
workflow run can select `release_channel=release` to publish the same verified
artifact set as a stable GitHub release. The embedded `BUILD_INFO.json` reports
the selected `prerelease` or `release` channel.

Tag and publishing runs fail before building when the tag without its leading
`v` does not exactly match the Python package version. This prevents a release
tag, Debian/RPM package metadata, Homebrew definition, and embedded build
identity from advertising different versions.

Pull requests that change the release workflow, frozen-CLI inputs, or Tauri
packaging files run the same cross-platform CLI and desktop build matrix without
publishing. These CI artifacts identify themselves as `dev`; tag and manual
publishes remain `prerelease` or `release`.

Public release filenames intentionally omit the release version because the
GitHub release tag already supplies it:

```text
kassiber-cli-linux-x64.tar.gz
kassiber-cli-linux-x64.deb
kassiber-cli-linux-x64.rpm
kassiber-cli-macos-arm64.tar.gz
kassiber-cli-windows-x64.zip
kassiber-linux-x64.deb
kassiber-linux-x64.rpm
kassiber-linux-x64.AppImage
kassiber-macos-arm64.app.zip
kassiber-macos-arm64.dmg
kassiber-windows-x64.exe
kassiber-windows-x64.msi
kassiber-<version>-manifest.txt
```

The versioned manifest uses Sparrow's `sha256sum` shape plus comment headers
that bind its format revision and semantic version; standard `sha256sum
--check` ignores those headers and remains compatible.
Once the permanent Kassiber release key is published, signed releases also
carry `kassiber-<version>-manifest.txt.asc`. The manifest signature
authenticates every artifact hash without requiring a separate PGP signature
for every package. See [Release signing](release-signing.md).

When the repository secret `HOMEBREW_TAP_TOKEN` is configured, successful
release publishes also update `bitcoinaustria/homebrew-kassiber` with a cask
for `kassiber-macos-arm64.dmg` and a `kassiber-cli` formula for the
CLI-only archives. See [Homebrew](homebrew.md) for the tap setup and
immutability requirements.

Use `x64` for public filenames instead of `x86_64`. Bundled sidecar resource
filenames are internal to the desktop package and use Rust target triples such
as `kassiber-cli-aarch64-apple-darwin`; those raw sidecars are not release
assets.

The macOS desktop leg uses GitHub's `macos-latest` runner with Tauri's
`--target aarch64-apple-darwin` and bundles the single arm64 CLI sidecar.

The shared CLI build always collects pinned `bdkpython` and pinned `lwk`;
every packaged platform (macOS arm64, Linux x86_64, Windows x86_64) ships
both, so packaged builds never fall back to the compatibility observer for
missing wheels.

### Local Apple Silicon build

For a local Apple Silicon build without Rosetta, use the arm64-only helper:

```bash
./scripts/build-macos-arm64-app.sh
# skip the DMG if you only want the .app:
BUNDLES=app ./scripts/build-macos-arm64-app.sh
# additionally install/repair the user-local shell command:
./scripts/build-macos-arm64-app.sh --install-cli
```

It builds the PyInstaller sidecar as
`ui-tauri/src-tauri/binaries/kassiber-cli-aarch64-apple-darwin`, verifies that
the executable is arm64, and runs Tauri with
`--target aarch64-apple-darwin --bundles app,dmg`. The result is a full
unsigned desktop app and DMG under
`ui-tauri/src-tauri/target/aarch64-apple-darwin/release/bundle`.
With `--install-cli`, the helper first smokes the finished app bundle's
`Contents/Resources/bin/kassiber` launcher and then installs a managed wrapper
through the desktop binary's same Rust implementation used by Settings. That
single implementation chooses `~/.local/bin` or `~/bin`, refuses conflicts, and
adds one marked PATH block to the current shell profile when needed. It never
starts a daemon or background process; each invocation remains a normal one-shot
CLI process. Without the flag, a local build does not modify the home directory.

The helper defaults to Python 3.11 to match the GitHub Actions prerelease
workflow; set `PYTHON_VERSION=<version>` only for intentional local debugging.
The Tauri package version is injected from `[project].version` in
`pyproject.toml`, so local app builds follow the Python package version instead
of the placeholder value in `tauri.conf.json`. The local helper also sets the
desktop display label to `dev` and injects the current Git commit, so the app
footer reads like `Kassiber dev · abc1234`.

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
launcher without administrator privileges for direct-DMG and portable builds.
It also maintains one marked shell-profile PATH block when needed. The launcher
forwards to the installed desktop executable with `--cli`, so the desktop
bundle's sidecar is used from a normal terminal. Native installers integrate
the command themselves: Homebrew links the app resource launcher, Linux `.deb`
installs `/usr/bin/kassiber`, and Windows MSI/NSIS packages add their bundled
`bin` directory to PATH. Installer upgrades retain the stable command path and
uninstall removes only the installer-owned PATH entry. Nothing autostarts.

The GUI executable also forwards `--cli ...` directly, for example:

```bash
./kassiber-linux-x64.AppImage --cli status
/Applications/Kassiber.app/Contents/MacOS/kassiber-ui --cli status
Kassiber.exe --cli status
```

If the GUI executable is symlinked with the exact stem `kassiber`, plain CLI
arguments are also forwarded. Use the Settings-managed launcher, or pass
`--cli ...` for any other executable name.

macOS `.app` bundles also include `Contents/Resources/bin/kassiber`, a stable
launcher that the Homebrew cask links directly with its `binary` stanza. A
`kassiber-cli` formula covers CLI-only installs; see [Homebrew](homebrew.md).

`KASSIBER_PYTHON` remains available as an intentional debug override for daemon
startup and installed-app CLI forwarding.

CLI-only archives are intentionally portable: extract them and place the
`kassiber` executable in an existing PATH directory, or invoke it by path.
They do not edit PATH or require a GUI. Linux users can instead install the
CLI-only Debian or RPM package; it owns `/usr/bin/kassiber`, conflicts cleanly
with the matching desktop package, and has no GTK/WebKit dependency. The
update checker treats these packages as manual installs and shows the GitHub
release link; see [Linux packaging](linux-packaging.md) for why no `apt`/`dnf`
command is offered.

## Commit Identity

Every packaged CLI embeds `BUILD_INFO.json`, and every CLI-only archive includes
a readable copy beside the executable. `kassiber --version` prints the package
version plus the channel and abbreviated commit without opening the database.
The desktop uses that exact executable as its sidecar, while its footer keeps
the existing version/commit display. The build record contains:

```json
{
  "commit": "${GITHUB_SHA}",
  "ref": "${GITHUB_REF_NAME}",
  "run_id": "${GITHUB_RUN_ID}",
  "built_at": "<UTC timestamp>",
  "channel": "prerelease",
  "version": "<package version>"
}
```

`scripts/write_build_info.py` honors `SOURCE_DATE_EPOCH`; CI and the local macOS
builder derive it from the source commit timestamp. This removes wall-clock
drift from `built_at` while retaining the run id as explicit provenance.

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
pnpm tauri build --target aarch64-apple-darwin --bundles app,dmg --ci
```
