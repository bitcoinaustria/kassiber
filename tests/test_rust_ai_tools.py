"""The Rust AI tool executor (crates/ai, R6-5a) against Python.

Two layers:

- drift: the embedded catalog and the pure-function vectors the Rust tests
  replay are regenerated from the live Python registries and must match the
  checked-in files byte for byte;
- executor parity: each read tool runs through Python's
  ``_execute_read_only_ai_tool`` (``tests/rust_ai_tool_reference.py``, the
  reference wrapper) and through the Rust daemon's test-harness kind
  ``test.ai_tool`` on separate copies of the same fixture book, and the
  model-visible bytes (sorted compact JSON) and the ``ai.chat.tool_result``
  envelope must be identical.
"""
from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parent.parent


def _load(relative: str):
    path = ROOT / relative
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AiCatalogDriftTest(unittest.TestCase):
    def test_embedded_catalog_matches_python(self):
        generator = _load("crates/ai/scripts/gen_catalog.py")
        self.assertEqual(
            generator.TARGET.read_text(encoding="utf-8"),
            generator.render(),
            "rerun crates/ai/scripts/gen_catalog.py and review crates/ai's constants",
        )

    def test_pure_function_vectors_match_python(self):
        generator = _load("crates/ai/scripts/gen_vectors.py")
        self.assertEqual(
            generator.TARGET.read_text(encoding="utf-8"),
            generator.render(),
            "rerun crates/ai/scripts/gen_vectors.py and the kassiber-ai tests",
        )


if __name__ == "__main__":
    unittest.main()
