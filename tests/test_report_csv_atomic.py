"""Report CSV artifacts are replaced only once every row is written."""

from __future__ import annotations

import os
import stat

import pytest

from kassiber.core.reports import _write_csv_rows


def _failing_rows():
    yield ["partial"]
    raise OSError("injected write failure")


def test_an_interrupted_write_keeps_the_previous_report(tmp_path):
    path = tmp_path / "report.csv"
    path.write_bytes(b"previous complete report\r\n")

    with pytest.raises(OSError, match="injected write failure"):
        _write_csv_rows(path, _failing_rows())

    assert path.read_bytes() == b"previous complete report\r\n"
    assert sorted(item.name for item in tmp_path.iterdir()) == ["report.csv"]


def test_a_new_report_gets_the_umask_default_mode(tmp_path):
    path = tmp_path / "report.csv"
    previous = os.umask(0o027)
    try:
        _write_csv_rows(path, [["a", "b"], ["1", "2"]])
    finally:
        os.umask(previous)

    assert path.read_bytes() == b"a,b\r\n1,2\r\n"
    assert stat.S_IMODE(path.stat().st_mode) == 0o640


def test_an_existing_report_keeps_its_mode(tmp_path):
    path = tmp_path / "report.csv"
    path.write_bytes(b"old\r\n")
    path.chmod(0o600)

    _write_csv_rows(path, [["new"]])

    assert path.read_bytes() == b"new\r\n"
    assert stat.S_IMODE(path.stat().st_mode) == 0o600


def test_a_final_symlink_is_written_through(tmp_path):
    target = tmp_path / "real.csv"
    target.write_bytes(b"old\r\n")
    link = tmp_path / "link.csv"
    link.symlink_to(target)

    _write_csv_rows(link, [["new"]])

    assert link.is_symlink()
    assert target.read_bytes() == b"new\r\n"
