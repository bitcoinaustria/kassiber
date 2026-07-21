from __future__ import annotations

from unittest.mock import patch

from kassiber import __version__
from kassiber.build_info import version_text


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
