"""Render the Kassiber Homebrew tap files: the desktop cask and the CLI formula."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
VERSION_RE = re.compile(r"^[0-9]+(?:\.[0-9A-Za-z_-]+)+$")

RELEASE_URL_BASE = "https://github.com/bitcoinaustria/kassiber/releases/download"
TAP_CASK_NAME = "bitcoinaustria/kassiber/kassiber"
TAP_FORMULA_NAME = "bitcoinaustria/kassiber/kassiber-cli"


def normalized_version(version: str) -> str:
    normalized = version[1:] if version.startswith("v") else version
    if not VERSION_RE.match(normalized):
        raise ValueError(f"invalid Homebrew version: {version!r}")
    return normalized


def validated_sha256(sha256: str, artifact: str) -> str:
    if not SHA256_RE.match(sha256):
        raise ValueError(f"sha256 for {artifact} must be 64 lowercase hex characters")
    return sha256


def render_cask(version: str, sha256: str) -> str:
    cask_version = normalized_version(version)
    dmg_sha256 = validated_sha256(sha256, "kassiber-macos-arm64.dmg")

    return f'''cask "kassiber" do
  version "{cask_version}"
  sha256 "{dmg_sha256}"

  url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-macos-arm64.dmg"
  name "Kassiber"
  desc "Local-first Bitcoin accounting suite"
  homepage "https://github.com/bitcoinaustria/kassiber"

  depends_on arch: :arm64

  app "Kassiber.app"
  binary "#{{appdir}}/Kassiber.app/Contents/Resources/bin/kassiber",
         target: "kassiber"

  zap trash: [
    "~/Library/Application Support/at.bitcoinaustria.kassiber",
    "~/Library/Preferences/at.bitcoinaustria.kassiber.plist",
    "~/Library/Saved Application State/at.bitcoinaustria.kassiber.savedState",
  ]

  # Homebrew has no cask<->formula conflict stanza (the cask one only accepts
  # cask:), so the overlap is surfaced as a caveat on both sides.
  caveats <<~EOS
    The {TAP_FORMULA_NAME} formula installs the same
    `kassiber` command. Install either this cask or the formula, not both.
  EOS
end
'''


def render_cli_formula(
    version: str,
    sha256_macos_arm64: str,
    sha256_linux_x64: str,
) -> str:
    formula_version = normalized_version(version)
    macos_arm64 = validated_sha256(sha256_macos_arm64, "kassiber-cli-macos-arm64.tar.gz")
    linux_x64 = validated_sha256(sha256_linux_x64, "kassiber-cli-linux-x64.tar.gz")

    return f'''class KassiberCli < Formula
  desc "Local-first Bitcoin accounting CLI"
  homepage "https://github.com/bitcoinaustria/kassiber"
  version "{formula_version}"
  license "AGPL-3.0-only"

  on_macos do
    on_arm do
      url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-cli-macos-arm64.tar.gz"
      sha256 "{macos_arm64}"
    end
  end

  on_linux do
    on_intel do
      url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-cli-linux-x64.tar.gz"
      sha256 "{linux_x64}"
    end
  end

  def install
    bin.install "kassiber"
  end

  # Formulae cannot declare conflicts with casks, so the overlap is surfaced
  # as user guidance only, mirroring the cask-side caveat.
  def caveats
    <<~EOS
      The Kassiber desktop cask ("{TAP_CASK_NAME}") links its own
      `kassiber` command. Install either kassiber-cli or the desktop cask,
      not both.
    EOS
  end

  test do
    assert_match "Kassiber", shell_output("#{{bin}}/kassiber --version")
  end
end
'''


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="kind", required=True)

    cask = subparsers.add_parser("cask", help="Render Casks/kassiber.rb")
    cask.add_argument("--version", required=True, help="Release version, with or without v prefix")
    cask.add_argument("--sha256", required=True, help="SHA-256 of kassiber-macos-arm64.dmg")
    cask.add_argument("--output", required=True, type=Path)

    formula = subparsers.add_parser("cli-formula", help="Render Formula/kassiber-cli.rb")
    formula.add_argument(
        "--version", required=True, help="Release version, with or without v prefix"
    )
    formula.add_argument(
        "--sha256-macos-arm64", required=True, help="SHA-256 of kassiber-cli-macos-arm64.tar.gz"
    )
    formula.add_argument(
        "--sha256-linux-x64", required=True, help="SHA-256 of kassiber-cli-linux-x64.tar.gz"
    )
    formula.add_argument("--output", required=True, type=Path)

    args = parser.parse_args()
    if args.kind == "cask":
        rendered = render_cask(args.version, args.sha256)
    else:
        rendered = render_cli_formula(
            args.version,
            args.sha256_macos_arm64,
            args.sha256_linux_x64,
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
