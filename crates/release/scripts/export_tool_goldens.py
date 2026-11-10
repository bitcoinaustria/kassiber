"""Freeze what the Python release scripts produce, for the Rust release tool.

Runs scripts/{release_manifest,render_aur,render_nix,render_homebrew,
release_notes,write_build_info}.py on fixed inputs in scratch directories and
writes their exit codes, stdout, stderr and output files to
crates/release/tests/data/tool_goldens.json. crates/release/tests/tool.rs runs
`kassiber-release` on the same inputs and requires the same bytes
(docs/plan/22-python-removal.md, decision D3). Run once, before Python is
removed:

    uv run --locked python crates/release/scripts/export_tool_goldens.py

The signing fixtures use a throwaway Ed25519 key made here; only its public
half, the policy and the signature are kept. The repository root is written
as `{repo}` wherever an output names it.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "crates/release/tests/data/tool_goldens.json"
SCRIPTS = ROOT / "scripts"
A, B, C = "a" * 64, "b" * 64, "c" * 64

CHANGELOG = (
    "# Changelog\n\n## 1.2.30\n\nOther\n\n## 1.2.3\n\n### Fixed\n\n- A & B.\r\n\n"
    "## 1.2.3-rc.1\n\nCandidate\n\n## 1.2.2\n\n\n\n## 1.2.1\n\nOlder\n"
)


def file_record(path: Path) -> dict:
    raw = path.read_bytes()
    record = {"sha256": hashlib.sha256(raw).hexdigest(), "mode": oct(path.stat().st_mode & 0o777)}
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return record
    if len(raw) <= 16384:
        record["text"] = text
    return record


def tree(root: Path) -> dict:
    return {str(p.relative_to(root)): file_record(p) for p in sorted(root.rglob("*")) if p.is_file()}


def make_signing_fixture() -> dict:
    """A throwaway key, its policy files, and a signed manifest."""
    work = Path(tempfile.mkdtemp(prefix="kassiber-tool-goldens-key-"))
    try:
        key = work / "key"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f", str(key)],
                       check=True)
        public = (work / "key.pub").read_text()
        blob = public.split()[1]
        import base64
        fingerprint = "SHA256:" + base64.b64encode(
            hashlib.sha256(base64.b64decode(blob)).digest()).decode().rstrip("=")
        manifest = (
            "# Kassiber release manifest v1\n# Version: 1.2.3\n"
            f"{hashlib.sha256(b'app').hexdigest()}  kassiber-cli-linux-x64.tar.gz\n"
            f"{hashlib.sha256(b'desktop').hexdigest()}  kassiber-linux-x64.AppImage\n"
        )
        (work / "m.txt").write_text(manifest)
        subprocess.run(["ssh-keygen", "-Y", "sign", "-n", "kassiber-release", "-f", str(key), str(work / "m.txt")],
                       check=True, capture_output=True)
        signature = (work / "m.txt.sig").read_text()
        return {"public_key": public, "fingerprint": fingerprint, "manifest": manifest,
                "signature": signature,
                "allowed_signers": f'release@kassiber namespaces="kassiber-release" {public.split()[0]} {blob}\n'}
    finally:
        shutil.rmtree(work)


def cases(signing: dict) -> list[dict]:
    fp = signing["fingerprint"]
    policy = {"schema_version": 2, "enabled": True, "fingerprint": fp, "namespace": "kassiber-release",
              "principal": "release@kassiber", "public_key_path": "release/fixture.pub",
              "allowed_signers_path": "release/allowed_signers"}
    release_files = {"release/kassiber-cli-linux-x64.tar.gz": "app", "release/kassiber-linux-x64.AppImage": "desktop"}
    signed = {**release_files,
              "release/kassiber-1.2.3-manifest.txt": signing["manifest"],
              "release/kassiber-1.2.3-manifest.txt.sig": signing["signature"],
              "keys/fixture.pub": signing["public_key"]}
    verify_release = ["verify-release", "--release-dir", "release", "--manifest",
                      "release/kassiber-1.2.3-manifest.txt", "--signature",
                      "release/kassiber-1.2.3-manifest.txt.sig", "--public-key", "keys/fixture.pub"]
    out = []

    def case(name, script, args, *, rust, setup=None, env=None, stderr=True, repo_files=None):
        out.append({"name": name, "script": script, "args": args, "rust": rust, "setup": setup or {},
                    "repo_files": repo_files or {}, "env": env or {}, "compare_stderr": stderr})

    # render_aur.py / render_nix.py: ValueErrors are uncaught tracebacks, so
    # only their exit status is compared.
    case("aur-desktop", "render_aur.py", ["desktop", "--version", "v1.2.3", "--sha256", A, "--output", "out"],
         rust=["render-aur"])
    case("aur-cli", "render_aur.py", ["cli", "--version", "1.2.3", "--sha256", B, "--output", "out/nested"],
         rust=["render-aur"])
    case("aur-bad-version", "render_aur.py", ["cli", "--version", "1.2.3-rc.1", "--sha256", B, "--output", "out"],
         rust=["render-aur"], stderr=False)
    case("aur-bad-sha", "render_aur.py", ["desktop", "--version", "1.2.3", "--sha256", A.upper(), "--output", "out"],
         rust=["render-aur"], stderr=False)
    case("nix", "render_nix.py", ["--version", "v1.2.3", "--sha256-desktop", A, "--sha256-cli", B, "--output", "out"],
         rust=["render-nix"])
    case("nix-bad-version", "render_nix.py", ["--version", "1", "--sha256-desktop", A, "--sha256-cli", B, "--output", "out"],
         rust=["render-nix"], stderr=False)
    case("cask", "render_homebrew.py", ["cask", "--version", "v1.2.3-rc.1", "--sha256", C, "--output", "Casks/kassiber.rb"],
         rust=["render-homebrew"])
    case("formula", "render_homebrew.py", ["cli-formula", "--version", "1.2.3", "--sha256-macos-arm64", A,
                                           "--sha256-linux-x64", B, "--output", "Formula/kassiber-cli.rb"],
         rust=["render-homebrew"])
    case("cask-bad-sha", "render_homebrew.py", ["cask", "--version", "1.2.3", "--sha256", "abc", "--output", "x.rb"],
         rust=["render-homebrew"], stderr=False)
    # release_notes.py
    for name, version in [("notes", "v1.2.3"), ("notes-rc", "1.2.3-rc.1"), ("notes-missing", "9.9.9"),
                          ("notes-empty", "1.2.2"), ("notes-bad", "1.2")]:
        case(name, "release_notes.py", ["--version", version, "--changelog", "CHANGELOG.md", "--output", "notes.md"],
             rust=["release-notes"], setup={"CHANGELOG.md": CHANGELOG})
    case("notes-no-changelog", "release_notes.py", ["--version", "1.2.3", "--output", "notes.md"],
         rust=["release-notes"])
    # write_build_info.py
    case("build-info", "write_build_info.py", ["--output", "out/BUILD_INFO.json", "--version", "1.2.3",
                                               "--commit", "0123456789abcdef", "--ref", "refs/tags/v1.2.3",
                                               "--run-id", "42", "--channel", "release",
                                               "--built-at", "2026-01-02T03:04:05Z"],
         rust=["build-info"])
    case("build-info-epoch", "write_build_info.py", ["--output", "BUILD_INFO.json", "--version", "1.2.3-dev",
                                                     "--commit", "ü", "--ref", "main", "--channel", "dev"],
         rust=["build-info"], env={"SOURCE_DATE_EPOCH": "1700000000"})
    # release_manifest.py
    case("manifest-generate", "release_manifest.py",
         ["generate", "--release-dir", "./release/", "--version", "v1.2.3", "--exclude", "notes.md"],
         rust=["manifest"], setup={**release_files, "release/notes.md": "x"})
    case("manifest-generate-empty", "release_manifest.py", ["generate", "--release-dir", "release", "--version", "1.2.3"],
         rust=["manifest"], setup={"release/kassiber-1.2.3-manifest.txt": "old"})
    case("manifest-generate-unsafe", "release_manifest.py", ["generate", "--release-dir", "release", "--version", "1.2.3"],
         rust=["manifest"], setup={"release/.hidden": "x"})
    case("manifest-generate-bad-version", "release_manifest.py", ["generate", "--release-dir", "release", "--version", "1.2"],
         rust=["manifest"], setup=release_files)
    case("verify-artifacts", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "release/kassiber-1.2.3-manifest.txt"],
         rust=["manifest"], setup={**release_files, "release/kassiber-1.2.3-manifest.txt": signing["manifest"]})
    case("verify-artifacts-subset", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "release/kassiber-1.2.3-manifest.txt",
          "--allow-subset"],
         rust=["manifest"], setup={"release/kassiber-linux-x64.AppImage": "desktop",
                                   "release/kassiber-1.2.3-manifest.txt": signing["manifest"]})
    case("verify-artifacts-tampered", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "release/kassiber-1.2.3-manifest.txt"],
         rust=["manifest"], setup={**release_files, "release/kassiber-linux-x64.AppImage": "evil",
                                   "release/kassiber-1.2.3-manifest.txt": signing["manifest"]})
    case("verify-artifacts-extra", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "release/kassiber-1.2.3-manifest.txt"],
         rust=["manifest"], setup={**release_files, "release/extra.txt": "x",
                                   "release/kassiber-1.2.3-manifest.txt": signing["manifest"]})
    case("verify-artifacts-outside", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "kassiber-1.2.3-manifest.txt"],
         rust=["manifest"], setup={**release_files, "kassiber-1.2.3-manifest.txt": signing["manifest"]})
    case("verify-artifacts-version", "release_manifest.py",
         ["verify-artifacts", "--release-dir", "release", "--manifest", "release/kassiber-1.2.4-manifest.txt"],
         rust=["manifest"], setup={**release_files, "release/kassiber-1.2.4-manifest.txt": signing["manifest"]})
    case("verify-release", "release_manifest.py", [*verify_release, "--fingerprint", fp],
         rust=["manifest"], setup=signed)
    case("verify-release-wrong-key", "release_manifest.py",
         [*verify_release, "--fingerprint", "SHA256:" + "A" * 43], rust=["manifest"], setup=signed)
    case("verify-release-bad-fingerprint", "release_manifest.py", [*verify_release, "--fingerprint", "SHA256:short"],
         rust=["manifest"], setup=signed)
    case("verify-release-tampered", "release_manifest.py", [*verify_release, "--fingerprint", fp],
         rust=["manifest"], setup={**signed, "release/kassiber-linux-x64.AppImage": "evil"})
    case("verify-release-bad-signature", "release_manifest.py", [*verify_release, "--fingerprint", fp],
         rust=["manifest"], setup={**signed, "release/kassiber-1.2.3-manifest.txt.sig": "not a signature"})
    # The policy reads keys relative to the repository root; the fixture
    # runs against a scratch "repository" holding the throwaway key.
    fixture_repo = {"release/fixture.pub": signing["public_key"],
                    "release/allowed_signers": signing["allowed_signers"]}
    for name, overrides in [("policy", {}), ("policy-disabled", {"enabled": False, "fingerprint": ""}),
                            ("policy-disabled-fingerprint", {"enabled": False}),
                            ("policy-mismatch", {"fingerprint": "SHA256:" + "A" * 43}),
                            ("policy-escape", {"public_key_path": "../outside.pub"}),
                            ("policy-absolute", {"allowed_signers_path": "/etc/passwd"}),
                            ("policy-schema", {"schema_version": True}),
                            ("policy-namespace", {"namespace": "other"})]:
        case(name, "release_manifest.py", ["policy", "--policy", "policy.json"], rust=["manifest"],
             setup={"policy.json": json.dumps({**policy, **overrides})}, repo_files=fixture_repo)
    case("policy-require-enabled", "release_manifest.py", ["policy", "--policy", "policy.json", "--require-enabled"],
         rust=["manifest"], setup={"policy.json": json.dumps({**policy, "enabled": False, "fingerprint": ""})},
         repo_files=fixture_repo)
    case("policy-not-json", "release_manifest.py", ["policy", "--policy", "policy.json"], rust=["manifest"],
         setup={"policy.json": "{"}, repo_files=fixture_repo)
    case("policy-repository", "release_manifest.py", ["policy", "--policy", "{repo}/packaging/release/signing-policy.json",
                                                      "--require-enabled"], rust=["manifest"])
    return out


def run_case(spec: dict) -> dict:
    scratch = Path(tempfile.mkdtemp(prefix="kassiber-tool-goldens-"))
    repo = ROOT
    try:
        work = scratch / "work"
        work.mkdir()
        for name, text in spec["setup"].items():
            path = work / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.encode())
        script = SCRIPTS / spec["script"]
        if spec["repo_files"]:
            # release_manifest.py resolves policies against its own checkout:
            # run a copy of the scripts inside a scratch "repository".
            repo = scratch / "repo"
            (repo / "scripts").mkdir(parents=True)
            shutil.copy(script, repo / "scripts" / spec["script"])
            os.symlink(ROOT / "kassiber", repo / "kassiber")
            for name, text in spec["repo_files"].items():
                path = repo / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            script = repo / "scripts" / spec["script"]
        env = {k: v for k, v in os.environ.items() if k != "SOURCE_DATE_EPOCH"}
        env.update(spec["env"])
        args = [arg.replace("{repo}", str(ROOT)) for arg in spec["args"]]
        result = subprocess.run([sys.executable, str(script), *args], cwd=work, env=env,
                                capture_output=True)

        def text(raw: bytes) -> str:
            decoded = raw.decode("utf-8")
            if spec["repo_files"]:
                decoded = decoded.replace(str(repo.resolve()), "{repo}")
            return decoded.replace(str(ROOT), "{repo}")

        record = {**spec, "exit": result.returncode, "stdout": text(result.stdout),
                  "stderr": text(result.stderr) if spec["compare_stderr"] else None,
                  "files": {name: rec for name, rec in tree(work).items() if name not in spec["setup"]}}
        if spec["name"] == "build-info-epoch":
            assert json.loads((work / "BUILD_INFO.json").read_text())["built_at"] == "2023-11-14T22:13:20Z"
        return record
    finally:
        shutil.rmtree(scratch)


def main() -> int:
    signing = make_signing_fixture()
    records = [run_case(spec) for spec in cases(signing)]
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps({"cases": records}, indent=1, ensure_ascii=False) + "\n")
    print(f"{OUTPUT.relative_to(ROOT)}: {len(records)} cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
