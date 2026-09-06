# Homebrew

Kassiber publishes two Homebrew packages from a project-owned tap:

- **Cask `kassiber`** — the desktop app. The macOS `.app` bundle includes a
  managed launcher at `Contents/Resources/bin/kassiber` that forwards to the
  bundled CLI sidecar via the desktop executable, and the cask links that
  launcher as the terminal command. Installing the cask therefore yields both
  the GUI and a working `kassiber` command with no further steps.
- **Formula `kassiber-cli`** — the terminal entrypoint. Verified macOS releases
  install the complete notarized app runtime under `libexec/Kassiber.app` and
  link its managed launcher; invoking it does not launch the GUI. Homebrew on
  Linux installs the frozen one-file CLI executable (Linux x86_64).

Both packages are Apple Silicon only on macOS; the cask declares
`depends_on arch: :arm64`. Intel Macs run Kassiber from source.

The cask and the formula both provide the `kassiber` command. Homebrew has no
cask<->formula conflict mechanism (`conflicts_with` only accepts same-type
targets), so both render a caveat telling users to install one or the other.

## Install and trust

Install one package by its fully qualified name:

```bash
# Desktop app plus the kassiber terminal command
brew install --cask bitcoinaustria/kassiber/kassiber

# Or: terminal command without installing the desktop app in Applications
brew install bitcoinaustria/kassiber/kassiber-cli
```

The fully qualified form is deliberate. `bitcoinaustria/kassiber` is a
project-owned third-party tap, not a repository maintained or reviewed by
Homebrew. Homebrew 6 and later require explicit trust for non-official tap
content because formula and cask definitions are executable Ruby. Installing a
fully qualified cask or formula automatically adds the tap and trusts only that
named item, which is Homebrew's recommended least-privilege flow.

Running `brew tap bitcoinaustria/kassiber` by itself only adds the repository;
it does not grant trust, so the resulting warning is expected. If the tap is
already present, run one of the fully qualified install commands above. Do not
disable Homebrew's tap-trust checks. Whole-tap trust is also unnecessary unless
you deliberately want to trust every current and future item published there.
See Homebrew's [Tap Trust](https://docs.brew.sh/Tap-Trust) documentation for the
underlying security model.

Tap trust and macOS Gatekeeper are separate checks:

- Homebrew trust controls whether Homebrew may evaluate the tap's package
  definition.
- Gatekeeper checks the downloaded macOS application. New verified releases
  require Developer ID signing and notarization; historical unsigned previews
  still need the first-launch approval described below. Merging the release
  tooling does not retroactively sign existing downloads.

## Tap setup

The tap lives at
[bitcoinaustria/homebrew-kassiber](https://github.com/bitcoinaustria/homebrew-kassiber):

```text
bitcoinaustria/homebrew-kassiber
├── Casks
│   └── kassiber.rb
└── Formula
    └── kassiber-cli.rb
```

Automated updates need a repository secret on `bitcoinaustria/kassiber`:

- `HOMEBREW_TAP_TOKEN` — a fine-grained GitHub token with write access to
  `bitcoinaustria/homebrew-kassiber` contents.

When the secret is absent, the release workflows can still publish Kassiber
artifacts but skip or reject the requested Homebrew update as appropriate.

## Publishing

For a tag build or `workflow_dispatch` run with `publish_release=true`, the
build workflow creates a draft only. It never publishes or updates the tap,
even while the release-signing policy is disabled. After the activation
requirements in [macOS release trust](macos-release.md) are met, publication
uses the following sequence:

1. Build unsigned inputs. Sign the macOS app locally, then notarize and staple
   it in the gated workflow. Generate the final DMG, app ZIP, and CLI archive
   from that same sealed app.
2. Generate `kassiber-<version>-manifest.txt` over the final release asset set
   and obtain its detached OpenPGP signature offline.
3. `finalize-signed-release.yml` authenticates the manifest signature, checks
   the exact asset set, and verifies the macOS signatures, tickets, provenance,
   and smoke tests without rebuilding or replacing release files.
4. When `publish_homebrew=true`, require `HOMEBREW_TAP_TOKEN`, check out
   `bitcoinaustria/homebrew-kassiber`, and render `Casks/kassiber.rb` and
   `Formula/kassiber-cli.rb` from the authenticated manifest hashes.
5. Recheck that the release assets, tag commit, and draft state are unchanged,
   then publish the verified draft.
6. Commit and push `Update Kassiber cask and CLI formula to <tag>`. A failed
   tap push leaves the already verified release published; rerunning the
   finalizer re-verifies those same assets and retries the tap update without
   replacing assets or requiring another signature.

The generated cask points at the immutable GitHub release DMG and links the
bundled terminal launcher:

```ruby
app "Kassiber.app"
binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",
       target: "kassiber"
```

The generated formula selects the matching CLI archive per platform and
architecture. On macOS it preserves the sealed app under `libexec` with
`skip_clean "libexec/Kassiber.app"` and `preserve_rpath`: Homebrew must not
clean bundled Python metadata or rewrite library install names after signing.
The release helper prepares and checks compatible Mach-O linkage before the
app is sealed, and verifies that invariant again on the sealed artifacts.
Linux continues to use `bin.install "kassiber"` directly.

Users install the desktop app and terminal command with a scoped-trust command:

```bash
brew install --cask bitcoinaustria/kassiber/kassiber
kassiber status
```

Or the terminal command without installing the app in Applications:

```bash
brew install bitcoinaustria/kassiber/kassiber-cli
kassiber status
```

Packaged human-terminal runs use a public, per-user release cache and
show a colored update notice when a newer version is available. The notice is
informational only. Failed background attempts wait an hour before retrying;
successful metadata remains fresh for 20 hours. For these two Homebrew routes
it prints the matching manual command:

```bash
brew upgrade --cask bitcoinaustria/kassiber/kassiber
brew upgrade bitcoinaustria/kassiber/kassiber-cli
```

`kassiber update --enable-checks` persists the app-wide permission and performs
the first foreground check. Later `kassiber update` calls show the same guidance
while permission remains enabled; `--disable-checks` revokes it without network
access. Kassiber never invokes either Homebrew command itself. Machine,
structured-format, non-interactive, daemon, redirected-output, and
source-checkout runs do not perform the automatic check.

Both routes use Homebrew's own prefix for the terminal command, so they do not
need Kassiber's Settings -> Desktop -> Terminal command helper. That helper
still matters for users who install the `.dmg` directly or do not use
Homebrew. Settings recognizes a Homebrew-managed `kassiber` command and does
not offer to overwrite it.

## Unsigned builds and Gatekeeper

This section describes historical unsigned previews, not the acceptance bar
for new production releases. The [macOS release pipeline](macos-release.md)
now requires Developer ID signing, notarization and verification before
publication. Never use a quarantine bypass as a release acceptance test.
Final macOS CLI formula archives preserve the full notarized app runtime
under `libexec`; invoking the CLI does not launch the GUI.

Homebrew's integrity model is the SHA-256 checksum in the rendered files, so
installs and upgrades work without Apple code signing or notarization. What
signing changes is Gatekeeper friction, and it differs per package:

The Homebrew repository checksum protects against download corruption and
asset replacement after the tap commit. A Kassiber OpenPGP signature over the
source manifest separately authenticates those checksums to the independently
published release key; neither mechanism replaces the other.

- **Cask**: Homebrew applies the macOS quarantine attribute to downloaded
  apps by default, so the unsigned, un-notarized Kassiber.app triggers
  Gatekeeper on first launch — and again after every upgrade, because each
  upgrade installs a fresh quarantined copy. Users approve it via System
  Settings -> Privacy & Security -> "Open Anyway" (macOS 15+ removed the
  right-click-Open shortcut). Installing with
  `brew install --cask --no-quarantine bitcoinaustria/kassiber/kassiber`
  skips the prompt at the user's own discretion. New production releases
  instead require the signing and notarization pipeline described above.
- **Formula**: the frozen CLI is downloaded by Homebrew itself, which does
  not quarantine formula resources, and the arm64 binary carries the ad-hoc
  signature PyInstaller applies. `kassiber-cli` therefore runs without any
  Gatekeeper prompt, making it the lowest-friction macOS path until the
  desktop app is notarized.

## Release discipline

Only publish tap updates for immutable tags and release assets. Homebrew
validates every download against the rendered SHA-256 checksums; replacing an
existing release asset after the tap has been updated can break installs for
users whose local Homebrew metadata or download cache no longer matches the
asset.
