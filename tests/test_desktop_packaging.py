import json
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class DesktopPackagingTest(unittest.TestCase):
    def test_desktop_installers_integrate_the_cli_without_autostart(self):
        config = json.loads(
            (ROOT / "ui-tauri/src-tauri/tauri.conf.json").read_text(encoding="utf-8")
        )
        bundle = config["bundle"]

        self.assertEqual(
            bundle["linux"]["deb"]["files"]["/usr/bin/kassiber"],
            "bin/kassiber-linux",
        )
        self.assertEqual(bundle["windows"]["nsis"]["installMode"], "currentUser")
        self.assertIn("windows/update-path.ps1", bundle["resources"])
        self.assertIn(
            "KassiberCliPath", bundle["windows"]["wix"]["componentRefs"]
        )

        hooks = (ROOT / "ui-tauri/src-tauri/windows/installer-hooks.nsh").read_text(
            encoding="utf-8"
        )
        self.assertIn("NSIS_HOOK_POSTINSTALL", hooks)
        self.assertIn("NSIS_HOOK_PREUNINSTALL", hooks)
        self.assertIn("-Action add", hooks)
        self.assertIn("-Action remove", hooks)
        self.assertNotIn("autorun", hooks.lower())
        self.assertNotIn("startup", hooks.lower())

    def test_wix_owns_and_removes_only_its_path_entry(self):
        fragment = ROOT / "ui-tauri/src-tauri/windows/fragments/cli-path.wxs"
        tree = ET.parse(fragment)
        namespace = {"w": "http://schemas.microsoft.com/wix/2006/wi"}
        environment = tree.find(".//w:Environment", namespace)
        self.assertIsNotNone(environment)
        assert environment is not None
        self.assertEqual(environment.attrib["Name"], "PATH")
        self.assertEqual(environment.attrib["Value"], "[INSTALLDIR]bin")
        self.assertEqual(environment.attrib["Permanent"], "no")

    def test_cli_release_and_desktop_sidecar_share_one_build(self):
        workflow = (
            ROOT / ".github/workflows/prerelease-binaries.yml"
        ).read_text(encoding="utf-8")

        self.assertEqual(workflow.count("pyinstaller \\"), 1)
        self.assertNotIn("build-desktop-sidecar:", workflow)
        self.assertIn("kassiber-cli-release-${{ matrix.target }}", workflow)
        self.assertIn("target: windows-x64", workflow)
        self.assertIn('archive: zip', workflow)
        self.assertIn("bundles: deb,appimage", workflow)
        self.assertIn("release_channel:", workflow)
        self.assertIn("inputs.release_channel == 'prerelease'", workflow)
        self.assertIn('"$cli" --version', workflow)
        self.assertIn("Smoke desktop terminal forwarding", workflow)

    def test_windows_bundle_launcher_executes_the_console_sidecar(self):
        launcher = (ROOT / "ui-tauri/src-tauri/bin/kassiber.cmd").read_text(
            encoding="utf-8"
        )
        self.assertIn("kassiber-cli-x86_64-pc-windows-msvc.exe", launcher)
        self.assertIn('"%KASSIBER_SIDECAR%" %*', launcher)
        self.assertNotIn("start ", launcher.lower())

    def test_local_macos_installer_uses_the_settings_managed_marker(self):
        installer = (ROOT / "scripts/install-macos-desktop-cli.sh").read_text(
            encoding="utf-8"
        )
        rust = (ROOT / "ui-tauri/src-tauri/src/lib.rs").read_text(encoding="utf-8")
        marker = "Kassiber desktop CLI launcher. Managed by Kassiber Settings."
        self.assertIn(marker, installer)
        self.assertIn(marker, rust)
        self.assertIn("AppTranslocation", installer)
        self.assertNotIn("autostart", installer.lower())


if __name__ == "__main__":
    unittest.main()
