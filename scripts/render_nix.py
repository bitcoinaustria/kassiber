"""Render a release-pinned binary Nix flake for Kassiber."""

from __future__ import annotations

import argparse
import json
import re
import shutil
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RELEASE_URL_BASE = "https://github.com/bitcoinaustria/kassiber/releases/download"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
VERSION_RE = re.compile(r"^[0-9]+(?:\.[0-9A-Za-z_]+)+$")


def normalized_version(version: str) -> str:
    normalized = version[1:] if version.startswith("v") else version
    if not VERSION_RE.fullmatch(normalized):
        raise ValueError(f"invalid Nix package version: {version!r}")
    return normalized


def validated_sha256(sha256: str, artifact: str) -> str:
    if not SHA256_RE.fullmatch(sha256):
        raise ValueError(f"sha256 for {artifact} must be 64 lowercase hex characters")
    return sha256


def marker(surface: str, package_name: str, executables: list[str]) -> str:
    return json.dumps(
        {
            "schema_version": 1,
            "product": "kassiber",
            "surface": surface,
            "artifact_kind": "nix-binary",
            "package_name": package_name,
            "package_manager": "nix",
            "repository_manager": "nix",
            "repository_provenance": "probe-required",
            "executables": executables,
        },
        indent=2,
    ) + "\n"


def render_flake(
    version: str,
    sha256_desktop: str,
    sha256_cli: str,
    output: Path,
) -> None:
    version = normalized_version(version)
    desktop_artifact = "kassiber-linux-x64.AppImage"
    cli_artifact = "kassiber-cli-linux-x64.tar.gz"
    desktop_sha256 = validated_sha256(sha256_desktop, desktop_artifact)
    cli_sha256 = validated_sha256(sha256_cli, cli_artifact)
    output.mkdir(parents=True, exist_ok=True)
    (output / "desktop-install-context.json").write_text(
        marker("desktop", "kassiber", ["bin/kassiber-ui", "bin/kassiber"]),
        encoding="utf-8",
    )
    (output / "cli-install-context.json").write_text(
        marker("cli", "kassiber-cli", ["bin/kassiber"]),
        encoding="utf-8",
    )
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")

    desktop_url = f"{RELEASE_URL_BASE}/v{version}/{desktop_artifact}"
    cli_url = f"{RELEASE_URL_BASE}/v{version}/{cli_artifact}"
    flake = f'''{{
  description = "Kassiber release packages";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {{ self, nixpkgs }}:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {{ inherit system; }};
      inherit (pkgs) lib;

      cli = pkgs.stdenvNoCC.mkDerivation {{
        pname = "kassiber-cli";
        version = "{version}";
        src = pkgs.fetchurl {{
          url = "{cli_url}";
          sha256 = "{cli_sha256}";
        }};
        sourceRoot = "kassiber-cli-linux-x64";
        nativeBuildInputs = [ pkgs.autoPatchelfHook ];
        buildInputs = [ pkgs.stdenv.cc.cc.lib pkgs.zlib ];
        installPhase = ''
          runHook preInstall
          install -Dpm755 kassiber "$out/bin/kassiber"
          install -Dpm644 ${{./cli-install-context.json}} \
            "$out/lib/kassiber/install-context.json"
          install -Dpm644 ${{./LICENSE}} \
            "$out/share/licenses/kassiber-cli/LICENSE"
          runHook postInstall
        '';
        meta = {{
          description = "Local-first Bitcoin accounting CLI";
          homepage = "https://github.com/bitcoinaustria/kassiber";
          license = lib.licenses.agpl3Only;
          platforms = [ "x86_64-linux" ];
          sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
          mainProgram = "kassiber";
        }};
      }};

      desktopSrc = pkgs.fetchurl {{
        url = "{desktop_url}";
        sha256 = "{desktop_sha256}";
      }};
      desktopContents = pkgs.appimageTools.extractType2 {{
        pname = "kassiber";
        src = desktopSrc;
        version = "{version}";
      }};
      desktop = pkgs.appimageTools.wrapType2 {{
        pname = "kassiber";
        version = "{version}";
        src = desktopSrc;
        nativeBuildInputs = [ pkgs.makeWrapper ];
        extraPkgs = pkgs: with pkgs; [ gtk3 webkitgtk_4_1 ];
        extraInstallCommands = ''
          mv "$out/bin/kassiber" "$out/bin/kassiber-ui"
          makeWrapper "$out/bin/kassiber-ui" "$out/bin/kassiber" \
            --add-flags "--cli"
          install -Dpm644 ${{./desktop-install-context.json}} \
            "$out/lib/kassiber/install-context.json"
          install -Dpm644 ${{./LICENSE}} \
            "$out/share/licenses/kassiber/LICENSE"
          desktop_file=""
          for candidate in \
            "${{desktopContents}}/Kassiber.desktop" \
            "${{desktopContents}}/usr/share/applications/Kassiber.desktop"; do
            if [ -f "$candidate" ]; then
              desktop_file="$candidate"
              break
            fi
          done
          if [ -n "$desktop_file" ]; then
            install -Dpm644 "$desktop_file" \
              "$out/share/applications/Kassiber.desktop"
            sed -i \
              's|^Exec=[^[:space:]]*|Exec=kassiber-ui|' \
              "$out/share/applications/Kassiber.desktop"
          fi
          for size in 32x32 128x128 256x256@2; do
            icon="${{desktopContents}}/usr/share/icons/hicolor/$size/apps/kassiber-ui.png"
            if [ -f "$icon" ]; then
              install -Dpm644 "$icon" \
                "$out/share/icons/hicolor/$size/apps/kassiber-ui.png"
            fi
          done
        '';
        meta = {{
          description = "Local-first Bitcoin accounting suite";
          homepage = "https://github.com/bitcoinaustria/kassiber";
          license = lib.licenses.agpl3Only;
          platforms = [ "x86_64-linux" ];
          sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
          mainProgram = "kassiber-ui";
        }};
      }};
    in {{
      packages.${{system}} = {{
        inherit cli desktop;
        default = desktop;
      }};
      apps.${{system}} = {{
        default = {{ type = "app"; program = "${{desktop}}/bin/kassiber-ui"; }};
        cli = {{ type = "app"; program = "${{cli}}/bin/kassiber"; }};
      }};
    }};
}}
'''
    (output / "flake.nix").write_text(flake, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha256-desktop", required=True)
    parser.add_argument("--sha256-cli", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    render_flake(
        args.version,
        args.sha256_desktop,
        args.sha256_cli,
        args.output,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
