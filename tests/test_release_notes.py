"""Version selection must never publish another release's notes."""
import importlib.util
from pathlib import Path

import pytest

spec = importlib.util.spec_from_file_location(
    "release_notes", Path(__file__).resolve().parents[1] / "scripts/release_notes.py")
notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notes)


def test_selects_exact_version_preserving_body():
    text = "# Changelog\n\n## 1.2.30\n\nOther\n\n## 1.2.3\n\n### Fixed\n\n- A & B.\n\n## 1.2.2\n\nOlder\n"
    assert notes.extract_notes(text, "v1.2.3") == "### Fixed\n\n- A & B.\n"


@pytest.mark.parametrize("text", ["# Changelog\n", "## 1.2.3\n\n", "## 1.2.3\nA\n## 1.2.3\nB\n"])
def test_missing_empty_duplicate_fail(text):
    with pytest.raises(ValueError):
        notes.extract_notes(text, "1.2.3")
