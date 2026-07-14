import importlib.util
import unittest
from pathlib import Path


def load_renderer():
    script = Path(__file__).resolve().parents[1] / "scripts" / "render_homebrew_cask.py"
    spec = importlib.util.spec_from_file_location("render_homebrew_cask", script)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load render_homebrew_cask.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class HomebrewCaskRenderTest(unittest.TestCase):
    def test_render_cask_links_bundled_terminal_launcher(self):
        renderer = load_renderer()
        sha256 = "a" * 64
        cask = renderer.render_cask("v0.22.9", sha256)

        self.assertIn('version "0.22.9"', cask)
        self.assertIn(f'sha256 "{sha256}"', cask)
        self.assertIn(
            'url "https://github.com/bitcoinaustria/kassiber/releases/download/v#{version}/kassiber-macos-universal.dmg"',
            cask,
        )
        self.assertIn('app "Kassiber.app"', cask)
        self.assertIn(
            'binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",',
            cask,
        )
        self.assertIn('target: "kassiber"', cask)

    def test_render_cask_zap_does_not_remove_primary_data_root(self):
        renderer = load_renderer()
        cask = renderer.render_cask("v0.22.9", "a" * 64)
        zap_block = cask[cask.index("zap trash") :]

        self.assertNotIn("~/.kassiber", zap_block)
        self.assertIn("~/Library/Application Support/at.bitcoinaustria.kassiber", zap_block)

    def test_render_cask_validates_checksum(self):
        renderer = load_renderer()

        with self.assertRaises(ValueError):
            renderer.render_cask("0.22.9", "not-a-sha")

    def test_prerelease_workflow_keeps_universal_macos_support(self):
        root = Path(__file__).resolve().parents[1]
        workflow = (
            root / ".github" / "workflows" / "prerelease-binaries.yml"
        ).read_text(encoding="utf-8")

        for required in (
            "macos-15-intel",
            "x86_64-apple-darwin",
            "universal-apple-darwin",
            "macos-universal",
        ):
            self.assertIn(required, workflow)
        self.assertIn("target: macos-universal", workflow)
        self.assertIn("tauri_args: --target universal-apple-darwin", workflow)
        self.assertIn(
            "sidecar_artifact_pattern: kassiber-desktop-sidecar-*-apple-darwin",
            workflow,
        )
        self.assertIn('dmg_name="kassiber-macos-universal.dmg"', workflow)
        self.assertIn("triple: x86_64-unknown-linux-gnu", workflow)
        self.assertIn("triple: x86_64-pc-windows-msvc", workflow)


if __name__ == "__main__":
    unittest.main()
