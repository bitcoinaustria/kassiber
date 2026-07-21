# Homebrew

Kassiber publishes two Homebrew packages from a project-owned tap:

- **Cask `kassiber`** — the desktop app. The macOS `.app` bundle includes a
  managed launcher at `Contents/Resources/bin/kassiber` that forwards to the
  bundled CLI sidecar via the desktop executable, and the cask links that
  launcher as the terminal command. Installing the cask therefore yields both
  the GUI and a working `kassiber` command with no further steps.
- **Formula `kassiber-cli`** — the CLI-only frozen executable, with no desktop
  GUI dependencies. It installs the same one-file binary that the CLI-only
  release archives ship (macOS arm64 and — for Homebrew on Linux — Linux
  x86_64).

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

# Or: CLI only, without desktop GUI dependencies
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
- Gatekeeper checks the downloaded macOS application. Kassiber's current cask
  is unsigned and unnotarized, so the desktop app still needs the first-launch
  approval described below even after Homebrew trusts its cask.

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

When the secret is absent, the prerelease workflow still publishes Kassiber
release artifacts and skips the Homebrew update.

## Publishing

For a tag publish or `workflow_dispatch` run with `publish_release=true`, the
release job:

1. Builds and uploads `kassiber-macos-arm64.dmg` plus the
   `kassiber-cli-*.tar.gz` archives.
2. Generates `SHA256SUMS.txt`.
3. Checks out `bitcoinaustria/homebrew-kassiber` when `HOMEBREW_TAP_TOKEN` is
   configured.
4. Renders `Casks/kassiber.rb` and `Formula/kassiber-cli.rb` with
   `scripts/render_homebrew.py`.
5. Commits and pushes `Update Kassiber cask and CLI formula to <tag>`.

The generated cask points at the immutable GitHub release DMG and links the
bundled terminal launcher:

```ruby
app "Kassiber.app"
binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",
       target: "kassiber"
```

The generated formula selects the matching CLI archive per platform and
architecture and installs the frozen executable directly:

```ruby
def install
  bin.install "kassiber"
end
```

Users install the desktop app and terminal command with a scoped-trust command:

```bash
brew install --cask bitcoinaustria/kassiber/kassiber
kassiber status
```

Or the CLI only, without any desktop GUI dependencies:

```bash
brew install bitcoinaustria/kassiber/kassiber-cli
kassiber status
```

Both routes use Homebrew's own prefix for the terminal command, so they do not
need Kassiber's Settings -> Desktop -> Terminal command helper. That helper
still matters for users who install the `.dmg` directly or do not use
Homebrew. Settings recognizes a Homebrew-managed `kassiber` command and does
not offer to overwrite it.

## Unsigned builds and Gatekeeper

Homebrew's integrity model is the SHA-256 checksum in the rendered files, so
installs and upgrades work without Apple code signing or notarization. What
signing changes is Gatekeeper friction, and it differs per package:

- **Cask**: Homebrew applies the macOS quarantine attribute to downloaded
  apps by default, so the unsigned, un-notarized Kassiber.app triggers
  Gatekeeper on first launch — and again after every upgrade, because each
  upgrade installs a fresh quarantined copy. Users approve it via System
  Settings -> Privacy & Security -> "Open Anyway" (macOS 15+ removed the
  right-click-Open shortcut). Installing with
  `brew install --cask --no-quarantine bitcoinaustria/kassiber/kassiber`
  skips the prompt at the user's own discretion. Apple Developer ID signing
  plus notarization is the eventual fix and is tracked separately in TODO.md.
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
