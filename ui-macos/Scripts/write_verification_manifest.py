#!/usr/bin/env python3
"""Validate native captures and bind them to an exact packaged app artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path


EXPECTED_PRODUCT = "kassiber_native"
EXPECTED_BUNDLE_ID = "at.bitcoinaustria.kassiber.native"
CAPTURE_BACKEND = "AppKit.NSView.cacheDisplay-own-window-detail"

CAPTURES = [
    ("dashboard-en.png", "dashboard", "en"),
    ("transactions-en.png", "transactions", "en"),
    ("wallets-en.png", "wallets", "en"),
    ("reports-en.png", "reports", "en"),
    ("journals-en.png", "journals", "en"),
    ("quarantine-en.png", "quarantine", "en"),
    ("swaps-en.png", "swaps", "en"),
    ("reconcile-en.png", "reconcile", "en"),
    ("books-en.png", "books", "en"),
    ("connections-en.png", "connections", "en"),
    ("imports-en.png", "imports", "en"),
    ("exit-tax-en.png", "exitTax", "en"),
    ("source-funds-en.png", "sourceFunds", "en"),
    ("activity-en.png", "activity", "en"),
    ("privacy-mirror-en.png", "privacyMirror", "en"),
    ("birds-eye-en.png", "birdsEye", "en"),
    ("egress-en.png", "egress", "en"),
    ("logs-en.png", "logs", "en"),
    ("settings-en.png", "settings", "en"),
    ("assistant-en.png", "assistant", "en"),
    ("foundation-en.png", "onboarding", "en"),
    ("dashboard-de.png", "dashboard", "de-AT"),
    ("transactions-de.png", "transactions", "de-AT"),
    ("assistant-de.png", "assistant", "de-AT"),
]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_png(path: Path) -> tuple[int, int]:
    data = path.read_bytes()
    if len(data) < 50_000 or not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise SystemExit(f"capture is missing, truncated, or not PNG: {path}")
    offset = 8
    idat = bytearray()
    width = height = bit_depth = color_type = interlace = 0
    saw_ihdr = saw_iend = False
    while offset + 12 <= len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        kind = data[offset + 4 : offset + 8]
        chunk_end = offset + 12 + length
        if chunk_end > len(data):
            raise SystemExit(f"capture PNG contains a truncated {kind!r} chunk: {path}")
        payload = data[offset + 8 : offset + 8 + length]
        expected_crc = struct.unpack(">I", data[offset + 8 + length : chunk_end])[0]
        actual_crc = zlib.crc32(kind)
        actual_crc = zlib.crc32(payload, actual_crc) & 0xFFFFFFFF
        if actual_crc != expected_crc:
            raise SystemExit(f"capture PNG contains a corrupt {kind!r} chunk: {path}")
        offset = chunk_end
        if kind == b"IHDR":
            if saw_ihdr or length != 13:
                raise SystemExit(f"capture PNG contains an invalid IHDR chunk: {path}")
            (
                width,
                height,
                bit_depth,
                color_type,
                compression,
                filter_method,
                interlace,
            ) = struct.unpack(">IIBBBBB", payload[:13])
            saw_ihdr = True
            if compression != 0 or filter_method != 0:
                raise SystemExit(f"capture PNG uses unsupported compression or filtering: {path}")
        elif kind == b"IDAT":
            if not saw_ihdr:
                raise SystemExit(f"capture PNG image data precedes IHDR: {path}")
            idat.extend(payload)
        elif kind == b"IEND":
            if length != 0:
                raise SystemExit(f"capture PNG contains an invalid IEND chunk: {path}")
            saw_iend = True
            break
    if not saw_ihdr or not saw_iend:
        raise SystemExit(f"capture PNG is missing IHDR or IEND: {path}")
    if (
        width < 900
        or height < 600
        or bit_depth != 8
        or color_type not in (2, 6)
        or interlace != 0
    ):
        raise SystemExit(
            f"capture has an unsupported or undersized raster: {path} "
            f"({width}x{height}, depth={bit_depth}, type={color_type})"
        )
    if not idat:
        raise SystemExit(f"capture PNG contains no image payload: {path}")
    if sampled_color_count(bytes(idat), width, height, color_type) < 12:
        raise SystemExit(f"capture is blank or near-uniform: {path}")
    return width, height


def paeth(left: int, above: int, upper_left: int) -> int:
    estimate = left + above - upper_left
    left_distance = abs(estimate - left)
    above_distance = abs(estimate - above)
    diagonal_distance = abs(estimate - upper_left)
    if left_distance <= above_distance and left_distance <= diagonal_distance:
        return left
    if above_distance <= diagonal_distance:
        return above
    return upper_left


def sampled_color_count(idat: bytes, width: int, height: int, color_type: int) -> int:
    """Decode enough of an 8-bit RGB/RGBA PNG to reject blank evidence.

    RGBA samples are composited over white before counting. Hidden RGB values
    in fully transparent pixels therefore cannot make an invisible capture
    look visually diverse to the validator.
    """

    channels = 3 if color_type == 2 else 4
    stride = width * channels
    try:
        raw = zlib.decompress(idat)
    except zlib.error as error:
        raise SystemExit(f"capture PNG image payload is corrupt: {error}") from error
    if len(raw) != (stride + 1) * height:
        raise SystemExit("capture PNG image payload has an unexpected size")
    prior = bytearray(stride)
    colors: set[tuple[int, int, int]] = set()
    # The native onboarding receipt is intentionally airy: most pixels are
    # white and the useful controls occupy a narrow centered column. Sample a
    # denser grid so sparse-but-real UI content is not mistaken for a blank
    # backing store merely because a coarse grid lands between text strokes.
    sample_x = max(1, width // 160)
    sample_y = max(1, height // 100)
    cursor = 0
    for row_index in range(height):
        filter_kind = raw[cursor]
        cursor += 1
        row = bytearray(raw[cursor : cursor + stride])
        cursor += stride
        if filter_kind == 1:
            for index in range(stride):
                left = row[index - channels] if index >= channels else 0
                row[index] = (row[index] + left) & 0xFF
        elif filter_kind == 2:
            for index in range(stride):
                row[index] = (row[index] + prior[index]) & 0xFF
        elif filter_kind == 3:
            for index in range(stride):
                left = row[index - channels] if index >= channels else 0
                row[index] = (row[index] + ((left + prior[index]) // 2)) & 0xFF
        elif filter_kind == 4:
            for index in range(stride):
                left = row[index - channels] if index >= channels else 0
                upper_left = prior[index - channels] if index >= channels else 0
                row[index] = (row[index] + paeth(left, prior[index], upper_left)) & 0xFF
        elif filter_kind != 0:
            raise SystemExit(f"capture PNG uses unsupported row filter {filter_kind}")
        if row_index % sample_y == 0:
            for column in range(0, width, sample_x):
                offset = column * channels
                red, green, blue = row[offset], row[offset + 1], row[offset + 2]
                if channels == 4:
                    alpha = row[offset + 3]
                    red = (red * alpha + 255 * (255 - alpha) + 127) // 255
                    green = (green * alpha + 255 * (255 - alpha) + 127) // 255
                    blue = (blue * alpha + 255 * (255 - alpha) + 127) // 255
                colors.add((red, green, blue))
                if len(colors) >= 64:
                    return len(colors)
        prior = row
    return len(colors)


def architecture(path: Path) -> str:
    try:
        return subprocess.check_output(["lipo", "-archs", str(path)], text=True).strip()
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"cannot read Mach-O architecture for {path}") from error


def artifact(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise SystemExit(f"packaged artifact is missing: {path}")
    arch = architecture(path)
    if arch != "arm64":
        raise SystemExit(f"packaged Mach-O must be arm64-only: {path} ({arch})")
    return {
        "path": path.name,
        "sha256": sha256(path),
        "architecture": arch,
    }


def bundle_tree_sha256(app: Path) -> str:
    """Hash every file and symlink path in an app bundle deterministically."""

    digest = hashlib.sha256()
    entries = sorted(app.rglob("*"), key=lambda item: item.relative_to(app).as_posix())
    for entry in entries:
        relative = entry.relative_to(app).as_posix().encode("utf-8")
        if entry.is_symlink():
            digest.update(b"L\0" + relative + b"\0" + os.readlink(entry).encode("utf-8") + b"\0")
        elif entry.is_file():
            digest.update(b"F\0" + relative + b"\0")
            with entry.open("rb") as handle:
                for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                    digest.update(chunk)
            digest.update(b"\0")
        elif entry.is_dir():
            digest.update(b"D\0" + relative + b"\0")
    return digest.hexdigest()


def verify_codesign(app: Path) -> None:
    result = subprocess.run(
        ["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app)],
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        raise SystemExit(f"packaged app signature is invalid: {detail}")


def verify_macho_tree(contents: Path) -> int:
    count = 0
    for candidate in sorted(contents.rglob("*")):
        if not candidate.is_file() or candidate.is_symlink():
            continue
        description = subprocess.check_output(["file", "-b", str(candidate)], text=True)
        if "Mach-O" not in description:
            continue
        count += 1
        arch = architecture(candidate)
        if arch != "arm64":
            raise SystemExit(f"app bundle contains a non-arm64 Mach-O: {candidate} ({arch})")
    if count < 3:
        raise SystemExit("app bundle contains an implausibly small Mach-O tree")
    return count


def verify_bundle(executable: Path, info: dict[str, object]) -> tuple[Path, str, int]:
    contents = executable.parent.parent
    app = contents.parent
    if app.suffix != ".app" or executable.name != EXPECTED_PRODUCT:
        raise SystemExit("verification requires kassiber_native.app/Contents/MacOS/kassiber_native")
    expected_info = {
        "CFBundleDisplayName": EXPECTED_PRODUCT,
        "CFBundleName": EXPECTED_PRODUCT,
        "CFBundleExecutable": EXPECTED_PRODUCT,
        "CFBundleIdentifier": EXPECTED_BUNDLE_ID,
    }
    for key, expected in expected_info.items():
        if info.get(key) != expected:
            raise SystemExit(f"packaged app has unexpected {key}: {info.get(key)!r}")
    for key in ("CFBundleShortVersionString", "CFBundleVersion", "KassiberBuildCommit"):
        if not str(info.get(key, "")).strip():
            raise SystemExit(f"packaged app is missing {key}")
    if info.get("KassiberSigningIdentityStrength") not in ("adhoc", "production"):
        raise SystemExit("packaged app is missing its signing-strength provenance")

    required = [
        contents / "Info.plist",
        executable,
        contents / "Resources" / "kassiber-sidecar",
        contents / "Resources" / "AppIcon.icns",
        contents / "Resources" / "Kassiber-LICENSE",
        contents / "Resources" / "ThirdPartyLicenses" / "Kassiber-Third-Party-Licenses.md",
        contents / "Frameworks" / "Sparkle.framework",
    ]
    missing = [str(path) for path in required if not path.exists()]
    if missing:
        raise SystemExit("packaged app is incomplete: " + ", ".join(missing))

    repo_root = Path(__file__).resolve().parents[2]
    tauri_icon = repo_root / "ui-tauri" / "src-tauri" / "icons" / "icon.icns"
    packaged_icon = contents / "Resources" / "AppIcon.icns"
    if packaged_icon.read_bytes() != tauri_icon.read_bytes():
        raise SystemExit("packaged icon is not byte-identical to the Tauri icon.icns")
    packaged_pngs = list((contents / "Resources").rglob("AppIcon-1024.png"))
    tauri_png = repo_root / "ui-tauri" / "src-tauri" / "icons" / "icon.png"
    if len(packaged_pngs) != 1 or packaged_pngs[0].read_bytes() != tauri_png.read_bytes():
        raise SystemExit("packaged runtime PNG is not byte-identical to the Tauri icon.png")

    verify_codesign(app)
    macho_count = verify_macho_tree(contents)
    return app, bundle_tree_sha256(app), macho_count


def verify_zip(app: Path, expected_tree_sha256: str) -> dict[str, object]:
    archive = app.parent / f"{EXPECTED_PRODUCT}-macos-arm64.zip"
    if not archive.is_file():
        raise SystemExit(f"packaged ZIP is missing: {archive}")
    with tempfile.TemporaryDirectory(prefix="kassiber-native-manifest-") as directory:
        result = subprocess.run(
            ["ditto", "-x", "-k", str(archive), directory],
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            raise SystemExit(f"packaged ZIP cannot be extracted: {result.stderr.strip()}")
        apps = list(Path(directory).glob("*.app"))
        if len(apps) != 1 or apps[0].name != app.name:
            raise SystemExit("packaged ZIP must contain exactly kassiber_native.app")
        extracted_hash = bundle_tree_sha256(apps[0])
        if extracted_hash != expected_tree_sha256:
            raise SystemExit("packaged ZIP does not contain the exact verified app bundle")
        verify_codesign(apps[0])
    return {"path": archive.name, "sha256": sha256(archive), "bytes": archive.stat().st_size}


def validate_receipt(
    receipt_path: Path,
    capture_path: Path,
    executable: Path,
    info: dict[str, object],
    screen: str,
    language: str,
    width: int,
    height: int,
) -> dict[str, object]:
    if not receipt_path.is_file():
        raise SystemExit(f"native app-rendered capture receipt is missing: {receipt_path}")
    try:
        receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"native capture receipt is invalid: {receipt_path}") from error
    expected = {
        "schema_version": 1,
        "backend": CAPTURE_BACKEND,
        "file": capture_path.name,
        "screen": screen,
        "language": language,
        "width": width,
        "height": height,
        "byte_count": capture_path.stat().st_size,
        "sha256": sha256(capture_path),
        "executable_sha256": sha256(executable),
        "product": EXPECTED_PRODUCT,
        "bundle_id": EXPECTED_BUNDLE_ID,
        "version": info.get("CFBundleShortVersionString"),
        "build": info.get("CFBundleVersion"),
        "commit": info.get("KassiberBuildCommit"),
    }
    for key, expected_value in expected.items():
        if receipt.get(key) != expected_value:
            raise SystemExit(
                f"native capture receipt mismatch for {capture_path.name}: "
                f"{key}={receipt.get(key)!r}, expected {expected_value!r}"
            )
    if not str(receipt.get("captured_at", "")).strip():
        raise SystemExit(f"native capture receipt has no timestamp: {receipt_path}")
    return {
        "file": receipt_path.relative_to(receipt_path.parent.parent).as_posix(),
        "sha256": sha256(receipt_path),
        "captured_at": receipt["captured_at"],
    }


def write_manifest(executable: Path, verification_dir: Path, output: Path) -> None:
    contents = executable.parent.parent
    info_path = contents / "Info.plist"
    if not info_path.is_file():
        raise SystemExit("verification requires a packaged app executable under Contents/MacOS")
    with info_path.open("rb") as handle:
        info = plistlib.load(handle)
    app, app_tree_sha256, macho_count = verify_bundle(executable, info)
    captures = []
    for filename, screen, language in CAPTURES:
        path = verification_dir / filename
        width, height = validate_png(path)
        receipt = validate_receipt(
            verification_dir / "capture-receipts" / f"{filename}.capture.json",
            path,
            executable,
            info,
            screen,
            language,
            width,
            height,
        )
        captures.append(
            {
                "file": filename,
                "screen": screen,
                "language": language,
                "width": width,
                "height": height,
                "sha256": sha256(path),
                "receipt": receipt,
            }
        )
    capture_hashes = [capture["sha256"] for capture in captures]
    if len(set(capture_hashes)) != len(capture_hashes):
        raise SystemExit("verification corpus contains duplicate screen images")
    manifest = {
        "schema_version": 2,
        "capture_backend": CAPTURE_BACKEND,
        "product": info.get("CFBundleDisplayName"),
        "bundle_id": info.get("CFBundleIdentifier"),
        "version": info.get("CFBundleShortVersionString"),
        "build": info.get("CFBundleVersion"),
        "commit": info.get("KassiberBuildCommit"),
        "dirty": bool(info.get("KassiberBuildDirty", False)),
        "signing": info.get("KassiberSigningIdentityStrength"),
        "executable": artifact(executable),
        "sidecar": artifact(contents / "Resources" / "kassiber-sidecar"),
        "icon_sha256": sha256(contents / "Resources" / "AppIcon.icns"),
        "app_bundle": {
            "path": app.name,
            "tree_sha256": app_tree_sha256,
            "macho_count": macho_count,
        },
        "archive": verify_zip(app, app_tree_sha256),
        "captures": captures,
    }
    output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote verification manifest for {len(captures)} packaged-app captures")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", type=Path)
    parser.add_argument("--app-executable", type=Path)
    parser.add_argument("--verification-dir", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.verify:
        width, height = validate_png(args.verify)
        print(f"verified native capture {args.verify.name}: {width}x{height}")
        return
    if not (args.app_executable and args.verification_dir and args.output):
        parser.error("manifest mode requires --app-executable, --verification-dir, and --output")
    write_manifest(args.app_executable, args.verification_dir, args.output)


if __name__ == "__main__":
    main()
