"""Automatic diagnostics reports each get their own file."""

from __future__ import annotations

import json
from pathlib import Path

from kassiber.diagnostics import save_public_diagnostics_report


def _save(data_root: Path, generated_at: str, marker: str):
    return save_public_diagnostics_report(
        {"generated_at": generated_at, "marker": marker},
        target="auto",
        data_root=str(data_root),
    )


def test_reports_are_named_by_their_timestamp(tmp_path):
    saved = _save(tmp_path / "data", "2026-10-29T01:02:03Z", "first")

    assert saved.path.name == "kassiber-diagnostics-2026-10-29T01-02-03Z.json"
    assert saved.relative_path == f"exports/diagnostics/{saved.path.name}"
    assert saved.target == "auto"


def test_a_later_report_never_overwrites_an_earlier_one(tmp_path):
    data_root = tmp_path / "data"
    first = _save(data_root, "2026-10-29T01:02:03Z", "first")
    second = _save(data_root, "2026-10-30T04:05:06Z", "second")
    same_second = _save(data_root, "2026-10-30T04:05:06Z", "third")

    assert len({first.path, second.path, same_second.path}) == 3
    assert same_second.path.name == "kassiber-diagnostics-2026-10-30T04-05-06Z-2.json"
    assert [json.loads(saved.path.read_text())["marker"] for saved in (first, second, same_second)] == [
        "first",
        "second",
        "third",
    ]


def test_a_custom_target_is_written_where_the_user_asked(tmp_path):
    target = tmp_path / "custom.json"
    target.write_text("old\n")

    saved = save_public_diagnostics_report(
        {"generated_at": "2026-10-29T01:02:03Z"}, target=str(target), data_root=str(tmp_path / "data")
    )

    assert saved.path == target
    assert saved.relative_path is None
    assert json.loads(target.read_text())["generated_at"] == "2026-10-29T01:02:03Z"
