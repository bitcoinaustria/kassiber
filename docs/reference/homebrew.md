# Homebrew

Kassiber publishes two Homebrew packages from a project-owned tap:

- **Cask `kassiber`** — the desktop app. The macOS `.app` bundle includes a
  managed launcher at `Contents/Resources/bin/kassiber` that forwards to the
  bundled CLI sidecar via the desktop executable, and the cask links that
  launcher as the terminal command. Installing the cask therefore yields both
  the GUI and a working `kassiber` command with no further steps.
- **Formula `kassiber-cli`** — the CLI-only frozen executable, with no desktop
  GUI dependencies. It installs the same one-file binary that the CLI-only
  release archives ship (macOS arm64, macOS x86_64, and — for Homebrew on
  Linux — Linux x86_64).

The cask and the formula both provide the `kassiber` command. Homebrew has no
cask<->formula conflict mechanism (`conflicts_with` only accepts same-type
targets), so both render a caveat telling users to install one or the other.

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

1. Builds and uploads `kassiber-macos-universal.dmg` plus the
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

With a tap in place, users can install the desktop app and terminal command:

```bash
brew tap bitcoinaustria/kassiber
brew install --cask kassiber
kassiber status
```

Or the CLI only, without any desktop GUI dependencies:

```bash
brew tap bitcoinaustria/kassiber
brew install kassiber-cli
kassiber status
```

Both routes use Homebrew's own prefix for the terminal command, so they do not
need Kassiber's Settings -> Desktop -> Terminal command helper. That helper
still matters for users who install the `.dmg` directly or do not use
Homebrew. Settings recognizes a Homebrew-managed `kassiber` command and does
not offer to overwrite it.

## Release discipline

Only publish tap updates for immutable tags and release assets. Homebrew
validates every download against the rendered SHA-256 checksums; replacing an
existing release asset after the tap has been updated can break installs for
users whose local Homebrew metadata or download cache no longer matches the
asset.
