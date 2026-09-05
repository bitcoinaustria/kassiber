from pathlib import Path
import tempfile
import unittest

from tests.integration.demo_fingerprint import demo_fingerprint


class DemoFingerprintTests(unittest.TestCase):
    def test_rebuild_follows_generator_and_recipe_but_not_location_or_docs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "dev/regtest").mkdir(parents=True)
            (root / "tests/integration").mkdir(parents=True)
            harness = root / "scripts/integration-harness.sh"
            harness.write_text("generator")
            recipe = root / "recipe.json"
            recipe.write_text('{"cycles": 1}')
            initial = demo_fingerprint(root, recipe)
            copied = root / "copy.json"
            copied.write_bytes(recipe.read_bytes())
            self.assertEqual(initial, demo_fingerprint(root, copied))
            (root / "dev/regtest/README.md").write_text("Explanation")
            self.assertEqual(initial, demo_fingerprint(root, recipe))
            generator = root / "tests/integration/regtest_exchange_cases.py"
            generator.write_text("new exchange case")
            with_generator = demo_fingerprint(root, recipe)
            self.assertNotEqual(initial, with_generator)
            generator.write_text("fixed exchange case")
            self.assertNotEqual(with_generator, demo_fingerprint(root, recipe))
            unchanged_generator = demo_fingerprint(root, recipe)
            recipe.write_text('{"cycles": 2}')
            self.assertNotEqual(unchanged_generator, demo_fingerprint(root, recipe))
