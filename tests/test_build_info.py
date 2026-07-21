from __future__ import annotations

from unittest.mock import patch

import pytest

from kassiber import __version__
from kassiber.build_info import version_text
from scripts.write_build_info import resolve_built_at


def test_version_text_includes_packaged_identity():
    with patch(
        "kassiber.build_info.packaged_build_info",
        return_value={
            "version": "1.2.3",
            "channel": "prerelease",
            "commit": "0123456789abcdef",
        },
    ):
        assert version_text() == "Kassiber 1.2.3 (prerelease, commit 0123456789ab)"


def test_version_text_falls_back_to_package_version():
    with patch("kassiber.build_info.packaged_build_info", return_value={}):
        assert version_text() == f"Kassiber {__version__}"


def test_build_timestamp_honors_source_date_epoch():
    with patch.dict("os.environ", {"SOURCE_DATE_EPOCH": "0"}):
        assert resolve_built_at(None) == "1970-01-01T00:00:00Z"


def test_explicit_build_timestamp_wins_over_source_date_epoch():
    with patch.dict("os.environ", {"SOURCE_DATE_EPOCH": "0"}):
        assert resolve_built_at("2026-01-02T03:04:05Z") == "2026-01-02T03:04:05Z"


def test_invalid_source_date_epoch_is_rejected():
    with patch.dict("os.environ", {"SOURCE_DATE_EPOCH": "not-a-timestamp"}):
        with pytest.raises(ValueError, match="integer Unix timestamp"):
            resolve_built_at(None)
