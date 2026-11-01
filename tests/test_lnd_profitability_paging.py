"""Profitability cannot silently use only LND's first page."""

from copy import deepcopy
from decimal import Decimal

import pytest

from kassiber.core.lightning import lnd
from kassiber.errors import AppError
from tests.test_lnd_adapter import (
    BACKEND, CANNED_FORWARDS, CANNED_PAYMENTS, _FakeLndRestClient,
)


class PagedNode(_FakeLndRestClient):
    def get(self, path, *, params=None):
        if path != "/v1/payments":
            return super().get(path, params=params)
        self.requests.append(("GET", path, dict(params or {})))
        offset = int((params or {}).get("index_offset", 0))
        rows = []
        for index in range(offset, min(offset + 2, 3)):
            row = deepcopy(CANNED_PAYMENTS["payments"][0])
            row.update(payment_index=str(index + 1), payment_hash=f"hash-{index}")
            if index < 2:
                row.update(creation_date="1", creation_time_ns="1000000000")
            rows.append(row)
        return {"payments": rows, "last_index_offset": str(offset + len(rows))}

    def post(self, path, payload=None):
        if path != "/v1/switch":
            return super().post(path, payload)
        self.requests.append(("POST", path, dict(payload or {})))
        offset = int((payload or {}).get("index_offset", 0))
        rows = []
        for index in range(offset, min(offset + 2, 3)):
            row = deepcopy(CANNED_FORWARDS["forwarding_events"][0])
            row["timestamp"] = str(int(row["timestamp"]) + index)
            row["timestamp_ns"] = str(int(row["timestamp_ns"]) + index * 1_000_000_000)
            rows.append(row)
        return {"forwarding_events": rows, "last_offset_index": str(offset + len(rows))}


def test_snapshot_reads_all_pages_and_finds_recent_payments_after_oldest_page(monkeypatch):
    node = PagedNode(BACKEND)
    constructed = []
    monkeypatch.setattr(lnd, "LND_MAX_PAGE_SIZE", 2)
    monkeypatch.setattr(lnd, "LndRestClient", lambda backend: constructed.append(backend) or node)
    adapter = lnd.LndAdapter()
    assert not constructed and not node.requests
    snapshot = adapter.fetch_node_snapshot({}, BACKEND, window_days=30)
    assert snapshot.routing.routing_revenue_sat == 30
    assert snapshot.routing.payment_cost_sat == 2
    assert snapshot.completed_payment_count == 1
    for method, path in (("POST", "/v1/switch"), ("GET", "/v1/payments")):
        calls = [data for verb, endpoint, data in node.requests if (verb, endpoint) == (method, path)]
        assert [int(data["index_offset"]) for data in calls][:2] == [0, 2]
    assert "PREIMAGE_THAT_SHOULD_NEVER_LEAK" not in repr(snapshot)
    assert "SOURCE_PUBKEY_SHOULD_BE_DROPPED" not in repr(snapshot)


@pytest.mark.parametrize("endpoint", ["forwards", "payments"])
def test_full_page_without_advancing_cursor_refuses_incomplete_history(monkeypatch, endpoint):
    node = PagedNode(BACKEND)
    monkeypatch.setattr(lnd, "LND_MAX_PAGE_SIZE", 2)
    if endpoint == "forwards":
        node.post = lambda *a, **kw: {"forwarding_events": [{}, {}]}
        fetch = lnd._fetch_lnd_forwarding_history
    else:
        node.get = lambda *a, **kw: {"payments": [{}, {}]}
        fetch = lnd._fetch_lnd_payments
    with pytest.raises(AppError) as raised:
        fetch(node)
    assert raised.value.code == "lightning_history_incomplete"


def test_proxy_capped_forward_pages_read_all_2500_events():
    class CappedNode(PagedNode):
        def post(self, path, payload=None):
            assert path == "/v1/switch"
            self.requests.append(("POST", path, dict(payload)))
            offset = int(payload["index_offset"])
            rows = []
            for index in range(offset, min(offset + 500, 2500)):
                row = deepcopy(CANNED_FORWARDS["forwarding_events"][0])
                row.update(timestamp_ns=str(1_710_000_000_000_000_000 + index), fee_msat="1000")
                rows.append(row)
            return {"forwarding_events": rows, "last_offset_index": str(offset + len(rows))}

    node = CappedNode(BACKEND)
    rows = lnd._fetch_lnd_forwarding_history(node)
    assert len(rows) == 2500
    assert sum(row["amount"] for row in lnd._lnd_routing_income_imports(rows)) == Decimal("0.000025")
    assert [int(payload["index_offset"]) for _, _, payload in node.requests] == [0, 500, 1000, 1500, 2000, 2500]
