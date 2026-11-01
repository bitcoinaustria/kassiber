"""CLN report acquisition and time-window regressions from the Rust port."""

from dataclasses import replace
from unittest.mock import Mock

import pytest

from kassiber.core.lightning import build_profitability_report
from kassiber.core.lightning import cln
from kassiber.errors import AppError


def test_cln_product_no_egress_refuses_before_spawning(monkeypatch, tmp_path):
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    monkeypatch.setenv("KASSIBER_OFFLINE_PREFERENCE_FILE", str(tmp_path / "offline.json"))
    run = Mock(side_effect=AssertionError("must not start the node client"))
    monkeypatch.setattr(cln.subprocess, "run", run)
    with pytest.raises(AppError) as error:
        cln.call_core_lightning({}, "getinfo")
    assert error.value.code == "network_egress_disabled"
    run.assert_not_called()
