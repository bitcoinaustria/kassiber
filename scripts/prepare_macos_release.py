#!/usr/bin/env python3
"""Download an exact successful CI build, sign locally, optionally hand off to CI."""
from __future__ import annotations

import argparse
import json
import re
import subprocess
from pathlib import Path

from macos_release import APP_ZIP, INPUT_DMG, run, sha256, sign

REPO = "bitcoinaustria/kassiber"


def prepare(args: argparse.Namespace) -> None:
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:[.-][A-Za-z0-9.-]+)?", args.tag):
        raise ValueError("Invalid release tag")
    if not re.fullmatch(r"[0-9]+", args.run_id):
        raise ValueError("Invalid build run ID")
    release = json.loads(run("gh", "release", "view", args.tag, "--repo", REPO,
                             "--json", "isDraft,assets"))
    if not release["isDraft"] or any(a["name"].endswith(".asc") for a in release["assets"]):
        raise ValueError("Require an unsigned draft release")
    build = json.loads(run("gh", "api", f"repos/{REPO}/actions/runs/{args.run_id}"))
    commit = json.loads(run("gh", "api", f"repos/{REPO}/commits/{args.tag}"))["sha"]
    if (build["conclusion"] != "success" or build["head_sha"] != commit
            or build["event"] not in ("push", "workflow_dispatch")
            or build["path"] != ".github/workflows/prerelease-binaries.yml"
            or build["head_repository"]["full_name"] != REPO):
        raise ValueError("Run must be a successful official release build of the exact tag commit")
    args.work_dir.mkdir(parents=True, exist_ok=False)
    download = args.work_dir / "download"
    run("gh", "run", "download", args.run_id, "--repo", REPO,
        "--name", "kassiber-desktop-macos-arm64-preview", "--dir", download)
    archive = download / APP_ZIP
    digest = sha256(archive)
    receipt = {"tag": args.tag, "commit": commit, "build_run": args.run_id,
               "build_attempt": build["run_attempt"], "unsigned_app_sha256": digest}
    (args.work_dir / "source.json").write_text(json.dumps(receipt, indent=2) + "\n")
    sign(argparse.Namespace(archive=archive, sha256=digest, identity=args.identity,
                            commit=commit, version=args.tag[1:], output=args.work_dir / "signed",
                            source=args.work_dir / "source.json",
                            provisioning_profile=args.provisioning_profile))
    if args.submit:
        image = args.work_dir / "signed" / INPUT_DMG
        # No clobber: changing an existing handoff requires explicit operator recovery.
        run("gh", "release", "upload", args.tag, image, "--repo", REPO)
        run("gh", "workflow", "run", "notarize-macos.yml", "--repo", REPO, "--ref", "main",
            "-f", f"tag_name={args.tag}", "-f", f"input_sha256={sha256(image)}")
        print("Notarization dispatched. Release remains a draft pending offline manifest signing.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--identity", required=True, help="Local certificate SHA-1 fingerprint")
    parser.add_argument("--provisioning-profile", required=True, type=Path)
    parser.add_argument("--work-dir", type=Path, required=True, help="New directory outside repository")
    parser.add_argument("--submit", action="store_true", help="Upload signed image and start notarization")
    args = parser.parse_args()
    try:
        prepare(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"Release preparation failed ({type(exc).__name__}); no release published.\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
