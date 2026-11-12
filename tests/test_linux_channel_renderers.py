"""Linux channel renderers, publication workflows and the repository
publisher. Standard library only, run with `python3 -I` (plan 22, D2); the
AUR and Nix renderers are the release tool's `render-aur` and `render-nix`,
found through KASSIBER_RELEASE_TOOL as in the workflows."""

import hashlib
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def release_tool() -> str:
    """The `kassiber-release` binary; CI must provide it, a local run skips."""
    tool = os.environ.get("KASSIBER_RELEASE_TOOL")
    if not tool:
        if os.environ.get("CI"):
            raise RuntimeError("KASSIBER_RELEASE_TOOL is required under CI")
        raise unittest.SkipTest("set KASSIBER_RELEASE_TOOL to the kassiber-release binary")
    return tool


def render(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [release_tool(), *args], capture_output=True, text=True, check=False
    )


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def dnf_snapshot_id(metadata: Path, signature: Path) -> str:
    component_hashes = f"{sha256(metadata)}\n{sha256(signature)}\n"
    return hashlib.sha256(component_hashes.encode("ascii")).hexdigest()


class AurRendererTest(unittest.TestCase):
    def test_desktop_recipe_uses_the_appimage(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            done = render("render-aur", "desktop", "--version", "v1.2.3",
                          "--sha256", "a" * 64, "--output", str(output))
            self.assertEqual(done.returncode, 0, done.stderr)

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
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            done = render("render-aur", "cli", "--version", "1.2.3",
                          "--sha256", "b" * 64, "--output", str(output))
            self.assertEqual(done.returncode, 0, done.stderr)

            pkgbuild = (output / "PKGBUILD").read_text(encoding="utf-8")
            self.assertIn("pkgname=kassiber-cli-bin", pkgbuild)
            self.assertIn("kassiber-cli-linux-x64.tar.gz", pkgbuild)
            self.assertIn("kassiber-cli-linux-x64/kassiber", pkgbuild)
            self.assertIn(
                "conflicts=('kassiber' 'kassiber-bin' 'kassiber-cli')",
                pkgbuild,
            )

    def test_rejects_invalid_versions_and_checksums(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            for package, version, digest in (
                ("cli", "not a version", "a" * 64),
                ("desktop", "1.2.3", "not-a-sha"),
            ):
                output = Path(temp_dir) / package
                done = render("render-aur", package, "--version", version,
                              "--sha256", digest, "--output", str(output))
                self.assertNotEqual(done.returncode, 0, done.stdout)
                self.assertFalse((output / "PKGBUILD").exists())


class NixRendererTest(unittest.TestCase):
    def test_flake_is_release_pinned_and_declares_binary_provenance(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir)
            done = render("render-nix", "--version", "v1.2.3",
                          "--sha256-desktop", "a" * 64, "--sha256-cli", "b" * 64,
                          "--output", str(output))
            self.assertEqual(done.returncode, 0, done.stderr)

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
        self.assertIn('"$KASSIBER_RELEASE_TOOL" manifest verify-release', workflow)
        self.assertIn("packaging/release/signing-policy.json", workflow)
        self.assertIn('--signature "release/$manifest_name.sig"', workflow)
        self.assertNotIn('$manifest_name.asc', workflow)
        self.assertIn('gpg --batch --export-ssh-key "${archive_key_fingerprint}!"', workflow)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", workflow)
        self.assertIn("LINUX_ARCHIVE_GPG_FINGERPRINT", workflow)

    def test_publication_jobs_execute_against_the_verified_release_commit(self):
        workflow = (
            ROOT / ".github/workflows/publish-linux-channels.yml"
        ).read_text(encoding="utf-8")
        prepare = workflow.split("\n  prepare:\n", 1)[1].split(
            "\n  repositories:\n", 1
        )[0]
        repositories = workflow.split("\n  repositories:\n", 1)[1].split(
            "\n  aur:\n", 1
        )[0]
        nix = workflow.split("\n  nix:\n", 1)[1]

        self.assertIn('echo "commit=$(git rev-parse HEAD)" >> "$GITHUB_OUTPUT"', prepare)
        self.assertIn(
            "release_commit: ${{ steps.release-ref.outputs.commit }}", prepare
        )
        for job in (repositories, nix):
            kassiber_checkout = job.split(
                "uses: actions/checkout@", 1
            )[1].split("\n      - ", 1)[0]
            self.assertIn(
                "ref: ${{ needs.prepare.outputs.release_commit }}",
                kassiber_checkout,
            )

        self.assertLess(
            repositories.index("ref: ${{ needs.prepare.outputs.release_commit }}"),
            repositories.index('"$KASSIBER_RELEASE_TOOL" manifest policy'),
        )

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
        self.assertIn('"$KASSIBER_RELEASE_TOOL" manifest verify-release', workflow)
        self.assertIn('manifest.txt.sig" >> "$GITHUB_OUTPUT"', workflow)
        self.assertNotIn("gnupg", workflow)
        self.assertNotIn(".asc", workflow)
        self.assertIn("gh release download", workflow)
        self.assertIn("gh release edit", workflow)
        self.assertIn("--draft=false", workflow)
        self.assertIn("fetch-depth: 0", workflow)
        tag_guard = 'git merge-base --is-ancestor "refs/tags/$RELEASE_TAG_NAME^{commit}" origin/main'
        self.assertIn(tag_guard, workflow)
        self.assertIn('ref: ${{ github.sha }}', workflow)
        self.assertLess(
            workflow.index(tag_guard),
            workflow.index('"$KASSIBER_RELEASE_TOOL" manifest policy'),
        )
        self.assertNotIn("gh release upload", workflow)
        self.assertNotIn("--clobber", workflow)
        self.assertNotIn("tauri build", workflow)
        self.assertNotIn("pyinstaller", workflow)

    def test_homebrew_follows_public_downloads_and_can_retry_without_replacing_assets(self):
        workflow = (
            ROOT / ".github/workflows/finalize-signed-release.yml"
        ).read_text(encoding="utf-8")

        self.assertLess(
            workflow.index('gh release edit "$RELEASE_TAG_NAME"'),
            workflow.index('git push origin "HEAD:${HOMEBREW_TAP_BRANCH}"'),
        )
        self.assertIn("if: steps.release.outputs.draft == 'true'", workflow)
        self.assertIn('case "$actual_draft" in true|false)', workflow)
        self.assertIn('cmp release-assets.json current-assets.json', workflow)
        self.assertNotIn('gh release upload', workflow)

    def test_release_publish_rejects_tags_outside_main_history(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")

        publish = workflow.split("\n  publish:\n", 1)[1]
        self.assertIn("fetch-depth: 0", publish)
        self.assertIn("git merge-base --is-ancestor HEAD origin/main", publish)
        self.assertLess(
            publish.index("git merge-base --is-ancestor HEAD origin/main"),
            publish.index('"$KASSIBER_RELEASE_TOOL" manifest policy'),
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

    def _write_apt_metadata(
        self,
        apt: Path,
        *,
        include_by_hash: bool = True,
        include_signed_metadata_entries: bool = False,
    ) -> None:
        release_dir = apt / "dists/prerelease"
        index = release_dir / "main/binary-amd64/Packages"
        index.parent.mkdir(parents=True)
        index.write_text("Package: kassiber-cli\n", encoding="utf-8")
        digest = sha256(index)
        signed_metadata_entries = ""
        if include_signed_metadata_entries:
            signed_metadata_entries = "".join(
                f" {'0' * 64} 0 {name}\n"
                for name in ("Release", "Release.gpg", "InRelease")
            )
        (release_dir / "Release").write_text(
            "Acquire-By-Hash: yes\n"
            "SHA256:\n"
            f" {digest} {index.stat().st_size} main/binary-amd64/Packages\n"
            f"{signed_metadata_entries}",
            encoding="utf-8",
        )
        (release_dir / "InRelease").write_text("signed", encoding="utf-8")
        (release_dir / "Release.gpg").write_text("signature", encoding="utf-8")
        if include_by_hash:
            by_hash = index.parent / f"by-hash/SHA256/{digest}"
            by_hash.parent.mkdir(parents=True)
            by_hash.write_bytes(index.read_bytes())

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

    def test_publisher_rejects_missing_by_hash_before_aws(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            apt, dnf = self._repository_roots(root)
            self._write_apt_metadata(apt, include_by_hash=False)
            (dnf / "repodata/repomd.xml").write_text("metadata", encoding="utf-8")
            (dnf / "repodata/repomd.xml.asc").write_text(
                "signature", encoding="utf-8"
            )
            fake_bin = root / "bin"
            fake_bin.mkdir()
            aws = fake_bin / "aws"
            aws.write_text(
                "#!/bin/sh\n"
                "printf called >> \"$KASSIBER_TEST_AWS_LOG\"\n",
                encoding="utf-8",
            )
            aws.chmod(0o755)
            log = root / "aws.log"
            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
            environment["KASSIBER_TEST_AWS_LOG"] = str(log)

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
                env=environment,
            )

            self.assertEqual(completed.returncode, 2)
            self.assertIn("APT by-hash object is missing", completed.stderr)
            self.assertFalse(log.exists())

    def test_publisher_accepts_a_release_that_lists_signed_metadata(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            apt, dnf = self._repository_roots(root)
            self._write_apt_metadata(apt, include_signed_metadata_entries=True)
            (dnf / "repodata/repomd.xml").write_text("metadata", encoding="utf-8")
            (dnf / "repodata/repomd.xml.asc").write_text(
                "signature", encoding="utf-8"
            )
            fake_bin = root / "bin"
            fake_bin.mkdir()
            aws = fake_bin / "aws"
            aws.write_text(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$KASSIBER_TEST_AWS_LOG\"\n",
                encoding="utf-8",
            )
            aws.chmod(0o755)
            log = root / "aws.log"
            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
            environment["KASSIBER_TEST_AWS_LOG"] = str(log)

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
                env=environment,
            )

            # The signed top-level objects are the atomic switch, not by-hash
            # indices, so listing them must not block publication.
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn(
                "apt/dists/prerelease/InRelease",
                log.read_text(encoding="utf-8"),
            )

    def test_dnf_publish_uses_a_suite_scoped_immutable_snapshot(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            apt, dnf = self._repository_roots(root)
            self._write_apt_metadata(apt)
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
            by_hash_publish = next(
                index
                for index, line in enumerate(call_lines)
                if "--include */by-hash/SHA256/*" in line
            )
            mutable_index_publish = next(
                index
                for index, line in enumerate(call_lines)
                if "--exclude */by-hash/*" in line
            )
            in_release_publish = next(
                index
                for index, line in enumerate(call_lines)
                if "apt/dists/prerelease/InRelease --content-type" in line
            )
            self.assertLess(by_hash_publish, mutable_index_publish)
            self.assertLess(mutable_index_publish, in_release_publish)
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
