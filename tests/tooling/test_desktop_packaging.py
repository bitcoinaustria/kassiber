import json
import os
import re
import shutil
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path



ROOT = Path(__file__).resolve().parents[2]
# The packaged helper the operator's native auth execs (the Rust constant).
_MACOS_APP_EXECUTABLE_NAME = re.search(
    r'pub const MACOS_APP_EXECUTABLE_NAME: &str = "([^"]+)";',
    (ROOT / "crates/operator/src/native_auth.rs").read_text(encoding="utf-8"),
).group(1)


class DesktopPackagingTest(unittest.TestCase):
    def test_desktop_installers_integrate_the_cli_without_autostart(self):
        packaging = ROOT / "crates/kassiber-desktop/packaging"
        linux = (packaging / "linux/build-packages.sh").read_text(encoding="utf-8")
        control = (packaging / "linux/debian/control.in").read_text(encoding="utf-8")
        forwarder = (packaging / "linux/kassiber").read_text(encoding="utf-8")
        msi = (packaging / "windows/kassiber.wxs").read_text(encoding="utf-8")
        nsis = (packaging / "windows/kassiber.nsi").read_text(encoding="utf-8")

        self.assertIn(
            'install -m 0755 "$here/kassiber" "$root/usr/bin/kassiber"', linux
        )
        self.assertIn('exec /usr/bin/kassiber-ui --cli "$@"', forwarder)
        self.assertIn("Conflicts: kassiber-cli", control)
        self.assertIn("Replaces: kassiber-cli", control)
        self.assertIn("Provides: kassiber-command", control)
        self.assertNotIn("libwebkit", control)
        self.assertNotIn("libgtk", control)
        self.assertIn('InstallDir "$LOCALAPPDATA\\Kassiber"', nsis)
        self.assertIn("RequestExecutionLevel user", nsis)
        self.assertIn('<ComponentRef Id="KassiberCliPath" />', msi)
        # The Tauri MSI's upgrade code, so an installed Tauri build is replaced.
        self.assertIn('UpgradeCode="f3c714a3-dc4a-53c3-ad68-fbede17c4009"', msi)
        self.assertIn("-Action add", nsis)
        self.assertIn("-Action remove", nsis)
        for text in (nsis, msi):
            self.assertNotIn("autorun", text.lower())
            self.assertNotIn("startup", text.lower())

    def test_wix_owns_and_removes_only_its_path_entry(self):
        fragment = ROOT / "crates/kassiber-desktop/packaging/windows/cli-path.wxs"
        tree = ET.parse(fragment)
        namespace = {"w": "http://schemas.microsoft.com/wix/2006/wi"}
        environment = tree.find(".//w:Environment", namespace)
        self.assertIsNotNone(environment)
        assert environment is not None
        self.assertEqual(environment.attrib["Name"], "PATH")
        self.assertEqual(environment.attrib["Value"], "[INSTALLDIR]bin")
        self.assertEqual(environment.attrib["Permanent"], "no")
        installers = (
            ROOT / "crates/kassiber-desktop/packaging/windows/build-installers.sh"
        ).read_text(encoding="utf-8")
        self.assertIn('"$here/cli-path.wxs"', installers)

    def test_cli_release_and_desktop_sidecar_are_one_rust_binary(self):
        """The downloadable CLI and the desktop sidecar are the same file.

        Plan 21 stage 7: every leg builds the Rust `kassiber` binary once with
        scripts/build-rust-cli.sh, smokes it, packages it as the CLI, and
        stages that exact file as the sidecar, with the build record beside
        it. No PyInstaller build may come back, and no Node provider-broker
        bundle either: the binary serves the broker itself (plan 23).
        """
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")
        smoke = (ROOT / "scripts/smoke-packaged-cli.sh").read_text(encoding="utf-8")
        build = (ROOT / "scripts/build-rust-cli.sh").read_text(encoding="utf-8")

        self.assertNotIn("pyinstaller", workflow.lower())
        self.assertNotIn("--onedir", workflow)
        self.assertEqual(workflow.count("scripts/build-rust-cli.sh \\"), 1)
        self.assertIn('--target "${{ matrix.triple }}"', workflow)
        self.assertIn("scripts/smoke-packaged-cli.sh", workflow)
        self.assertIn("scripts/check-packaged-cli-no-egress.sh", workflow)
        self.assertIn("scripts/package-cli-archive.sh", workflow)
        # The sidecar is the smoked CLI binary, copied, never rebuilt.
        self.assertIn(
            'cp "dist/kassiber${{ matrix.exe_ext }}" \\\n'
            '            "$sidecar_dir/kassiber-cli-${{ matrix.triple }}${{ matrix.exe_ext }}"',
            workflow,
        )
        self.assertNotIn("provider_broker", workflow)
        self.assertNotIn("broker:build", workflow)
        self.assertIn('"$sidecar_dir/BUILD_INFO.json"', workflow)
        self.assertIn('test ! -e "$root/_internal"', workflow)
        # The production binary: never the test harness.
        self.assertNotIn("--features", build)
        self.assertNotIn("--all-features", build)
        self.assertIn("-p kassiber --bin kassiber", build)
        self.assertIn("check-linux-glibc-floor.sh", build)

        self.assertNotIn("build-desktop-sidecar:", workflow)
        self.assertIn("kassiber-cli-release-${{ matrix.target }}", workflow)
        self.assertIn("target: windows-x64", workflow)
        self.assertIn('archive: zip', workflow)
        self.assertIn("os: ubuntu-22.04", workflow)
        self.assertIn("crates/kassiber-desktop/packaging/linux/build-packages.sh", workflow)
        self.assertIn("crates/kassiber-desktop/packaging/macos/build-app.sh", workflow)
        self.assertIn("crates/kassiber-desktop/packaging/windows/build-installers.sh", workflow)
        # The shipped app never carries Rune's agent tooling.
        self.assertIn("cargo build --release --locked --no-default-features", workflow)
        self.assertNotIn("tauri", workflow.lower())
        self.assertIn("scripts/package-cli-deb.sh", workflow)
        self.assertIn('package_commit="$(git rev-parse HEAD)"', workflow)
        self.assertNotIn('--commit "$GITHUB_SHA"', workflow)
        self.assertIn("release_channel:", workflow)
        self.assertIn("inputs.release_channel == 'prerelease'", workflow)
        self.assertIn("pull_request:", workflow)
        self.assertIn("BUILD_CHANNEL:", workflow)
        self.assertIn('--channel "$BUILD_CHANNEL"', workflow)
        self.assertIn('"$appimage" --cli --version | grep "$expected_commit"', workflow)
        self.assertIn('"$launcher" --version | grep "$expected_commit"', workflow)
        self.assertIn("--operator-unlock", workflow)

        self.assertIn('kassiber --version', smoke)
        self.assertIn("error.code not_initialized", smoke)
        self.assertIn('kassiber --data-root "$status_root" --machine init', smoke)
        self.assertIn("Hosted Linux runners have no live logind user session", smoke)
        self.assertIn("--db-passphrase-fd 0 status", smoke)
        self.assertIn("--machine operator unlock", smoke)
        self.assertIn("SQLite format 3", smoke)
        self.assertIn("KASSIBER_OFFLINE_PREFERENCE_FILE", smoke)

    def test_appimage_uses_host_display_libraries_and_a_pinned_runtime(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")
        packages = (
            ROOT / "crates/kassiber-desktop/packaging/linux/build-packages.sh"
        ).read_text(encoding="utf-8")

        # Nothing display- or GPU-related is bundled: the Wayland client, the
        # Vulkan loader and xkbcommon come from the host.
        appdir = packages[packages.index('appdir="$work/Kassiber.AppDir"'):]
        for library in ("libwayland", "libvulkan", "libxkbcommon.so"):
            self.assertNotIn(f"cp {library}", appdir)
            self.assertNotIn(f"/{library}", appdir)
        # appimagetool never fetches its own runtime.
        self.assertIn('--runtime-file "$appimage_runtime"', packages)
        self.assertIn("--appimage-runtime PATH is required", packages)
        self.assertIn('--appimage-runtime "$APPIMAGE_RUNTIME"', workflow)
        self.assertRegex(workflow, r"APPIMAGETOOL_SHA256: [0-9a-f]{64}")
        self.assertRegex(workflow, r"APPIMAGE_RUNTIME_SHA256: [0-9a-f]{64}")
        self.assertIn("| sha256sum -c -", workflow)

    def test_windows_bundle_stages_a_native_cli_executable(self):
        windows = ROOT / "crates/kassiber-desktop/packaging/windows"
        installers = (windows / "build-installers.sh").read_text(encoding="utf-8")
        terminal = (
            ROOT / "crates/kassiber-desktop/src/native/terminal.rs"
        ).read_text(encoding="utf-8")
        path_script = (windows / "update-path.ps1").read_text(encoding="utf-8")

        # The PATH entry runs a native copy, so cmd.exe never reparses arguments.
        self.assertIn('cp "$kassiber_bin" "$stage/bin/kassiber.exe"', installers)
        self.assertIn('"$stage/bin/kassiber.exe" --version', installers)
        self.assertNotIn("kassiber.cmd", installers)
        self.assertIn('"kassiber.exe"', terminal)
        self.assertNotIn("%*", terminal)
        self.assertIn('"kassiber.cmd"', path_script)
        self.assertIn("Remove-Item", path_script)

    def test_macos_bundle_launcher_executes_the_console_sidecar_without_appkit(self):
        launcher = (
            ROOT / "crates/kassiber-desktop/packaging/macos/kassiber-launcher"
        ).read_text(encoding="utf-8")
        self.assertIn("kassiber-cli-aarch64-apple-darwin", launcher)
        self.assertIn("kassiber-cli-x86_64-apple-darwin", launcher)
        self.assertIn(
            f"app_executable=$contents_dir/MacOS/{_MACOS_APP_EXECUTABLE_NAME}",
            launcher,
        )
        self.assertIn("KASSIBER_NATIVE_AUTH_HELPER", launcher)
        self.assertIn('exec "$sidecar" "$@"', launcher)
        self.assertNotIn('exec "$app_executable" --cli', launcher)

    def test_macos_bundle_launcher_preserves_cli_context_and_arguments(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            contents = root / "Kassiber.app/Contents"
            launcher = contents / "Resources/bin/kassiber"
            sidecar = (
                contents
                / "Resources/binaries/kassiber-cli-aarch64-apple-darwin"
            )
            app_executable = contents / "MacOS/kassiber-ui"
            scratch = root / "scratch"
            launcher.parent.mkdir(parents=True)
            sidecar.parent.mkdir(parents=True)
            app_executable.parent.mkdir(parents=True)
            scratch.mkdir()

            shutil.copy2(
                ROOT / "crates/kassiber-desktop/packaging/macos/kassiber-launcher",
                launcher,
            )
            sidecar.write_text(
                "#!/bin/sh\n"
                "printf '%s\\n' \"$KASSIBER_NATIVE_AUTH_HELPER\" \"$PWD\" \"$#\"\n"
                "printf '<%s>\\n' \"$@\"\n",
                encoding="utf-8",
            )
            sidecar.chmod(0o755)
            app_executable.write_text("#!/bin/sh\nexit 99\n", encoding="utf-8")
            app_executable.chmod(0o755)

            environment = os.environ.copy()
            environment["TMPDIR"] = str(scratch)
            completed = subprocess.run(
                [str(launcher), "status", "--machine"],
                check=True,
                capture_output=True,
                text=True,
                env=environment,
                cwd=root,
            )

            self.assertEqual(
                completed.stdout.splitlines(),
                [
                    str(app_executable),
                    str(scratch),
                    "2",
                    "<status>",
                    "<--machine>",
                ],
            )
            self.assertEqual(completed.stderr, "")

    def test_local_macos_installer_delegates_to_the_rust_manager(self):
        installer = (ROOT / "scripts/install-macos-desktop-cli.sh").read_text(
            encoding="utf-8"
        )
        rust = (
            ROOT / "crates/kassiber-desktop/src/native/terminal.rs"
        ).read_text(encoding="utf-8")
        self.assertIn("--install-terminal-command", installer)
        self.assertIn("--install-terminal-command", rust)
        self.assertNotIn("PATH_MARKER_START", installer)
        self.assertNotIn("shell_quote", installer)
        self.assertIn("AppTranslocation", installer)
        self.assertNotIn("autostart", installer.lower())

    @unittest.skipUnless(shutil.which("dpkg-deb"), "dpkg-deb is required")
    def test_cli_only_deb_has_no_gui_dependencies_and_conflicts_cleanly(self):
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
            control = subprocess.check_output(
                ["dpkg-deb", "-f", str(package)], text=True
            )
            self.assertIn("Package: kassiber-cli", control)
            self.assertIn("Conflicts: kassiber", control)
            self.assertNotIn("libgtk", control)
            self.assertNotIn("libwebkit", control)
            listing = subprocess.check_output(
                ["dpkg-deb", "-c", str(package)], text=True
            )
            self.assertIn("./usr/bin/kassiber", listing)

    @unittest.skipUnless(shutil.which("dpkg-deb"), "dpkg-deb is required")
    def test_cli_only_deb_rejects_invalid_architectures_before_building(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            for index, architecture in enumerate(
                (
                    "amd64\nPre-Depends: unexpected-package",
                    "amd64 all",
                    "amd64_invalid",
                    "ämd64",
                )
            ):
                with self.subTest(architecture=architecture):
                    package = root / f"invalid-{index}.deb"
                    completed = subprocess.run(
                        [
                            str(ROOT / "scripts/package-cli-deb.sh"),
                            "--binary",
                            str(binary),
                            "--version",
                            "1.2.3",
                            "--architecture",
                            architecture,
                            "--output",
                            str(package),
                        ],
                        cwd=ROOT,
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(completed.returncode, 2)
                    self.assertFalse(package.exists())


    def test_cli_archive_lands_in_a_relative_output_dir_for_both_formats(self):
        # Release CI passes `--output-dir release`. The zip branch runs 7z from
        # inside its staging directory, so the archive path must already be
        # absolute there; a stand-in 7z records where it was told to write.
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            binary = root / "kassiber"
            binary.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            binary.chmod(0o755)
            build_info = root / "BUILD_INFO.json"
            build_info.write_text("{}\n", encoding="utf-8")
            tools = root / "tools"
            tools.mkdir()
            seven_zip = tools / "7z"
            seven_zip.write_text(
                '#!/bin/sh\n[ "$1 $2" = "a -tzip" ] || exit 9\n'
                'case "$3" in /*) ;; *) exit 8 ;; esac\n'
                '[ -d "$4" ] || exit 7\nprintf zip > "$3"\n',
                encoding="utf-8",
            )
            seven_zip.chmod(0o755)
            env = dict(os.environ, PATH=f"{tools}{os.pathsep}{os.environ['PATH']}")
            for archive_format, suffix in (("zip", ".zip"), ("tar", ".tar.gz")):
                with self.subTest(archive_format=archive_format):
                    work = root / f"work-{archive_format}"
                    work.mkdir()
                    completed = subprocess.run(
                        [
                            str(ROOT / "scripts/package-cli-archive.sh"),
                            "--binary",
                            str(binary),
                            "--build-info",
                            str(build_info),
                            "--name",
                            "kassiber-cli-test-x64",
                            "--format",
                            archive_format,
                            "--output-dir",
                            "release",
                        ],
                        cwd=work,
                        env=env,
                        capture_output=True,
                        text=True,
                    )
                    self.assertEqual(completed.returncode, 0, completed.stderr)
                    archive = work / "release" / f"kassiber-cli-test-x64{suffix}"
                    self.assertTrue(archive.is_file())
                    self.assertGreater(archive.stat().st_size, 0)
                    self.assertEqual(
                        Path(completed.stdout.strip()).resolve(), archive.resolve()
                    )


if __name__ == "__main__":
    unittest.main()
