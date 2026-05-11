from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

from kassiber import __version__


_ROOT = Path(__file__).resolve().parent.parent
_RP2_PIN_RE = re.compile(r"bitcoinaustria/rp2\.git@(?P<rev>[0-9a-f]{40})")
_VERSION_RE = re.compile(r'(?m)^version\s*=\s*"([^"]+)"')


def _rp2_pin_from_pyproject() -> str:
    text = (_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    match = _RP2_PIN_RE.search(text)
    if match:
        return match.group("rev")
    raise AssertionError("pyproject.toml does not pin bitcoinaustria/rp2 to a commit")


def _project_version_from_pyproject() -> str:
    text = (_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    match = _VERSION_RE.search(text)
    if match:
        return match.group(1)
    raise AssertionError("pyproject.toml does not declare a project version")


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
        cargo_text = (
            _ROOT / "ui-tauri" / "src-tauri" / "Cargo.toml"
        ).read_text(
            encoding="utf-8"
        )
        cargo_match = _VERSION_RE.search(cargo_text)
        self.assertIsNotNone(
            cargo_match,
            "Cargo.toml does not declare a package version",
        )

        self.assertEqual(__version__, pyproject_version)
        self.assertEqual(package_json["version"], pyproject_version)
        self.assertEqual(tauri_config["version"], pyproject_version)
        self.assertEqual(cargo_match.group(1), pyproject_version)


if __name__ == "__main__":  # pragma: no cover
    unittest.main()
