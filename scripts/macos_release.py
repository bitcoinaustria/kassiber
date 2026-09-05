#!/usr/bin/env python3
"""Local Developer ID signing and keyless CI notarization of one sealed app.

No package code is executed while signing. Secrets are read by Apple's tools
from the local keychain, never accepted as command-line passwords here.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import plistlib
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import zipfile
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath

TEAM = "6Q4R2C3GJK"
APP_ID = "at.bitcoinaustria.kassiber"
APP_ZIP = "kassiber-macos-arm64.app.zip"
DMG = "kassiber-macos-arm64.dmg"
CLI_TAR = "kassiber-cli-macos-arm64.tar.gz"
INPUT_DMG = "kassiber-macos-signing-input.dmg"
APP_ENTITLEMENTS = {
    "com.apple.application-identifier": f"{TEAM}.{APP_ID}",
    "com.apple.developer.team-identifier": TEAM,
    "keychain-access-groups": [f"{TEAM}.{APP_ID}"],
}
MACHO = {bytes.fromhex(x) for x in (
    "feedface", "cefaedfe", "feedfacf", "cffaedfe", "cafebabe", "bebafeca",
    "cafebabf", "bfbafeca",
)}


def run(*args: str | Path) -> str:
    return subprocess.run([str(a) for a in args], check=True, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE).stdout


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def check_digest(path: Path, expected: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{64}", expected) or sha256(path) != expected:
        raise ValueError("Artifact SHA-256 mismatch")


def validate_profile_data(data: dict) -> None:
    """Check the intended narrow contract; macOS validates Apple's authorization."""
    entitlements = data.get("Entitlements", {})
    expires = data.get("ExpirationDate")
    if (data.get("TeamIdentifier") != [TEAM] or data.get("ProvisionsAllDevices") is not True
            or not isinstance(expires, datetime)
            or expires.replace(tzinfo=timezone.utc) <= datetime.now(timezone.utc)):
        raise ValueError("Require an unexpired Bitcoin Austria Developer ID distribution profile")
    for key in ("com.apple.application-identifier", "com.apple.developer.team-identifier"):
        if entitlements.get(key) != APP_ENTITLEMENTS[key]:
            raise ValueError("Provisioning profile does not authorize the exact production app")
    groups = entitlements.get("keychain-access-groups", [])
    if f"{TEAM}.{APP_ID}" not in groups and f"{TEAM}.*" not in groups:
        raise ValueError("Provisioning profile does not authorize the app Keychain group")


def load_profile(path: Path) -> dict:
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 1024 * 1024:
        raise ValueError("Invalid provisioning profile file")
    data = plistlib.loads(run("/usr/bin/security", "cms", "-D", "-i", path).encode())
    validate_profile_data(data)
    return data


def verify_entitlements(path: Path, expected: dict) -> None:
    xml = run("/usr/bin/codesign", "-d", "--entitlements", "-", "--xml", path)
    actual = plistlib.loads(xml.encode()) if xml.strip() else {}
    if actual != expected:
        raise ValueError("Unexpected code entitlements (no debug/runtime exemptions allowed)")


def verify_profile_certificate(app: Path, profile: dict) -> None:
    with tempfile.TemporaryDirectory(prefix="kassiber-public-cert-") as tmp:
        prefix = Path(tmp) / "cert"
        run("/usr/bin/codesign", "-d", "--extract-certificates", prefix, app)
        leaf = prefix.with_name("cert0").read_bytes()
        if leaf not in profile.get("DeveloperCertificates", []):
            raise ValueError("App signing certificate is not authorized by the profile")


def extract_app(archive: Path, destination: Path) -> Path:
    # Validate before ditto: no traversal, links, duplicate names or special
    # files. Our Tauri onedir build intentionally contains no symlinks.
    with zipfile.ZipFile(archive) as zf:
        seen: set[str] = set()
        total = 0
        for item in zf.infolist():
            path = PurePosixPath(item.filename)
            mode = item.external_attr >> 16
            if (path.is_absolute() or ".." in path.parts or "\\" in item.filename
                    or not path.parts or path.parts[0] != "Kassiber.app"
                    or item.filename in seen
                    or stat.S_IFMT(mode) not in (0, stat.S_IFREG, stat.S_IFDIR)):
                raise ValueError("Unsafe or unexpected app archive member")
            seen.add(item.filename)
            total += item.file_size
            if total > 4 * 1024**3 or len(seen) > 100000:
                raise ValueError("App archive exceeds extraction limits")
    run("/usr/bin/ditto", "-x", "-k", archive, destination)
    app = destination / "Kassiber.app"
    if not app.is_dir():
        raise ValueError("Missing Kassiber.app")
    return app


def validate_app(app: Path, commit: str, version: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Expected full source commit")
    with (app / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    if info.get("CFBundleIdentifier") != APP_ID:
        raise ValueError("Unexpected app identity (dev builds cannot be released)")
    if info.get("CFBundleShortVersionString") != version:
        raise ValueError("App version does not match release tag")
    metadata = list(app.rglob("BUILD_INFO.json"))
    if not metadata:
        raise ValueError("Missing embedded build provenance")
    for path in metadata:
        data = json.loads(path.read_text())
        if data.get("commit") != commit or data.get("version") != version:
            raise ValueError("Embedded build provenance does not match release")
        if data.get("channel") not in ("release", "prerelease"):
            raise ValueError("Development artifact cannot be released")


def code_files(app: Path) -> list[Path]:
    result = []
    for path in sorted(app.rglob("*")):
        if path.is_symlink():
            raise ValueError("Unexpected symlink in sealed app")
        if path.is_file():
            with path.open("rb") as stream:
                if stream.read(4) in MACHO:
                    result.append(path)
    if not result:
        raise ValueError("No Mach-O code in app")
    return result


def verify_code(path: Path, *, runtime: bool = True) -> None:
    requirement = (f'anchor apple generic and certificate leaf[subject.OU] = "{TEAM}" '
                   'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists')
    run("/usr/bin/codesign", "--verify", "--strict", "-R", requirement, path)
    result = subprocess.run(["/usr/bin/codesign", "-d", "--verbose=4", str(path)],
                            check=True, capture_output=True, text=True)
    details = result.stderr
    if "Timestamp=" not in details or (runtime and "runtime" not in details):
        raise ValueError("Missing secure timestamp or hardened runtime")


def verify_app(app: Path, commit: str, version: str, *, ticket: bool) -> None:
    validate_app(app, commit, version)
    profile = load_profile(app / "Contents/embedded.provisionprofile")
    verify_profile_certificate(app, profile)
    source = json.loads((app / "Contents/Resources/RELEASE_SOURCE.json").read_text())
    if (source.get("commit") != commit or source.get("tag") != "v" + version
            or not re.fullmatch(r"[0-9]+", str(source.get("build_run", "")))
            or not isinstance(source.get("build_attempt"), int)):
        raise ValueError("Missing or mismatched signed source provenance")
    for path in code_files(app):
        verify_code(path)
        expected = APP_ENTITLEMENTS if path == app / "Contents/MacOS/kassiber-ui" else {}
        verify_entitlements(path, expected)
    verify_code(app)
    verify_entitlements(app, APP_ENTITLEMENTS)
    run("/usr/bin/codesign", "--verify", "--deep", "--strict", app)
    if ticket:
        run("/usr/bin/xcrun", "stapler", "validate", app)
        run("/usr/sbin/spctl", "--assess", "--type", "execute", "--verbose=4", app)
        # CMS decoding above is not treated as Apple's authorization verdict.
        # Let the OS validate the complete provisioned distribution too.
        run("/usr/bin/syspolicy_check", "distribution", app)


def sign(args: argparse.Namespace) -> None:
    if os.environ.get("CI"):
        raise ValueError("Signing must run locally, not in CI")
    if not re.fullmatch(r"[0-9A-Fa-f]{40}", args.identity):
        raise ValueError("Select an exact keychain certificate SHA-1 fingerprint")
    check_digest(args.archive, args.sha256)
    profile = load_profile(args.provisioning_profile)
    if args.identity.upper() not in {
        hashlib.sha1(cert).hexdigest().upper() for cert in profile.get("DeveloperCertificates", [])
    }:
        raise ValueError("Selected signing identity is not authorized by the provisioning profile")
    args.output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="kassiber-sign-") as tmp:
        stage = Path(tmp) / "payload"
        stage.mkdir()
        app = extract_app(args.archive.resolve(), stage)
        validate_app(app, args.commit, args.version)
        shutil.copy2(args.provisioning_profile, app / "Contents/embedded.provisionprofile")
        entitlements = Path(tmp) / "entitlements.plist"
        entitlements.write_bytes(plistlib.dumps(APP_ENTITLEMENTS))
        source = json.loads(args.source.read_text())
        if (source.get("commit") != args.commit or source.get("tag") != "v" + args.version
                or source.get("unsigned_app_sha256") != args.sha256):
            raise ValueError("Signing provenance does not match input")
        (app / "Contents/Resources/RELEASE_SOURCE.json").write_text(
            json.dumps(source, sort_keys=True, indent=2) + "\n")
        # Explicit inner-to-outer signing. Never --deep sign or disable library
        # validation: every Python extension/library receives the same Team ID.
        for path in code_files(app):
            run("/usr/bin/codesign", "--force", "--sign", args.identity,
                "--timestamp", "--options", "runtime", "--identifier", path.name, path)
        run("/usr/bin/codesign", "--force", "--sign", args.identity,
            "--timestamp", "--options", "runtime", "--identifier", APP_ID,
            "--entitlements", entitlements, app)
        verify_app(app, args.commit, args.version, ticket=False)
        (stage / "Applications").symlink_to("/Applications")
        image = args.output.resolve() / INPUT_DMG
        run("/usr/bin/hdiutil", "create", "-volname", "Kassiber", "-srcfolder", stage,
            "-format", "UDZO", image)
        run("/usr/bin/codesign", "--sign", args.identity, "--timestamp", image)
        verify_code(image, runtime=False)
        print(json.dumps({"input": str(image), "sha256": sha256(image),
                          "commit": args.commit, "version": args.version}, indent=2))


def mount_image(image: Path, mount: Path) -> None:
    run("/usr/bin/hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen",
        "-mountpoint", mount, image)


def app_from_image(image: Path, stage: Path) -> Path:
    mount = stage / "mount"
    mount.mkdir()
    mount_image(image, mount)
    try:
        names = {p.name for p in mount.iterdir()}
        if names - {"Kassiber.app", "Applications", ".DS_Store", ".Trashes", ".fseventsd"}:
            raise ValueError("Unexpected disk image contents")
        app = stage / "Kassiber.app"
        run("/usr/bin/ditto", mount / "Kassiber.app", app)
        return app
    finally:
        run("/usr/bin/hdiutil", "detach", mount)


def notarize(args: argparse.Namespace) -> None:
    check_digest(args.image, args.sha256)
    verify_code(args.image, runtime=False)
    args.output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="kassiber-notary-") as tmp:
        app = app_from_image(args.image.resolve(), Path(tmp))
        verify_app(app, args.commit, args.version, ticket=False)
        # Apple creates tickets for both this signed image and its inner app.
        result = json.loads(run("/usr/bin/xcrun", "notarytool", "submit", args.image,
                                "--keychain-profile", args.profile, "--wait",
                                "--keychain", args.keychain,
                                "--timeout", "45m", "--output-format", "json"))
        (args.output / "notarization.json").write_text(json.dumps(result, indent=2) + "\n")
        if result.get("status") != "Accepted":
            raise ValueError("Notarization was not Accepted; no release files are ready")
        image = args.output / DMG
        shutil.copy2(args.image, image)
        for path in (image, app):
            run("/usr/bin/xcrun", "stapler", "staple", path)
            run("/usr/bin/xcrun", "stapler", "validate", path)
        verify_app(app, args.commit, args.version, ticket=True)
        verify_code(image, runtime=False)
        run("/usr/sbin/spctl", "--assess", "--type", "open", "--context",
            "context:primary-signature", "--verbose=4", image)
        run("/usr/bin/ditto", "-c", "-k", "--keepParent", app, args.output / APP_ZIP)
        # A single notarized app supplies the CLI too: PyInstaller onefile
        # embedded libraries cannot be Developer-ID-signed after the CI build.
        cli_root = Path(tmp) / "kassiber-cli-macos-arm64"
        cli_root.mkdir()
        shutil.copytree(app, cli_root / "Kassiber.app")
        (cli_root / "kassiber").symlink_to("Kassiber.app/Contents/Resources/bin/kassiber")
        with tarfile.open(args.output / CLI_TAR, "w:gz") as archive:
            archive.add(cli_root, arcname=cli_root.name)


def verify_cli_archive(archive_path: Path, app: Path) -> None:
    expected = {str(p.relative_to(app)): p for p in app.rglob("*") if p.is_file()}
    with tarfile.open(archive_path) as archive:
        prefix = "kassiber-cli-macos-arm64/"
        actual: set[str] = set()
        seen: set[str] = set()
        launcher_found = False
        for member in archive:
            if (member.name in seen or not member.name.startswith(prefix.rstrip("/"))
                    or ".." in PurePosixPath(member.name).parts):
                raise ValueError("Duplicate/unsafe CLI archive member")
            seen.add(member.name)
            if len(seen) > 100000:
                raise ValueError("CLI archive exceeds member limit")
            if member.isdir():
                if (member.name not in (prefix.rstrip("/"), prefix + "Kassiber.app")
                        and not member.name.startswith(prefix + "Kassiber.app/")):
                    raise ValueError("Unexpected CLI directory")
                continue
            if member.name == prefix + "kassiber" and member.issym():
                if member.linkname != "Kassiber.app/Contents/Resources/bin/kassiber":
                    raise ValueError("Unexpected CLI launcher")
                launcher_found = True
                continue
            if not member.isfile() or not member.name.startswith(prefix + "Kassiber.app/"):
                raise ValueError("Unexpected CLI archive member")
            name = member.name.removeprefix(prefix + "Kassiber.app/")
            source = expected.get(name)
            if source is None or member.size != source.stat().st_size:
                raise ValueError("CLI member does not match sealed app")
            if member.mode & 0o7777 != source.stat().st_mode & 0o7777:
                raise ValueError("CLI permissions differ from sealed app")
            stream = archive.extractfile(member)
            assert stream is not None
            if hashlib.file_digest(stream, "sha256").hexdigest() != sha256(source):
                raise ValueError("CLI archive and notarized app differ")
            actual.add(name)
        if not launcher_found or actual != set(expected):
            raise ValueError("CLI archive is incomplete")


def verify(args: argparse.Namespace) -> None:
    with tempfile.TemporaryDirectory(prefix="kassiber-verify-") as tmp:
        stage = Path(tmp)
        app = extract_app(args.release_dir / APP_ZIP, stage / "zip")
        verify_app(app, args.commit, args.version, ticket=True)
        image = args.release_dir / DMG
        verify_code(image, runtime=False)
        run("/usr/bin/xcrun", "stapler", "validate", image)
        run("/usr/sbin/spctl", "--assess", "--type", "open", "--context",
            "context:primary-signature", "--verbose=4", image)
        # Compare actual sealed resources, not just outer CDHash, across all
        # distributions. Stapled ticket bytes may differ inside the DMG.
        dmg_stage = stage / "dmg"
        dmg_stage.mkdir()
        dmg_app = app_from_image(image, dmg_stage)
        verify_app(dmg_app, args.commit, args.version, ticket=False)
        def payload(root: Path) -> dict[str, str]:
            return {str(p.relative_to(root)): sha256(p) for p in root.rglob("*")
                    if p.is_file() and str(p.relative_to(root)) != "Contents/CodeResources"}
        if payload(app) != payload(dmg_app):
            raise ValueError("DMG and app ZIP differ")
        verify_cli_archive(args.release_dir / CLI_TAR, app)
        if args.smoke:
            # Only opt in on a disposable verification Mac, after signature
            # and ticket checks. Never execute build downloads on the signer.
            launcher = app / "Contents/Resources/bin/kassiber"
            version_output = run(launcher, "--version")
            if args.commit[:12] not in version_output:
                raise ValueError("Packaged CLI reported a different build")
            run(launcher, "--help")
            # Starting the entitled outer executable also exercises the OS
            # provisioning-profile launch check (the CLI sidecar is unentitled).
            run(app / "Contents/MacOS/kassiber-ui", "--cli", "--version")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    for command in ("sign", "notarize", "verify"):
        p = sub.add_parser(command)
        p.add_argument("--commit", required=True)
        p.add_argument("--version", required=True)
        if command == "verify":
            p.add_argument("--release-dir", type=Path, required=True)
            p.add_argument("--smoke", action="store_true", help="Execute verified CLI on a disposable Mac")
        else:
            p.add_argument("--output", type=Path, required=True)
            p.add_argument("--sha256", required=True)
        if command == "sign":
            p.add_argument("--archive", type=Path, required=True)
            p.add_argument("--identity", required=True)
            p.add_argument("--source", type=Path, required=True)
            p.add_argument("--provisioning-profile", type=Path, required=True)
        if command == "notarize":
            p.add_argument("--image", type=Path, required=True)
            p.add_argument("--profile", required=True)
            p.add_argument("--keychain", type=Path, required=True)
    args = parser.parse_args()
    try:
        {"sign": sign, "notarize": notarize, "verify": verify}[args.command](args)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        # Do not print subprocess arguments: future tools may contain credentials.
        parser.exit(1, f"macOS release {args.command} failed: {type(exc).__name__}: "
                    f"{exc if not isinstance(exc, subprocess.CalledProcessError) else 'platform tool rejected artifact; inspect locally'}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
