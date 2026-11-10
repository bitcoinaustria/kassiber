# Prerelease Binaries

Kassiber is still in early development. Version tags stage draft release
artifacts, while pull requests that touch packaging inputs run the same build
matrix without publishing. New public releases must complete the
[local signing / CI notarization runbook](macos-release.md) and SSH
release-manifest signing; older unsigned downloads are not retroactively signed.

## What Runs Automatically

- Pull requests run `.github/workflows/ci.yml`. Pull requests that touch the
  release workflow, packaging scripts/metadata, the Rust backend crates and
  the data they embed, the Rust tax engine, the pinned Rust toolchain, or Tauri bundle inputs also run
  `.github/workflows/prerelease-binaries.yml` without publishing.
- Pushes to `main` run `.github/workflows/ci.yml` only.
- Pushes of tags matching `v*` run `.github/workflows/prerelease-binaries.yml`
  and stage the resulting artifacts in a GitHub draft. Only
  `finalize-signed-release.yml` publishes after verification.

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

Then either push a new `v*` tag, which stages a draft automatically, or manually
run the workflow with draft staging enabled for an existing tag:

```bash
gh workflow run prerelease-binaries.yml \
  --repo bitcoinaustria/kassiber \
  --ref <tag-name> \
  -f publish_release=true \
  -f tag_name=<tag-name>
```

The build always leaves a draft, regardless of `draft_release` or whether the
release-signing policy is enabled. Follow the
[macOS release runbook](macos-release.md) to sign locally, notarize in CI, and
verify the final artifacts **before** signing their manifest with the SSH
release key. Only `finalize-signed-release.yml` on `main` can publish after
authenticating the exact asset set against the enabled, code-reviewed signing
policy and checking the sealed macOS distributions.
It then updates Homebrew from the authenticated hashes. A failed tap push can
be retried without replacing release assets. The documented authorization
model supports one maintainer; a second operator is not silently required.

Only use `publish_release=true` for real prerelease tags. PR and branch tester
builds should stay workflow artifacts, not GitHub Releases.

The publish job intentionally waits for every CLI and desktop matrix leg. A
failed macOS, Linux, or Windows leg blocks the prerelease instead of shipping a
partial artifact set. Re-run the failed workflow/job after fixing runner or
packaging failures; do not create a partial release unless the user explicitly
asks for one.

## Artifact Set

The workflow currently builds:

- Unsigned CLI build inputs: the Rust `kassiber` binary for macOS arm64 and
  Linux x86_64 as `.tar.gz` archives, Windows x86_64 as a `.zip`, and Linux
  x86_64 additionally as GUI-free `kassiber-cli` `.deb` and binary `.rpm`
  packages. The extracted
  executable is named `kassiber` (`kassiber.exe` on Windows), with
  `BUILD_INFO.json` and the AI provider broker (`provider_broker/index.mjs`)
  beside it. These artifacts do not require the desktop app. Linux is built on
  Ubuntu 22.04 to keep the glibc floor aligned with the AppImage build, and
  the build refuses a binary above that floor.
- Desktop previews: a macOS arm64 `.app` zip plus `.dmg`, Linux `.AppImage`,
  `.deb`, and binary `.rpm`, and Windows `.msi` plus
  NSIS setup `.exe`, published with short user-facing filenames. Every preview
  carries the exact binary the matching CLI-only matrix leg built and smoked,
  under its target-triple sidecar name; macOS keeps it under
  `binaries/kassiber-cli/`, the location and signing identifier the Python
  one-dir sidecar used. See [Rust packaging](rust-packaging.md).

The final verified macOS CLI archive replaces the unsigned input with
the complete notarized `Kassiber.app` runtime and its terminal launcher. The
formula installs it under `libexec`, not Applications; CLI calls do not launch
the GUI. Final DMG, app ZIP, and CLI archive share the same signed app payload;
stapled-ticket metadata inside the DMG may differ.

macOS is Apple Silicon only. Intel macOS builds were dropped deliberately:
they could not ship the pinned `lwk` wheel (no macOS x86_64 wheel exists), so
they silently downgraded Liquid observation to the compatibility route, and
half-capable builds are worse than an explicit platform floor. Intel Mac users
can run from source.

Tag pushes stage drafts marked as prereleases. A manual workflow run can select
`release_channel=release` to stage a stable-release draft instead; finalization
must use the matching channel. The embedded `BUILD_INFO.json` reports
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
Signed releases also carry `kassiber-<version>-manifest.txt.sig`, a detached
OpenSSH signature from the Kassiber release key
(`SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M`). The manifest signature
authenticates every artifact hash without requiring a separate signature for
every package. See [Release signing](release-signing.md).

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

The Rust binary links BDK and LWK natively on every packaged platform (macOS
arm64, Linux x86_64, Windows x86_64), so packaged builds never fall back to a
compatibility observer.

### Local Apple Silicon build

For a local Apple Silicon build without Rosetta, use the arm64-only helper:

```bash
./scripts/build-macos-arm64-app.sh
# skip the DMG if you only want the .app:
BUNDLES=app ./scripts/build-macos-arm64-app.sh
# additionally install/repair the user-local shell command:
./scripts/build-macos-arm64-app.sh --install-cli
```

It builds the one-dir PyInstaller sidecar as
`ui-tauri/src-tauri/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin`,
verifies that the executable is arm64, and runs Tauri with
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
bundled Rust sidecar and do not require a separate Python checkout.
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

`KASSIBER_BIN` is the intentional debug override for daemon startup and
installed-app CLI forwarding.

CLI-only archives are intentionally portable: extract them and place the
`kassiber` executable in an existing PATH directory, or invoke it by path.
They do not edit PATH or require a GUI. Linux users can instead install the
CLI-only Debian or RPM package; it owns `/usr/bin/kassiber`, conflicts cleanly
with the matching desktop package, and has no GTK/WebKit dependency. The
update checker treats these packages as manual installs and shows the GitHub
release link; see [Linux packaging](linux-packaging.md) for why no `apt`/`dnf`
command is offered.

### Local Apple Silicon build with the Rust backend

While [plan 21](../plan/21-rust-backend.md) moves the backend to Rust, a local
app with the Rust binary as its sidecar can be tested beside the Python one:

```bash
./scripts/build-macos-arm64-rust-app.sh
# also build the Python-backed app from the same commit:
./scripts/build-macos-arm64-rust-app.sh --with-python-app
```

The helper builds the production `kassiber` binary (no `test-harness`
feature) for `aarch64-apple-darwin` through `scripts/build-rust-cli.sh`, which
strips it with Xcode's `strip`, ad-hoc signs it under the identifier
`kassiber-cli-aarch64-apple-darwin`, and refuses it unless it is an arm64
Mach-O that links only macOS system libraries and reports
`Kassiber <version> (dev, commit <12 hex>)`. It then runs the packaged-binary
smoke CI runs. It places the binary where the Python sidecar goes,
`binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin`, with the provider
broker beside it, so the supervisor, the bundled CLI launcher and the Touch ID
helper's identity check find it unchanged, and re-signs it under that
identifier if Tauri's bundle signing changed it. It probes the freshly built
bundles and only reads installed apps' build records, never executes them. The app is named **Kassiber Rust Dev**
(`at.bitcoinaustria.kassiber.rust-dev`) and shows the build label `dev-rust`.
Finished apps and DMGs are copied to `dist/macos-arm64/`.

Both apps use the same books. The data location does not depend on the bundle
identifier, and ownership locks let only one desktop app hold a book at a
time; the other reports that another desktop app owns the project. A newer
build may upgrade a book, which an older build then refuses, so switch back
and forth only between apps built from the same commit (`--with-python-app`).
The helper warns when an installed Kassiber Dev or Kassiber app reports
another commit. Requests the Rust backend does not serve yet return
`not_implemented`.

Before finishing, the helper snapshots every local book with
`scripts/macos-kassiber-data.sh snapshot --if-changed`, unless `--no-backup` is
given. A snapshot copies both state roots
(`~/Library/Application Support/at.bitcoinaustria.kassiber` and a legacy
`~/.kassiber`) and every project directory their catalogs point to, into
`~/Library/Application Support/at.bitcoinaustria.kassiber-snapshots/`. The
copy is private to the account and skips live lock and socket files. Snapshots
need every Kassiber app and CLI closed, so the files are consistent; encrypted
books stay encrypted. Keychain items are not included. To roll back:

```bash
./scripts/macos-kassiber-data.sh list
./scripts/macos-kassiber-data.sh restore "<snapshot directory>"
```

Restore renames the current data to `<path>.before-restore-<timestamp>` and
never deletes it.

## Commit Identity

The Rust binary compiles the channel and commit in (`KASSIBER_BUILD_CHANNEL`,
`KASSIBER_BUILD_COMMIT`), and every CLI-only archive and desktop sidecar
directory carries `BUILD_INFO.json` beside the executable; the macOS signer
refuses an app without it. `kassiber --version` prints the package version plus
the channel and abbreviated commit without opening the database. The desktop
uses that exact executable as its sidecar, while its footer keeps the existing
version/commit display. The build record contains:

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
`scripts/build-rust-cli.sh` passes the same epoch to the compiler so vendored
OpenSSL's build stamp is fixed too; see
[Rust packaging](rust-packaging.md#reproducibility).

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
