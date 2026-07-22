"""Render Kassiber's two binary AUR package repositories."""

from __future__ import annotations

import argparse
import hashlib
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
        raise ValueError(f"invalid AUR version: {version!r}")
    return normalized


def validated_sha256(sha256: str, artifact: str) -> str:
    if not SHA256_RE.fullmatch(sha256):
        raise ValueError(f"sha256 for {artifact} must be 64 lowercase hex characters")
    return sha256


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _write(path: Path, content: str, *, executable: bool = False) -> None:
    path.write_text(content, encoding="utf-8")
    path.chmod(0o755 if executable else 0o644)


def _marker(surface: str, package_name: str, executables: list[str]) -> str:
    return json.dumps(
        {
            "schema_version": 1,
            "product": "kassiber",
            "surface": surface,
            "artifact_kind": "aur-bin",
            "package_name": package_name,
            "package_manager": "pacman",
            "repository_manager": "pacman",
            "repository_provenance": "probe-required",
            "executables": executables,
        },
        indent=2,
    ) + "\n"


def render_desktop(version: str, sha256: str, output: Path) -> None:
    version = normalized_version(version)
    artifact = "kassiber-linux-x64.AppImage"
    artifact_sha256 = validated_sha256(sha256, artifact)
    output.mkdir(parents=True, exist_ok=True)

    _write(
        output / "kassiber",
        "#!/bin/sh\n"
        "export APPIMAGE_EXTRACT_AND_RUN=1\n"
        'exec /opt/kassiber/kassiber.AppImage --cli "$@"\n',
        executable=True,
    )
    _write(
        output / "kassiber-ui",
        "#!/bin/sh\n"
        "export APPIMAGE_EXTRACT_AND_RUN=1\n"
        'exec /opt/kassiber/kassiber.AppImage "$@"\n',
        executable=True,
    )
    _write(
        output / "kassiber.desktop",
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Kassiber\n"
        "Comment=Local-first Bitcoin accounting suite\n"
        "Exec=kassiber-ui\n"
        "Icon=kassiber-ui\n"
        "Terminal=false\n"
        "Categories=Office;Finance;\n",
    )
    _write(
        output / "install-context.json",
        _marker("desktop", "kassiber-bin", ["/usr/bin/kassiber-ui", "/usr/bin/kassiber"]),
    )
    shutil.copyfile(ROOT / "ui-tauri/src-tauri/icons/128x128.png", output / "kassiber.png")
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")

    local_sources = [
        "kassiber",
        "kassiber-ui",
        "kassiber.desktop",
        "kassiber.png",
        "install-context.json",
        "LICENSE",
    ]
    local_hashes = [_sha256(output / name) for name in local_sources]
    renamed_artifact = f"kassiber-bin-{version}.AppImage"
    artifact_url = f"{RELEASE_URL_BASE}/v{version}/{artifact}"
    source_lines = "\n".join(
        [f"        '{name}'" for name in local_sources]
        + [f"        '{renamed_artifact}::{artifact_url}'"]
    )
    hash_lines = "\n".join(
        [f"          '{value}'" for value in local_hashes]
        + [f"          '{artifact_sha256}'"]
    )
    pkgbuild = f"""pkgname=kassiber-bin
pkgver={version}
pkgrel=1
pkgdesc='Local-first Bitcoin accounting desktop application'
arch=('x86_64')
url='https://github.com/bitcoinaustria/kassiber'
license=('AGPL-3.0-only')
depends=('glibc' 'zlib' 'gtk3' 'webkit2gtk-4.1')
provides=('kassiber' 'kassiber-command')
conflicts=('kassiber' 'kassiber-cli' 'kassiber-cli-bin')
options=('!strip')
source=(
{source_lines}
)
sha256sums=(
{hash_lines}
)

package() {{
  install -Dpm755 "${{srcdir}}/{renamed_artifact}" \
    "${{pkgdir}}/opt/kassiber/kassiber.AppImage"
  install -Dpm755 "${{srcdir}}/kassiber" "${{pkgdir}}/usr/bin/kassiber"
  install -Dpm755 "${{srcdir}}/kassiber-ui" "${{pkgdir}}/usr/bin/kassiber-ui"
  install -Dpm644 "${{srcdir}}/kassiber.desktop" \
    "${{pkgdir}}/usr/share/applications/Kassiber.desktop"
  install -Dpm644 "${{srcdir}}/kassiber.png" \
    "${{pkgdir}}/usr/share/icons/hicolor/128x128/apps/kassiber-ui.png"
  install -Dpm644 "${{srcdir}}/install-context.json" \
    "${{pkgdir}}/usr/lib/kassiber/install-context.json"
  install -Dpm644 "${{srcdir}}/LICENSE" \
    "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE"
}}
"""
    _write(output / "PKGBUILD", pkgbuild)
    source_info = "\n".join(f"\tsource = {name}" for name in local_sources)
    source_info += f"\n\tsource = {renamed_artifact}::{artifact_url}"
    hash_info = "\n".join(f"\tsha256sums = {value}" for value in local_hashes)
    hash_info += f"\n\tsha256sums = {artifact_sha256}"
    _write(
        output / ".SRCINFO",
        f"""pkgbase = kassiber-bin
\tpkgdesc = Local-first Bitcoin accounting desktop application
\tpkgver = {version}
\tpkgrel = 1
\turl = https://github.com/bitcoinaustria/kassiber
\tarch = x86_64
\tlicense = AGPL-3.0-only
\tdepends = glibc
\tdepends = zlib
\tdepends = gtk3
\tdepends = webkit2gtk-4.1
\tprovides = kassiber
\tprovides = kassiber-command
\tconflicts = kassiber
\tconflicts = kassiber-cli
\tconflicts = kassiber-cli-bin
\toptions = !strip
{source_info}
{hash_info}

pkgname = kassiber-bin
""",
    )


def render_cli(version: str, sha256: str, output: Path) -> None:
    version = normalized_version(version)
    artifact = "kassiber-cli-linux-x64.tar.gz"
    artifact_sha256 = validated_sha256(sha256, artifact)
    output.mkdir(parents=True, exist_ok=True)
    _write(
        output / "install-context.json",
        _marker("cli", "kassiber-cli-bin", ["/usr/bin/kassiber"]),
    )
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE")
    local_sources = ["install-context.json", "LICENSE"]
    local_hashes = [_sha256(output / name) for name in local_sources]
    renamed_artifact = f"kassiber-cli-bin-{version}.tar.gz"
    artifact_url = f"{RELEASE_URL_BASE}/v{version}/{artifact}"
    pkgbuild = f"""pkgname=kassiber-cli-bin
pkgver={version}
pkgrel=1
pkgdesc='Local-first Bitcoin accounting CLI'
arch=('x86_64')
url='https://github.com/bitcoinaustria/kassiber'
license=('AGPL-3.0-only')
depends=('glibc' 'zlib')
provides=('kassiber-cli' 'kassiber-command')
conflicts=('kassiber' 'kassiber-bin' 'kassiber-cli')
options=('!strip')
source=('install-context.json'
        'LICENSE'
        '{renamed_artifact}::{artifact_url}')
sha256sums=('{local_hashes[0]}'
            '{local_hashes[1]}'
            '{artifact_sha256}')

package() {{
  install -Dpm755 "${{srcdir}}/kassiber-cli-linux-x64/kassiber" \
    "${{pkgdir}}/usr/bin/kassiber"
  install -Dpm644 "${{srcdir}}/install-context.json" \
    "${{pkgdir}}/usr/lib/kassiber/install-context.json"
  install -Dpm644 "${{srcdir}}/LICENSE" \
    "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE"
}}
"""
    _write(output / "PKGBUILD", pkgbuild)
    _write(
        output / ".SRCINFO",
        f"""pkgbase = kassiber-cli-bin
\tpkgdesc = Local-first Bitcoin accounting CLI
\tpkgver = {version}
\tpkgrel = 1
\turl = https://github.com/bitcoinaustria/kassiber
\tarch = x86_64
\tlicense = AGPL-3.0-only
\tdepends = glibc
\tdepends = zlib
\tprovides = kassiber-cli
\tprovides = kassiber-command
\tconflicts = kassiber
\tconflicts = kassiber-bin
\tconflicts = kassiber-cli
\toptions = !strip
\tsource = install-context.json
\tsource = LICENSE
\tsource = {renamed_artifact}::{artifact_url}
\tsha256sums = {local_hashes[0]}
\tsha256sums = {local_hashes[1]}
\tsha256sums = {artifact_sha256}

pkgname = kassiber-cli-bin
""",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="surface", required=True)
    for surface in ("desktop", "cli"):
        command = subparsers.add_parser(surface)
        command.add_argument("--version", required=True)
        command.add_argument("--sha256", required=True)
        command.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.surface == "desktop":
        render_desktop(args.version, args.sha256, args.output)
    else:
        render_cli(args.version, args.sha256, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
