from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

from kassiber import __version__


_ROOT = Path(__file__).resolve().parent.parent
_RP2_PIN_RE = re.compile(r"bitcoinaustria/rp2\.git@(?P<rev>[0-9a-f]{40})")
_VERSION_RE = re.compile(r'(?m)^version\s*=\s*"([^"]+)"')


def _toml_table_body(text: str, table: str) -> str:
    """Return the body of a top-level TOML table, scoped so per-dependency
    `version = "..."` entries elsewhere in the file cannot be mistaken for the
    package version."""

    pattern = re.compile(
        rf"(?ms)^\[{re.escape(table)}\]\s*\n(?P<body>.*?)(?=^\[|\Z)"
    )
    match = pattern.search(text)
    if match is None:
        raise AssertionError(f"TOML table [{table}] not found")
    return match.group("body")


def _version_in_table(path: Path, table: str) -> str:
    text = path.read_text(encoding="utf-8")
    body = _toml_table_body(text, table)
    match = _VERSION_RE.search(body)
    if match is None:
        raise AssertionError(f"[{table}] table in {path.name} does not declare a version")
    return match.group(1)


def _rp2_pin_from_pyproject() -> str:
    text = (_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    match = _RP2_PIN_RE.search(text)
    if match:
        return match.group("rev")
    raise AssertionError("pyproject.toml does not pin bitcoinaustria/rp2 to a commit")


def _project_version_from_pyproject() -> str:
    return _version_in_table(_ROOT / "pyproject.toml", "project")


def _rp2_pin_from_uv_lock() -> str:
    text = (_ROOT / "uv.lock").read_text(encoding="utf-8")
    match = re.search(
        r'(?ms)^\[\[package\]\]\s*name\s*=\s*"rp2".*?'
        r'source\s*=\s*\{[^}]*#(?P<rev>[0-9a-f]{40})"',
        text,
    )
    if match:
        return match.group("rev")
    raise AssertionError("uv.lock does not pin bitcoinaustria/rp2 to a commit")


def _rp2_pin_from_license_notes() -> str:
    text = (_ROOT / "THIRD_PARTY_LICENSES.md").read_text(encoding="utf-8")
    match = _RP2_PIN_RE.search(text)
    if match:
        return match.group("rev")
    raise AssertionError("THIRD_PARTY_LICENSES.md does not mention the pinned rp2 commit")


class DependencyDriftTests(unittest.TestCase):
    def test_rp2_pin_is_consistent_across_dependency_metadata(self):
        pyproject_pin = _rp2_pin_from_pyproject()
        self.assertEqual(_rp2_pin_from_uv_lock(), pyproject_pin)
        self.assertEqual(_rp2_pin_from_license_notes(), pyproject_pin)

    def test_app_version_is_consistent_across_package_metadata(self):
        pyproject_version = _project_version_from_pyproject()
        package_json = json.loads(
            (_ROOT / "ui-tauri" / "package.json").read_text(encoding="utf-8")
        )
        tauri_config = json.loads(
            (_ROOT / "ui-tauri" / "src-tauri" / "tauri.conf.json").read_text(
                encoding="utf-8"
            )
        )
        cargo_version = _version_in_table(
            _ROOT / "ui-tauri" / "src-tauri" / "Cargo.toml", "package"
        )

        self.assertEqual(__version__, pyproject_version)
        self.assertEqual(package_json["version"], pyproject_version)
        self.assertEqual(tauri_config["version"], pyproject_version)
        self.assertEqual(cargo_version, pyproject_version)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
