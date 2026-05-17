# Homebrew Cask

Kassiber does not publish an official Homebrew tap yet, but macOS desktop
artifacts are prepared for one: the `.app` bundle includes a managed launcher at
`Contents/Resources/bin/kassiber` that forwards to the bundled CLI sidecar via
the desktop executable.

Recommended first tap layout:

```text
bitcoinaustria/homebrew-kassiber
└── Casks
    └── kassiber.rb
```

Starter cask:

```ruby
cask "kassiber" do
  version "<version_without_v>"
  sha256 "<sha256 of kassiber-macos-universal.dmg>"

  url "https://github.com/bitcoinaustria/kassiber/releases/download/v#{version}/kassiber-macos-universal.dmg"
  name "Kassiber"
  desc "Local-first Bitcoin accounting suite"
  homepage "https://github.com/bitcoinaustria/kassiber"

  app "Kassiber.app"
  binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",
         target: "kassiber"

  livecheck do
    url :url
    strategy :github_latest
  end

  zap trash: [
    "~/.kassiber",
    "~/Library/Application Support/at.bitcoinaustria.kassiber",
    "~/Library/Preferences/at.bitcoinaustria.kassiber.plist",
    "~/Library/Saved Application State/at.bitcoinaustria.kassiber.savedState",
  ]
end
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
