import hashlib
import importlib.util
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def load_script(name: str):
    path = ROOT / "scripts" / name
    spec = importlib.util.spec_from_file_location(path.stem, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dnf_snapshot_id(metadata: Path, signature: Path) -> str:
    component_hashes = f"{sha256(metadata)}\n{sha256(signature)}\n"
    return hashlib.sha256(component_hashes.encode("ascii")).hexdigest()


class AurRendererTest(unittest.TestCase):
    def test_desktop_recipe_uses_the_appimage(self):
        renderer = load_script("render_aur.py")
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            renderer.render_desktop("v1.2.3", "a" * 64, output)

            pkgbuild = (output / "PKGBUILD").read_text(encoding="utf-8")
            srcinfo = (output / ".SRCINFO").read_text(encoding="utf-8")
            self.assertIn("pkgname=kassiber-bin", pkgbuild)
            self.assertIn("kassiber-linux-x64.AppImage", pkgbuild)
            self.assertIn("APPIMAGE_EXTRACT_AND_RUN", (output / "kassiber").read_text())
            self.assertIn("--cli", (output / "kassiber").read_text())
            self.assertIn("conflicts=('kassiber' 'kassiber-cli'", pkgbuild)
            self.assertIn("'webkit2gtk-4.1'", pkgbuild)
            self.assertIn("sha256sums = " + sha256(output / "kassiber"), srcinfo)

    def test_cli_recipe_uses_the_frozen_archive(self):
        renderer = load_script("render_aur.py")
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            renderer.render_cli("1.2.3", "b" * 64, output)

            pkgbuild = (output / "PKGBUILD").read_text(encoding="utf-8")
            self.assertIn("pkgname=kassiber-cli-bin", pkgbuild)
            self.assertIn("kassiber-cli-linux-x64.tar.gz", pkgbuild)
            self.assertIn("kassiber-cli-linux-x64/kassiber", pkgbuild)
            self.assertIn(
                "conflicts=('kassiber' 'kassiber-bin' 'kassiber-cli')",
                pkgbuild,
            )

    def test_rejects_invalid_versions_and_checksums(self):
        renderer = load_script("render_aur.py")
        with tempfile.TemporaryDirectory() as temp_dir:
            with self.assertRaises(ValueError):
                renderer.render_cli("not a version", "a" * 64, Path(temp_dir))
            with self.assertRaises(ValueError):
                renderer.render_desktop("1.2.3", "not-a-sha", Path(temp_dir))


class NixRendererTest(unittest.TestCase):
    def test_flake_is_release_pinned_and_declares_binary_provenance(self):
        renderer = load_script("render_nix.py")
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            renderer.render_flake("v1.2.3", "a" * 64, "b" * 64, output)

            flake = (output / "flake.nix").read_text(encoding="utf-8")
            self.assertIn("/v1.2.3/kassiber-linux-x64.AppImage", flake)
            self.assertIn("/v1.2.3/kassiber-cli-linux-x64.tar.gz", flake)
            self.assertIn('sha256 = "' + "a" * 64 + '"', flake)
            self.assertIn('sha256 = "' + "b" * 64 + '"', flake)
            self.assertIn("lib.sourceTypes.binaryNativeCode", flake)
            self.assertIn("pkgs.autoPatchelfHook", flake)
            self.assertIn('system = "x86_64-linux"', flake)
            self.assertIn('makeWrapper "$out/bin/kassiber-ui"', flake)
            self.assertNotIn("aarch64-linux", flake)


class LinuxChannelWorkflowTest(unittest.TestCase):
    def test_publication_is_manual_guarded_and_checksum_verified(self):
        workflow = (
            ROOT / ".github/workflows/publish-linux-channels.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("workflow_dispatch:", workflow)
        self.assertNotIn("\n  push:", workflow)
        self.assertNotIn("apt upgrade", workflow)
        self.assertNotIn("dnf upgrade", workflow)
        self.assertIn("scripts/release_manifest.py verify-release", workflow)
        self.assertIn("packaging/release/signing-policy.json", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn("LINUX_ARCHIVE_GPG_FINGERPRINT", workflow)

    def test_release_workflow_uploads_rpms(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("release/*.rpm", workflow)
        self.assertIn("package-cli-rpm.sh", workflow)
        self.assertIn("package-desktop-rpm.sh", workflow)

    def test_signed_release_finalizer_never_rebuilds_or_replaces_assets(self):
        workflow = (
            ROOT / ".github/workflows/finalize-signed-release.yml"
        ).read_text(encoding="utf-8")

        self.assertIn("environment: release-production", workflow)
        self.assertIn("scripts/release_manifest.py verify-release", workflow)
        self.assertIn("gh release download", workflow)
        self.assertIn("gh release edit", workflow)
        self.assertIn("--draft=false", workflow)
        self.assertIn("fetch-depth: 0", workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertLess(
            workflow.index("git merge-base --is-ancestor HEAD origin/main"),
            workflow.index("scripts/release_manifest.py policy"),
        )
        self.assertNotIn("gh release upload", workflow)
        self.assertNotIn("--clobber", workflow)
        self.assertNotIn("tauri build", workflow)
        self.assertNotIn("pyinstaller", workflow)

    def test_release_publish_rejects_tags_outside_main_history(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")

        publish = workflow.split("\n  publish:\n", 1)[1]
        self.assertIn("fetch-depth: 0", publish)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", publish)
        self.assertLess(
            publish.index("git merge-base --is-ancestor HEAD origin/main"),
            publish.index("scripts/release_manifest.py policy"),
        )


class RepositoryPublisherTest(unittest.TestCase):
    def _repository_roots(self, root: Path) -> tuple[Path, Path]:
        apt = root / "apt"
        dnf = root / "dnf"
        (apt / "pool").mkdir(parents=True)
        (apt / "dists/prerelease").mkdir(parents=True)
        (dnf / "packages").mkdir(parents=True)
        (dnf / "repodata").mkdir(parents=True)
        return apt, dnf

    def test_publisher_preflights_every_signature_before_aws(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            apt, dnf = self._repository_roots(root)
            (apt / "dists/prerelease/InRelease").write_text("signed")
            (dnf / "repodata/repomd.xml").write_text("metadata")

            completed = subprocess.run(
                [
                    str(ROOT / "scripts/publish-linux-repositories-s3.sh"),
                    "--apt",
                    str(apt),
                    "--dnf",
                    str(dnf),
                    "--suite",
                    "prerelease",
                    "--destination",
                    "s3://example/kassiber",
                    "--base-url",
                    "https://packages.invalid",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("APT Release is missing", completed.stderr)

    def test_dnf_publish_uses_a_suite_scoped_immutable_snapshot(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            apt, dnf = self._repository_roots(root)
            release_dir = apt / "dists/prerelease"
            for name in ("InRelease", "Release", "Release.gpg"):
                (release_dir / name).write_text(name, encoding="utf-8")
            repomd = dnf / "repodata/repomd.xml"
            repomd.write_text("metadata", encoding="utf-8")
            signature = dnf / "repodata/repomd.xml.asc"
            signature.write_text(
                "signature", encoding="utf-8"
            )
            (dnf / "packages/kassiber-1.2.3-1.x86_64.rpm").write_text(
                "package", encoding="utf-8"
            )
            fake_bin = root / "bin"
            fake_bin.mkdir()
            aws = fake_bin / "aws"
            aws.write_text(
                "#!/bin/sh\n"
                "printf '%s\\n' \"$*\" >> \"$KASSIBER_TEST_AWS_LOG\"\n",
                encoding="utf-8",
            )
            aws.chmod(0o755)
            log = root / "aws.log"
            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
            environment["KASSIBER_TEST_AWS_LOG"] = str(log)

            subprocess.run(
                [
                    str(ROOT / "scripts/publish-linux-repositories-s3.sh"),
                    "--apt",
                    str(apt),
                    "--dnf",
                    str(dnf),
                    "--suite",
                    "prerelease",
                    "--destination",
                    "s3://example/kassiber",
                    "--base-url",
                    "https://packages.invalid",
                ],
                check=True,
                capture_output=True,
                text=True,
                env=environment,
            )
            snapshot = dnf_snapshot_id(repomd, signature)
            calls = log.read_text(encoding="utf-8")
            call_lines = calls.splitlines()
            for name in ("Release", "Release.gpg", "InRelease"):
                matching = [
                    line
                    for line in call_lines
                    if f"apt/dists/prerelease/{name} --content-type" in line
                    and line.startswith("s3 cp ")
                ]
                self.assertEqual(len(matching), 1, (name, call_lines))
                self.assertIn("--cache-control no-cache", matching[0])
            self.assertIn(
                f"dnf/prerelease/snapshots/{snapshot}/packages",
                calls,
            )
            self.assertIn(
                f"dnf/prerelease/snapshots/{snapshot}/repodata",
                calls,
            )
            self.assertIn("dnf/prerelease/mirrorlist", calls)
            self.assertNotIn("s3://example/kassiber/dnf/packages", calls)


if __name__ == "__main__":
    unittest.main()
