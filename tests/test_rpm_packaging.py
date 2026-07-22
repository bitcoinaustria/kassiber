import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RPM_TOOLS = ("cpio", "createrepo_c", "dpkg-deb", "rpm", "rpm2cpio", "rpmbuild")


def build_desktop_deb(root: Path, version: str = "1.2.3") -> Path:
    package_root = root / "desktop-root"
    (package_root / "DEBIAN").mkdir(parents=True)
    for path in (
        "usr/bin",
        "usr/lib/Kassiber/binaries",
        "usr/lib/kassiber",
        "usr/share/applications",
        "usr/share/icons/hicolor/128x128/apps",
    ):
        (package_root / path).mkdir(parents=True)
    for executable in ("usr/bin/kassiber", "usr/bin/kassiber-ui"):
        path = package_root / executable
        path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        path.chmod(0o755)
    sidecar = package_root / "usr/lib/Kassiber/binaries/kassiber-cli-x86_64-unknown-linux-gnu"
    sidecar.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
    sidecar.chmod(0o755)
    shutil.copy2(
        ROOT / "packaging/linux/install-context/deb-desktop.json",
        package_root / "usr/lib/kassiber/install-context.json",
    )
    (package_root / "usr/share/applications/Kassiber.desktop").write_text(
        "[Desktop Entry]\nName=Kassiber\nExec=kassiber-ui\nType=Application\n",
        encoding="utf-8",
    )
    (package_root / "usr/share/icons/hicolor/128x128/apps/kassiber-ui.png").write_bytes(
        b"test-icon"
    )
    (package_root / "DEBIAN/control").write_text(
        "\n".join(
            (
                "Package: kassiber",
                f"Version: {version}",
                "Architecture: amd64",
                "Maintainer: Bitcoin Austria",
                "Description: Kassiber desktop test package",
                " Test fixture.",
                "",
            )
        ),
        encoding="utf-8",
    )
    output = root / "kassiber.deb"
    subprocess.run(
        ["dpkg-deb", "--root-owner-group", "--build", str(package_root), str(output)],
        check=True,
        capture_output=True,
        text=True,
    )
    return output


def rpm_file(path: Path, member: str) -> bytes:
    rpm2cpio = subprocess.Popen(
        ["rpm2cpio", str(path)],
        stdout=subprocess.PIPE,
    )
    completed = subprocess.run(
        ["cpio", "--quiet", "-i", "--to-stdout", f".{member}"],
        stdin=rpm2cpio.stdout,
        capture_output=True,
        check=True,
    )
    assert rpm2cpio.stdout is not None
    rpm2cpio.stdout.close()
    returncode = rpm2cpio.wait()
    if returncode != 0:
        raise subprocess.CalledProcessError(returncode, rpm2cpio.args)
    return completed.stdout


@unittest.skipUnless(
    all(shutil.which(command) for command in RPM_TOOLS),
    "RPM, repository, and Debian tooling is required",
)
class RpmPackagingTest(unittest.TestCase):
    def test_builds_cli_desktop_and_source_rpms(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            cli_rpm = root / "kassiber-cli.rpm"
            cli_srpm = root / "kassiber-cli.src.rpm"
            subprocess.run(
                [
                    str(ROOT / "scripts/package-cli-rpm.sh"),
                    "--binary",
                    str(binary),
                    "--version",
                    "1.2.3",
                    "--architecture",
                    "x86_64",
                    "--output",
                    str(cli_rpm),
                    "--source-output",
                    str(cli_srpm),
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            desktop_rpm = root / "kassiber.rpm"
            desktop_srpm = root / "kassiber.src.rpm"
            subprocess.run(
                [
                    str(ROOT / "scripts/package-desktop-rpm.sh"),
                    "--deb",
                    str(build_desktop_deb(root)),
                    "--version",
                    "1.2.3",
                    "--architecture",
                    "x86_64",
                    "--output",
                    str(desktop_rpm),
                    "--source-output",
                    str(desktop_srpm),
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )

            self.assertTrue(cli_srpm.is_file())
            self.assertTrue(desktop_srpm.is_file())
            obs_package = root / "obs-kassiber-cli"
            obs_result = subprocess.run(
                [
                    str(ROOT / "scripts/prepare-obs-package.sh"),
                    "--source-rpm",
                    cli_srpm.name,
                    "--output",
                    obs_package.name,
                ],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertEqual(obs_result.returncode, 0, obs_result.stderr)
            self.assertTrue((obs_package / "kassiber-cli.spec").is_file())
            rejected_binary = subprocess.run(
                [
                    str(ROOT / "scripts/prepare-obs-package.sh"),
                    "--source-rpm",
                    cli_rpm.name,
                    "--output",
                    "obs-invalid-binary",
                ],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertEqual(rejected_binary.returncode, 2)
            self.assertIn("Expected a source RPM", rejected_binary.stderr)
            for source_rpm, package in (
                (cli_srpm, "kassiber-cli"),
                (desktop_srpm, "kassiber"),
            ):
                rebuild = root / f"rebuild-{package}"
                for directory in (
                    "BUILD",
                    "BUILDROOT",
                    "RPMS",
                    "SOURCES",
                    "SPECS",
                    "SRPMS",
                ):
                    (rebuild / directory).mkdir(parents=True)
                subprocess.run(
                    [
                        "rpmbuild",
                        "--rebuild",
                        "--define",
                        f"_topdir {rebuild}",
                        str(source_rpm),
                    ],
                    check=True,
                    capture_output=True,
                    text=True,
                )
                rebuilt = rebuild / "RPMS/x86_64" / f"{package}-1.2.3-1.x86_64.rpm"
                self.assertTrue(rebuilt.is_file())
            for rpm_path, package, surface in (
                (cli_rpm, "kassiber-cli", "cli"),
                (desktop_rpm, "kassiber", "desktop"),
            ):
                metadata = subprocess.check_output(
                    [
                        "rpm",
                        "-qp",
                        "--queryformat",
                        "%{NAME} %{VERSION} %{ARCH}",
                        str(rpm_path),
                    ],
                    text=True,
                )
                self.assertEqual(metadata, f"{package} 1.2.3 x86_64")
                marker = json.loads(
                    rpm_file(rpm_path, "/usr/lib/kassiber/install-context.json")
                )
                self.assertEqual(marker["surface"], surface)
                self.assertEqual(marker["package_manager"], "rpm")
                self.assertEqual(marker["repository_manager"], "dnf")
                self.assertEqual(marker["repository_provenance"], "probe-required")

            cli_requires = subprocess.check_output(
                ["rpm", "-qp", "--requires", str(cli_rpm)], text=True
            )
            desktop_requires = subprocess.check_output(
                ["rpm", "-qp", "--requires", str(desktop_rpm)], text=True
            )
            self.assertIn("glibc", cli_requires)
            self.assertIn("zlib", cli_requires)
            self.assertIn("gtk3", desktop_requires)
            self.assertIn("webkit2gtk4.1", desktop_requires)

            repository = root / "repository"
            subprocess.run(
                [
                    str(ROOT / "scripts/build-rpm-repository.sh"),
                    "--input",
                    str(root),
                    "--output",
                    str(repository),
                    "--architecture",
                    "x86_64",
                    "--unsigned",
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertTrue((repository / "repodata/repomd.xml").is_file())
            self.assertEqual(
                sorted(path.name for path in (repository / "packages").glob("*.rpm")),
                [
                    "kassiber-1.2.3-1.x86_64.rpm",
                    "kassiber-cli-1.2.3-1.x86_64.rpm",
                ],
            )

            signing_tools = ("dnf", "gpg", "rpmsign")
            if not all(shutil.which(command) for command in signing_tools):
                return
            gnupg_home = root / "gnupg"
            gnupg_home.mkdir(mode=0o700)
            environment = os.environ.copy()
            environment["GNUPGHOME"] = str(gnupg_home)
            subprocess.run(
                [
                    "gpg",
                    "--batch",
                    "--pinentry-mode",
                    "loopback",
                    "--passphrase",
                    "",
                    "--quick-generate-key",
                    "Kassiber RPM Test <rpm-test@invalid.example>",
                    "rsa3072",
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
            signed_repository = root / "signed-repository"
            subprocess.run(
                [
                    str(ROOT / "scripts/build-rpm-repository.sh"),
                    "--input",
                    str(root),
                    "--output",
                    str(signed_repository),
                    "--architecture",
                    "x86_64",
                    "--signing-key",
                    fingerprint,
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
                env=environment,
            )
            self.assertTrue(
                (signed_repository / "repodata/repomd.xml.asc").is_file()
            )
            public_key = root / "kassiber-rpm-key.asc"
            public_key.write_bytes(
                subprocess.check_output(
                    ["gpg", "--batch", "--armor", "--export", fingerprint],
                    env=environment,
                )
            )
            rpmdb = root / "rpmdb"
            rpmdb.mkdir()
            subprocess.run(["rpm", "--dbpath", str(rpmdb), "--initdb"], check=True)
            subprocess.run(
                ["rpmkeys", "--dbpath", str(rpmdb), "--import", str(public_key)],
                check=True,
            )
            for package in (signed_repository / "packages").glob("*.rpm"):
                subprocess.run(
                    [
                        "rpmkeys",
                        "--dbpath",
                        str(rpmdb),
                        "--checksig",
                        str(package),
                    ],
                    check=True,
                    capture_output=True,
                    text=True,
                )

            if os.environ.get("KASSIBER_RPM_LIFECYCLE") != "1":
                return
            if os.geteuid() != 0:
                self.fail("KASSIBER_RPM_LIFECYCLE=1 requires a disposable root container")
            repo_file = Path("/etc/yum.repos.d/kassiber-test.repo")
            repo_file.write_text(
                "\n".join(
                    (
                        "[kassiber-test]",
                        "name=Kassiber packaging test",
                        f"baseurl=file://{signed_repository}",
                        "enabled=1",
                        "gpgcheck=1",
                        "repo_gpgcheck=1",
                        f"gpgkey=file://{public_key}",
                        "",
                    )
                ),
                encoding="utf-8",
            )
            dnf = [
                "dnf",
                "-y",
                "--enablerepo=kassiber-test",
            ]
            subprocess.run([*dnf, "install", "kassiber-cli"], check=True)
            installed_marker = Path("/usr/lib/kassiber/install-context.json")
            self.assertEqual(json.loads(installed_marker.read_text())["surface"], "cli")
            subprocess.run([*dnf, "swap", "kassiber-cli", "kassiber"], check=True)
            self.assertEqual(
                json.loads(installed_marker.read_text())["surface"], "desktop"
            )
            subprocess.run([*dnf, "remove", "kassiber"], check=True)
            self.assertFalse(installed_marker.exists())
            repo_file.unlink()

    def test_rpm_builders_reject_mismatched_versions(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            completed = subprocess.run(
                [
                    str(ROOT / "scripts/package-desktop-rpm.sh"),
                    "--deb",
                    str(build_desktop_deb(root, version="1.2.3")),
                    "--version",
                    "1.2.4",
                    "--output",
                    str(root / "kassiber.rpm"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("does not match", completed.stderr)

    def test_desktop_builder_rejects_mismatched_architectures(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            completed = subprocess.run(
                [
                    str(ROOT / "scripts/package-desktop-rpm.sh"),
                    "--deb",
                    str(build_desktop_deb(root)),
                    "--version",
                    "1.2.3",
                    "--architecture",
                    "aarch64",
                    "--output",
                    str(root / "kassiber.rpm"),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            self.assertEqual(completed.returncode, 2)
            self.assertIn("does not match", completed.stderr)


if __name__ == "__main__":
    unittest.main()
