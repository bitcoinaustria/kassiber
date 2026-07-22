import json
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
INSTALL_CONTEXT_DIR = ROOT / "packaging" / "linux" / "install-context"


def _build_test_deb(
    root: Path,
    *,
    package: str,
    surface: str,
    version: str = "1.2.3",
    architecture: str = "amd64",
    marker_surface: str | None = None,
) -> Path:
    package_root = root / f"root-{package}"
    (package_root / "DEBIAN").mkdir(parents=True)
    (package_root / "DEBIAN").chmod(0o755)
    (package_root / "usr/bin").mkdir(parents=True)
    (package_root / "usr/lib/kassiber").mkdir(parents=True)

    executables = (
        ("kassiber", "kassiber-ui")
        if surface == "desktop"
        else ("kassiber",)
    )
    for executable in executables:
        executable_path = package_root / "usr/bin" / executable
        executable_path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        executable_path.chmod(0o755)
    shutil.copy2(
        INSTALL_CONTEXT_DIR / f"deb-{marker_surface or surface}.json",
        package_root / "usr/lib/kassiber/install-context.json",
    )

    conflict = "kassiber-cli" if package == "kassiber" else "kassiber"
    (package_root / "DEBIAN/control").write_text(
        "\n".join(
            (
                f"Package: {package}",
                f"Version: {version}",
                f"Architecture: {architecture}",
                "Maintainer: Bitcoin Austria",
                "Section: utils",
                "Priority: optional",
                f"Conflicts: {conflict}",
                f"Replaces: {conflict}",
                "Provides: kassiber-command",
                f"Description: Kassiber {surface} test package",
                " Packaging integration fixture.",
                "",
            )
        ),
        encoding="utf-8",
    )
    output = root / f"{package}.deb"
    subprocess.run(
        ["dpkg-deb", "--root-owner-group", "--build", str(package_root), str(output)],
        check=True,
        capture_output=True,
        text=True,
    )
    return output


@unittest.skipUnless(shutil.which("dpkg-deb"), "dpkg-deb is required")
class InstallContextTest(unittest.TestCase):
    def test_debian_markers_are_surface_specific_and_do_not_claim_repository_origin(self):
        cli = json.loads((INSTALL_CONTEXT_DIR / "deb-cli.json").read_text())
        desktop = json.loads((INSTALL_CONTEXT_DIR / "deb-desktop.json").read_text())

        for marker in (cli, desktop):
            self.assertEqual(marker["schema_version"], 1)
            self.assertEqual(marker["product"], "kassiber")
            self.assertEqual(marker["artifact_kind"], "deb")
            self.assertEqual(marker["package_manager"], "dpkg")
            self.assertEqual(marker["repository_manager"], "apt")
            self.assertEqual(marker["repository_provenance"], "probe-required")
            self.assertNotIn("upgrade_command", marker)
            self.assertNotIn("repository_origin", marker)

        self.assertEqual(cli["surface"], "cli")
        self.assertEqual(cli["package_name"], "kassiber-cli")
        self.assertEqual(desktop["surface"], "desktop")
        self.assertEqual(desktop["package_name"], "kassiber")

    def test_rpm_markers_are_surface_specific_and_require_a_repository_probe(self):
        cli = json.loads((INSTALL_CONTEXT_DIR / "rpm-cli.json").read_text())
        desktop = json.loads((INSTALL_CONTEXT_DIR / "rpm-desktop.json").read_text())

        for marker in (cli, desktop):
            self.assertEqual(marker["schema_version"], 1)
            self.assertEqual(marker["product"], "kassiber")
            self.assertEqual(marker["artifact_kind"], "rpm")
            self.assertEqual(marker["package_manager"], "rpm")
            self.assertEqual(marker["repository_manager"], "dnf")
            self.assertEqual(marker["repository_provenance"], "probe-required")
            self.assertNotIn("upgrade_command", marker)
            self.assertNotIn("repository_origin", marker)

        self.assertEqual(cli["surface"], "cli")
        self.assertEqual(cli["package_name"], "kassiber-cli")
        self.assertEqual(desktop["surface"], "desktop")
        self.assertEqual(desktop["package_name"], "kassiber")

    def test_desktop_deb_installs_the_desktop_marker(self):
        config = json.loads(
            (ROOT / "ui-tauri/src-tauri/tauri.conf.json").read_text(encoding="utf-8")
        )
        files = config["bundle"]["linux"]["deb"]["files"]
        self.assertEqual(
            files["/usr/lib/kassiber/install-context.json"],
            "../../packaging/linux/install-context/deb-desktop.json",
        )

    def test_cli_deb_installs_the_cli_marker(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            package = root / "kassiber-cli.deb"
            subprocess.run(
                [
                    str(ROOT / "scripts/package-cli-deb.sh"),
                    "--binary",
                    str(binary),
                    "--version",
                    "1.2.3",
                    "--architecture",
                    "amd64",
                    "--output",
                    str(package),
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            listing = subprocess.check_output(["dpkg-deb", "-c", str(package)], text=True)
            control = subprocess.check_output(["dpkg-deb", "-f", str(package)], text=True)
            self.assertIn("./usr/lib/kassiber/install-context.json", listing)
            self.assertIn(
                "X-Kassiber-Install-Context: /usr/lib/kassiber/install-context.json",
                control,
            )

    @unittest.skipUnless(shutil.which("dpkg"), "dpkg is required")
    def test_install_upgrade_conflict_replace_and_remove_lifecycle(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            packages_123 = root / "packages-1.2.3"
            packages_124 = root / "packages-1.2.4"
            packages_123.mkdir()
            packages_124.mkdir()
            cli_123 = _build_test_deb(
                packages_123,
                package="kassiber-cli",
                surface="cli",
                version="1.2.3",
            )
            cli_124 = _build_test_deb(
                packages_124,
                package="kassiber-cli",
                surface="cli",
                version="1.2.4",
            )
            desktop_124 = _build_test_deb(
                packages_124,
                package="kassiber",
                surface="desktop",
                version="1.2.4",
            )

            install_root = root / "install-root"
            admin_dir = install_root / "var/lib/dpkg"
            admin_dir.mkdir(parents=True)
            (admin_dir / "status").write_text("", encoding="utf-8")
            dpkg = [
                "dpkg",
                f"--root={install_root}",
                f"--admindir={admin_dir}",
                "--force-not-root",
                "--force-bad-path",
            ]

            subprocess.run(
                [*dpkg, "--install", str(cli_123)],
                check=True,
                capture_output=True,
                text=True,
            )
            marker_path = install_root / "usr/lib/kassiber/install-context.json"
            self.assertEqual(json.loads(marker_path.read_text())["surface"], "cli")

            subprocess.run(
                [*dpkg, "--install", str(cli_124)],
                check=True,
                capture_output=True,
                text=True,
            )
            status = subprocess.check_output(
                [
                    "dpkg-query",
                    f"--admindir={admin_dir}",
                    "--showformat=${Version}",
                    "--show",
                    "kassiber-cli",
                ],
                text=True,
            )
            self.assertEqual(status, "1.2.4")

            subprocess.run(
                [*dpkg, "--install", str(desktop_124)],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertEqual(json.loads(marker_path.read_text())["surface"], "desktop")
            removed_cli = subprocess.run(
                [
                    "dpkg-query",
                    f"--admindir={admin_dir}",
                    "--showformat=${db:Status-Abbrev}",
                    "--show",
                    "kassiber-cli",
                ],
                capture_output=True,
                text=True,
            )
            if removed_cli.returncode == 0:
                self.assertNotEqual(removed_cli.stdout, "ii ")
            subprocess.run(
                [*dpkg, "--remove", "kassiber"],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertFalse(marker_path.exists())

            subprocess.run(
                [*dpkg, "--install", str(desktop_124)],
                check=True,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                [*dpkg, "--install", str(cli_124)],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertEqual(json.loads(marker_path.read_text())["surface"], "cli")
            subprocess.run(
                [*dpkg, "--remove", "kassiber-cli"],
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertFalse(marker_path.exists())


@unittest.skipUnless(
    all(
        shutil.which(command)
        for command in ("apt-cache", "apt-ftparchive", "apt-get", "dpkg-deb", "gpg")
    ),
    "APT and GnuPG tooling is required",
)
class AptRepositoryBuilderTest(unittest.TestCase):
    def _generate_signing_key(self, home: Path) -> tuple[dict[str, str], str]:
        home.mkdir(mode=0o700)
        environment = os.environ.copy()
        environment["GNUPGHOME"] = str(home)
        subprocess.run(
            [
                "gpg",
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Kassiber APT Test <apt-test@invalid.example>",
                "ed25519",
                "sign",
                "1d",
            ],
            check=True,
            capture_output=True,
            env=environment,
        )
        key_listing = subprocess.check_output(
            ["gpg", "--batch", "--with-colons", "--list-secret-keys"],
            text=True,
            env=environment,
        )
        fingerprint = next(
            line.split(":")[9]
            for line in key_listing.splitlines()
            if line.startswith("fpr:")
        )
        return environment, fingerprint

    def _apt_options(self, root: Path, source_list: Path) -> list[str]:
        state = root / "apt-state"
        cache = root / "apt-cache"
        (state / "lists/partial").mkdir(parents=True)
        (cache / "archives/partial").mkdir(parents=True)
        (state / "status").write_text("", encoding="utf-8")
        return [
            "-o",
            f"Dir::Etc::sourcelist={source_list}",
            "-o",
            "Dir::Etc::sourceparts=-",
            "-o",
            f"Dir::State={state}",
            "-o",
            f"Dir::State::status={state / 'status'}",
            "-o",
            f"Dir::Cache={cache}",
            "-o",
            "APT::Get::List-Cleanup=0",
            "-o",
            "Debug::NoLocking=1",
        ]

    def test_builder_requires_an_explicit_signing_decision(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = root / "input"
            inputs.mkdir()
            _build_test_deb(inputs, package="kassiber-cli", surface="cli")
            completed = subprocess.run(
                [
                    str(ROOT / "scripts/build-apt-repository.sh"),
                    "--input",
                    str(inputs),
                    "--output",
                    str(root / "repo"),
                    "--suite",
                    "prerelease",
                    "--architecture",
                    "amd64",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("--signing-key is required", completed.stderr)
            self.assertFalse((root / "repo").exists())

    def test_builder_rejects_a_marker_for_the_wrong_package_surface(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = root / "input"
            inputs.mkdir()
            _build_test_deb(
                inputs,
                package="kassiber-cli",
                surface="cli",
                marker_surface="desktop",
            )
            completed = subprocess.run(
                [
                    str(ROOT / "scripts/build-apt-repository.sh"),
                    "--input",
                    str(inputs),
                    "--output",
                    str(root / "repo"),
                    "--suite",
                    "prerelease",
                    "--architecture",
                    "amd64",
                    "--unsigned",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("does not match its package surface", completed.stderr)
            self.assertFalse((root / "repo").exists())

    def test_builds_and_verifies_a_signed_prerelease_repository(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = root / "input"
            inputs.mkdir()
            _build_test_deb(inputs, package="kassiber", surface="desktop")
            _build_test_deb(inputs, package="kassiber-cli", surface="cli")
            environment, fingerprint = self._generate_signing_key(root / "gnupg")
            repository = root / "repository"
            release_epoch = int(time.time())

            subprocess.run(
                [
                    str(ROOT / "scripts/build-apt-repository.sh"),
                    "--input",
                    str(inputs),
                    "--output",
                    str(repository),
                    "--suite",
                    "prerelease",
                    "--architecture",
                    "amd64",
                    "--not-automatic",
                    "--but-automatic-upgrades",
                    "--release-epoch",
                    str(release_epoch),
                    "--signing-key",
                    fingerprint,
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
                env=environment,
            )

            release_dir = repository / "dists/prerelease"
            release = (release_dir / "Release").read_text(encoding="utf-8")
            self.assertIn("Origin: Kassiber", release)
            self.assertIn("Label: Kassiber", release)
            self.assertIn("Suite: prerelease", release)
            self.assertIn("Architectures: amd64", release)
            self.assertIn("NotAutomatic: yes", release)
            self.assertIn("ButAutomaticUpgrades: yes", release)
            self.assertIn("Acquire-By-Hash: yes", release)
            self.assertTrue((release_dir / "InRelease").is_file())
            self.assertTrue((release_dir / "Release.gpg").is_file())

            subprocess.run(
                ["gpg", "--batch", "--verify", str(release_dir / "InRelease")],
                check=True,
                capture_output=True,
                env=environment,
            )
            subprocess.run(
                [
                    "gpg",
                    "--batch",
                    "--verify",
                    str(release_dir / "Release.gpg"),
                    str(release_dir / "Release"),
                ],
                check=True,
                capture_output=True,
                env=environment,
            )

            packages = (
                release_dir / "main/binary-amd64/Packages"
            ).read_text(encoding="utf-8")
            self.assertIn("Package: kassiber\n", packages)
            self.assertIn("Package: kassiber-cli\n", packages)
            self.assertIn("pool/main/k/kassiber/kassiber_1.2.3_amd64.deb", packages)
            self.assertIn(
                "pool/main/k/kassiber-cli/kassiber-cli_1.2.3_amd64.deb",
                packages,
            )
            by_hash = release_dir / "main/binary-amd64/by-hash/SHA256"
            self.assertEqual(len(list(by_hash.iterdir())), 2)

            public_key = root / "kassiber-archive-keyring.gpg"
            public_key.write_bytes(
                subprocess.check_output(
                    ["gpg", "--batch", "--export", fingerprint], env=environment
                )
            )
            source_list = root / "kassiber.sources.list"
            source_list.write_text(
                "deb [signed-by="
                f"{public_key}] file:{repository} prerelease main\n",
                encoding="utf-8",
            )
            apt_options = self._apt_options(root, source_list)
            subprocess.run(
                ["apt-get", *apt_options, "update"],
                check=True,
                capture_output=True,
                text=True,
            )
            policy = subprocess.check_output(
                ["apt-cache", *apt_options, "policy", "kassiber-cli"],
                text=True,
            )
            self.assertIn("Candidate: 1.2.3", policy)


class PackagingWorkflowVersionGateTest(unittest.TestCase):
    def test_release_tag_must_match_the_embedded_package_version(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")
        self.assertIn(
            "DependencyDriftTests.test_app_version_is_consistent_across_package_metadata",
            workflow,
        )
        self.assertIn('expected_version="${release_tag#v}"', workflow)
        self.assertIn('if [ "$version" != "$expected_version" ]', workflow)
        self.assertIn("does not match package version", workflow)


if __name__ == "__main__":
    unittest.main()
