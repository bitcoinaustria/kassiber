# Homebrew Cask

Kassiber can publish a project-owned Homebrew cask from the prerelease workflow.
The macOS `.app` bundle includes a managed launcher at
`Contents/Resources/bin/kassiber` that forwards to the bundled CLI sidecar via
the desktop executable, and `.github/workflows/prerelease-binaries.yml` can
write a cask that links that launcher as the terminal command.

## Tap setup

Create the tap repository:

```text
bitcoinaustria/homebrew-kassiber
└── Casks
    └── kassiber.rb
```

Then add a repository secret to `bitcoinaustria/kassiber`:

- `HOMEBREW_TAP_TOKEN` — a fine-grained GitHub token with write access to
  `bitcoinaustria/homebrew-kassiber` contents.

When the secret is absent, the prerelease workflow still publishes Kassiber
release artifacts and skips the Homebrew update.

## Publishing

For a tag publish or `workflow_dispatch` run with `publish_release=true`, the
release job:

1. Builds and uploads `kassiber-macos-universal.dmg`.
2. Generates `SHA256SUMS.txt`.
3. Checks out `bitcoinaustria/homebrew-kassiber` when `HOMEBREW_TAP_TOKEN` is
   configured.
4. Renders `Casks/kassiber.rb` with `scripts/render_homebrew_cask.py`.
5. Commits and pushes `Update Kassiber cask to <tag>`.

The generated cask points at the immutable GitHub release DMG and links the
bundled terminal launcher:

```ruby
app "Kassiber.app"
binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",
       target: "kassiber"
```

With a tap in place, users can install the desktop app and terminal command via:

```bash
brew tap bitcoinaustria/kassiber
brew install --cask kassiber
kassiber status
```

The cask route uses Homebrew's own prefix for the terminal command, so it does
not need Kassiber's Settings -> Desktop -> Terminal command helper. That helper
still matters for users who install the `.dmg` directly or do not use Homebrew.

## Release discipline

Only publish cask updates for immutable tags and DMG assets. Homebrew validates
the downloaded DMG against the cask's SHA-256 checksum; replacing an existing
release asset after the tap has been updated can break installs for users whose
local Homebrew metadata or download cache no longer matches the asset.
