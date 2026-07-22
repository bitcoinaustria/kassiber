import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class LocalMacosPackagerTest(unittest.TestCase):
    def test_app_has_a_distinct_prerelease_identity(self):
        packager = (ROOT / "scripts/build-macos-arm64-app.sh").read_text(
            encoding="utf-8"
        )
        release_config = json.loads(
            (ROOT / "ui-tauri/src-tauri/tauri.conf.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertIn('APP_PRODUCT_NAME="Kassiber Dev"', packager)
        self.assertIn(
            'APP_IDENTIFIER="at.bitcoinaustria.kassiber.dev"', packager
        )
        self.assertIn('"productName": "%s"', packager)
        self.assertIn('"identifier": "%s"', packager)
        self.assertIn('macos/$APP_PRODUCT_NAME.app', packager)
        self.assertEqual(release_config["productName"], "Kassiber")
        self.assertEqual(release_config["identifier"], "at.bitcoinaustria.kassiber")


if __name__ == "__main__":
    unittest.main()
