"""Render the Kassiber Homebrew cask file."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
VERSION_RE = re.compile(r"^[0-9]+(?:\.[0-9A-Za-z_-]+)+$")


def render_cask(version: str, sha256: str) -> str:
    normalized_version = version[1:] if version.startswith("v") else version
    if not VERSION_RE.match(normalized_version):
        raise ValueError(f"invalid cask version: {version!r}")
    if not SHA256_RE.match(sha256):
        raise ValueError("sha256 must be 64 lowercase hex characters")

    return f'''cask "kassiber" do
  version "{normalized_version}"
  sha256 "{sha256}"

  url "https://github.com/bitcoinaustria/kassiber/releases/download/v#{{version}}/kassiber-macos-universal.dmg"
  name "Kassiber"
  desc "Local-first Bitcoin accounting suite"
  homepage "https://github.com/bitcoinaustria/kassiber"

  app "Kassiber.app"
  binary "#{{appdir}}/Kassiber.app/Contents/Resources/bin/kassiber",
         target: "kassiber"

  zap trash: [
    "~/Library/Application Support/at.bitcoinaustria.kassiber",
    "~/Library/Preferences/at.bitcoinaustria.kassiber.plist",
    "~/Library/Saved Application State/at.bitcoinaustria.kassiber.savedState",
  ]
end
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True, help="Release version, with or without v prefix")
    parser.add_argument("--sha256", required=True, help="SHA-256 of kassiber-macos-universal.dmg")
    parser.add_argument("--output", required=True, type=Path, help="Path to write Casks/kassiber.rb")
    args = parser.parse_args()

    rendered = render_cask(args.version, args.sha256)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
