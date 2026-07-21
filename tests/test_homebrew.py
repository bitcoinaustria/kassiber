import importlib.util
import unittest
from pathlib import Path


def load_renderer():
    script = Path(__file__).resolve().parents[1] / "scripts" / "render_homebrew.py"
    spec = importlib.util.spec_from_file_location("render_homebrew", script)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load render_homebrew.py")
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
            'url "https://github.com/bitcoinaustria/kassiber/releases/download/v#{version}/kassiber-macos-arm64.dmg"',
            cask,
        )
        self.assertIn("depends_on arch: :arm64", cask)
        self.assertIn('app "Kassiber.app"', cask)
        self.assertIn(
            'binary "#{appdir}/Kassiber.app/Contents/Resources/bin/kassiber",',
            cask,
        )
        self.assertIn('target: "kassiber"', cask)

    def test_render_cask_warns_about_cli_formula_overlap(self):
        renderer = load_renderer()
        cask = renderer.render_cask("v0.22.9", "a" * 64)

        # Cask conflicts_with only accepts cask:, so the CLI-formula overlap
        # must surface as a caveat rather than a conflicts_with stanza.
        self.assertNotIn("conflicts_with", cask)
        self.assertIn("caveats", cask)
        self.assertIn("bitcoinaustria/kassiber/kassiber-cli", cask)

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


class HomebrewCliFormulaRenderTest(unittest.TestCase):
    def render(self, renderer, **overrides):
        kwargs = {
            "version": "v0.22.9",
            "sha256_macos_arm64": "a" * 64,
            "sha256_linux_x64": "c" * 64,
        }
        kwargs.update(overrides)
        return renderer.render_cli_formula(**kwargs)

    def test_render_formula_covers_each_published_cli_archive(self):
        renderer = load_renderer()
        formula = self.render(renderer)

        self.assertIn("class KassiberCli < Formula", formula)
        self.assertIn('version "0.22.9"', formula)
        self.assertIn('license "AGPL-3.0-only"', formula)
        for artifact, sha256 in (
            ("kassiber-cli-macos-arm64.tar.gz", "a" * 64),
            ("kassiber-cli-linux-x64.tar.gz", "c" * 64),
        ):
            self.assertIn(
                f'url "https://github.com/bitcoinaustria/kassiber/releases/download/v#{{version}}/{artifact}"',
                formula,
            )
            self.assertIn(f'sha256 "{sha256}"', formula)
        # Intel macOS builds are intentionally dropped (arm64-only Macs).
        self.assertNotIn("kassiber-cli-macos-x64", formula)
        self.assertNotIn("on_intel", formula.split("on_linux")[0])

    def test_render_formula_installs_frozen_cli_and_warns_about_cask_overlap(self):
        renderer = load_renderer()
        formula = self.render(renderer)

        self.assertIn('bin.install "kassiber"', formula)
        # The Formula DSL has no cask conflicts, so the overlap must surface
        # as a caveat while the cask side declares the machine-readable
        # conflicts_with.
        self.assertNotIn("conflicts_with", formula)
        self.assertIn("def caveats", formula)
        self.assertIn("bitcoinaustria/kassiber/kassiber", formula)
        self.assertIn('shell_output("#{bin}/kassiber --version")', formula)

    def test_render_formula_validates_each_checksum(self):
        renderer = load_renderer()

        for field in ("sha256_macos_arm64", "sha256_linux_x64"):
            with self.assertRaises(ValueError):
                self.render(renderer, **{field: "not-a-sha"})

    def test_render_formula_validates_version(self):
        renderer = load_renderer()

        with self.assertRaises(ValueError):
            self.render(renderer, version="not a version")


class PrereleaseWorkflowHomebrewTest(unittest.TestCase):
    def workflow_text(self):
        root = Path(__file__).resolve().parents[1]
        return (root / ".github" / "workflows" / "prerelease-binaries.yml").read_text(
            encoding="utf-8"
        )

    def test_prerelease_workflow_is_arm64_only_on_macos(self):
        workflow = self.workflow_text()

        # macOS ships arm64-only: no Intel runner, no universal target, no
        # second Apple sidecar.
        for forbidden in (
            "macos-15-intel",
            "x86_64-apple-darwin",
            "universal-apple-darwin",
            "macos-universal",
            "macos-x64",
        ):
            self.assertNotIn(forbidden, workflow)
        self.assertIn("target: macos-arm64", workflow)
        self.assertIn("tauri_args: --target aarch64-apple-darwin", workflow)
        self.assertIn(
            "sidecar_artifact_pattern: kassiber-desktop-sidecar-aarch64-apple-darwin",
            workflow,
        )
        self.assertIn("release_sha256 kassiber-macos-arm64.dmg", workflow)
        self.assertIn("triple: x86_64-unknown-linux-gnu", workflow)
        self.assertIn("triple: x86_64-pc-windows-msvc", workflow)

    def test_prerelease_workflow_publishes_cask_and_cli_formula_together(self):
        workflow = self.workflow_text()

        self.assertIn("HOMEBREW_CASK_PATH: Casks/kassiber.rb", workflow)
        self.assertIn("HOMEBREW_FORMULA_PATH: Formula/kassiber-cli.rb", workflow)
        self.assertIn("render_homebrew.py cask", workflow)
        self.assertIn("render_homebrew.py cli-formula", workflow)
        self.assertNotIn("render_homebrew_cask.py", workflow)
        self.assertIn('git add "$HOMEBREW_CASK_PATH" "$HOMEBREW_FORMULA_PATH"', workflow)


if __name__ == "__main__":
    unittest.main()
