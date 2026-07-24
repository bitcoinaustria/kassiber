import os
import shutil
import subprocess
import tempfile
import time
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


class RpmVersionContractTest(unittest.TestCase):
    def test_rpm_builders_escape_the_literal_tilde_replacement(self):
        for script_name in ("package-cli-rpm.sh", "package-desktop-rpm.sh"):
            script = (ROOT / "scripts" / script_name).read_text(encoding="utf-8")
            self.assertIn('rpm_version="${version//-/\\~}"', script)
        workflow = (
            ROOT / ".github/workflows/publish-linux-channels.yml"
        ).read_text(encoding="utf-8")
        self.assertIn('rpm_version="${version//-/\\~}"', workflow)


class RpmRepositoryScriptSafetyTest(unittest.TestCase):
    def test_failed_package_discovery_cannot_publish_partial_input(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = root / "input"
            inputs.mkdir()
            partial_package = inputs / "partial.rpm"
            partial_package.write_text("not an rpm", encoding="utf-8")
            fake_bin = root / "bin"
            fake_bin.mkdir()

            for command in (
                "awk",
                "cmp",
                "cpio",
                "createrepo_c",
                "gpg",
                "rpm",
                "rpm2cpio",
                "rpmkeys",
            ):
                shim = fake_bin / command
                shim.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                shim.chmod(0o755)
            find = fake_bin / "find"
            find.write_text(
                "#!/bin/sh\n"
                "printf '%s\\0' \"$1/partial.rpm\"\n"
                "exit 23\n",
                encoding="utf-8",
            )
            find.chmod(0o755)

            output = root / "repository"
            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
            completed = subprocess.run(
                [
                    str(ROOT / "scripts/build-rpm-repository.sh"),
                    "--input",
                    str(inputs),
                    "--output",
                    str(output),
                    "--unsigned",
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                env=environment,
            )

            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("Failed to discover binary RPM packages", completed.stderr)
            self.assertFalse(output.exists())

    def test_concurrent_publication_collision_fails_without_nesting(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            inputs = root / "input"
            inputs.mkdir()
            package = inputs / "kassiber-cli.rpm"
            package.write_text("test rpm", encoding="utf-8")
            fake_bin = root / "bin"
            fake_bin.mkdir()

            for command in (
                "awk",
                "cmp",
                "cpio",
                "gpg",
                "rpm2cpio",
                "rpmkeys",
            ):
                shim = fake_bin / command
                shim.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                shim.chmod(0o755)
            rpm = fake_bin / "rpm"
            rpm.write_text(
                "#!/bin/sh\n"
                "case \"$*\" in\n"
                "  *'%{NAME}'*) printf 'kassiber-cli' ;;\n"
                "  *'%{VERSION}'*) printf '1.2.3' ;;\n"
                "  *'%{RELEASE}'*) printf '1' ;;\n"
                "  *'%{ARCH}'*) printf 'x86_64' ;;\n"
                "  *) exit 2 ;;\n"
                "esac\n",
                encoding="utf-8",
            )
            rpm.chmod(0o755)
            createrepo = fake_bin / "createrepo_c"
            createrepo.write_text(
                "#!/bin/sh\n"
                "for argument in \"$@\"; do repository=$argument; done\n"
                "mkdir -p \"$repository/repodata\"\n"
                "printf 'metadata\\n' > \"$repository/repodata/repomd.xml\"\n",
                encoding="utf-8",
            )
            createrepo.chmod(0o755)

            ready = root / "ready"
            resume = root / "resume"
            real_mv = shutil.which("mv")
            self.assertIsNotNone(real_mv)
            mv = fake_bin / "mv"
            mv.write_text(
                "#!/bin/sh\n"
                "printf ready > \"$KASSIBER_TEST_READY\"\n"
                "while [ ! -e \"$KASSIBER_TEST_RESUME\" ]; do sleep 0.01; done\n"
                f'exec "{real_mv}" "$@"\n',
                encoding="utf-8",
            )
            mv.chmod(0o755)

            output = root / "repository"
            environment = os.environ.copy()
            environment["PATH"] = f"{fake_bin}:{environment['PATH']}"
            environment["KASSIBER_TEST_READY"] = str(ready)
            environment["KASSIBER_TEST_RESUME"] = str(resume)
            process = subprocess.Popen(
                [
                    str(ROOT / "scripts/build-rpm-repository.sh"),
                    "--input",
                    str(inputs),
                    "--output",
                    str(output),
                    "--unsigned",
                ],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                env=environment,
            )
            deadline = time.monotonic() + 5
            while (
                not ready.exists()
                and process.poll() is None
                and time.monotonic() < deadline
            ):
                time.sleep(0.01)
            if not ready.exists():
                resume.touch()
                stdout, stderr = process.communicate(timeout=5)
                self.fail(
                    "builder did not reach publication barrier: "
                    f"stdout={stdout!r}, stderr={stderr!r}"
                )

            output.mkdir()
            resume.touch()
            stdout, stderr = process.communicate(timeout=5)

            self.assertNotEqual(process.returncode, 0, stdout)
            self.assertIn("Output path appeared during repository build", stderr)
            self.assertEqual(list(output.iterdir()), [])
            self.assertEqual(list(root.glob("repository.tmp.*")), [])


@unittest.skipUnless(
    all(shutil.which(command) for command in RPM_TOOLS),
    "RPM, repository, and Debian tooling is required",
)
class RpmPackagingTest(unittest.TestCase):
    def test_builds_cli_and_desktop_rpms(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            cli_rpm = root / "kassiber-cli.rpm"
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
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            desktop_rpm = root / "kassiber.rpm"
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
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )

            for rpm_path, package in (
                (cli_rpm, "kassiber-cli"),
                (desktop_rpm, "kassiber"),
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
            cli_binary = Path("/usr/bin/kassiber")
            desktop_binary = Path("/usr/bin/kassiber-ui")
            self.assertTrue(cli_binary.exists())
            self.assertFalse(desktop_binary.exists())
            subprocess.run([*dnf, "swap", "kassiber-cli", "kassiber"], check=True)
            self.assertTrue(desktop_binary.exists())
            subprocess.run([*dnf, "remove", "kassiber"], check=True)
            self.assertFalse(desktop_binary.exists())
            repo_file.unlink()

    def test_builds_cli_and_desktop_rpms_for_semver_prereleases(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            version = "1.2.3-rc.1"
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            cli_rpm = root / "kassiber-cli-prerelease.rpm"
            desktop_rpm = root / "kassiber-prerelease.rpm"

            subprocess.run(
                [
                    str(ROOT / "scripts/package-cli-rpm.sh"),
                    "--binary",
                    str(binary),
                    "--version",
                    version,
                    "--architecture",
                    "x86_64",
                    "--output",
                    str(cli_rpm),
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )
            subprocess.run(
                [
                    str(ROOT / "scripts/package-desktop-rpm.sh"),
                    "--deb",
                    str(build_desktop_deb(root, version=version)),
                    "--version",
                    version,
                    "--architecture",
                    "x86_64",
                    "--output",
                    str(desktop_rpm),
                ],
                check=True,
                cwd=ROOT,
                capture_output=True,
                text=True,
            )

            for rpm_path in (cli_rpm, desktop_rpm):
                evr = subprocess.check_output(
                    [
                        "rpm",
                        "-qp",
                        "--queryformat",
                        "%{VERSION}-%{RELEASE}",
                        str(rpm_path),
                    ],
                    text=True,
                )
                self.assertEqual(evr, "1.2.3~rc.1-1")

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
