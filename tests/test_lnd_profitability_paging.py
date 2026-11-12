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


@pytest.mark.parametrize("endpoint", ["forwards", "payments"])
@pytest.mark.parametrize("cursor", [None, "0"])
def test_short_page_without_advancing_cursor_refuses_incomplete_history(monkeypatch, endpoint, cursor):
    # A proxy may cap pages below LND_MAX_PAGE_SIZE: a short page is no end.
    node = PagedNode(BACKEND)
    extra = {} if cursor is None else {"last_offset_index": cursor, "last_index_offset": cursor}
    if endpoint == "forwards":
        node.post = lambda *a, **kw: {"forwarding_events": [{}], **extra}
        fetch = lnd._fetch_lnd_forwarding_history
    else:
        node.get = lambda *a, **kw: {"payments": [{}], **extra}
        fetch = lnd._fetch_lnd_payments
    with pytest.raises(AppError) as raised:
        fetch(node)
    assert raised.value.code == "lightning_history_incomplete"


def _loopback_node(mode, *, total=2500, cap=500):
    """A capping LND REST proxy on loopback whose cursor can stick."""
    import json
    import threading
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    from urllib.parse import parse_qs, urlparse
    from tests.test_lnd_adapter import CANNED_GETINFO, CANNED_INVOICES

    requests = []

    def page(template, offset, limit, update):
        rows = []
        for index in range(offset, min(offset + min(limit, cap), total)):
            row = deepcopy(template)
            update(row, index)
            rows.append(row)
        return rows

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def _send(self, body):
            data = json.dumps(body).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            url = urlparse(self.path)
            query = {key: values[0] for key, values in parse_qs(url.query).items()}
            requests.append(("GET", url.path))
            offset = int(query.get("index_offset", 0))
            if url.path == "/v1/payments":
                rows = page(CANNED_PAYMENTS["payments"][0], offset, int(query["max_payments"]),
                            lambda row, i: row.update(payment_index=str(i + 1), payment_hash=f"h{i}"))
                cursor = offset if mode == "stuck_payments" else offset + len(rows)
                return self._send({"payments": rows, "last_index_offset": str(cursor)})
            if url.path == "/v1/invoices":
                rows = page(CANNED_INVOICES["invoices"][0], offset, int(query["num_max_invoices"]),
                            lambda row, i: row.update(add_index=str(i + 1), r_hash=f"r{i}"))
                return self._send({"invoices": rows, "last_index_offset": str(offset + len(rows))})
            if url.path == "/v1/getinfo":
                return self._send(dict(CANNED_GETINFO))
            return self._send(dict(_FakeLndRestClient(BACKEND).responses.get(("GET", url.path), {})))

        def do_POST(self):
            url = urlparse(self.path)
            payload = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
            requests.append(("POST", url.path))
            if url.path != "/v1/switch":
                return self._send({})
            offset = int(payload.get("index_offset", 0))
            rows = page(CANNED_FORWARDS["forwarding_events"][0], offset, int(payload["num_max_events"]),
                        lambda row, i: row.update(timestamp_ns=str(int(row["timestamp_ns"]) + i)))
            cursor = offset if mode == "stuck_forwards" else offset + len(rows)
            return self._send({"forwarding_events": rows, "last_offset_index": str(cursor)})

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    backend = {"name": "node", "kind": "lnd", "token": "00aa",
               "url": f"http://127.0.0.1:{server.server_address[1]}"}
    return server, thread, backend, requests


@pytest.mark.parametrize("mode", ["stuck_forwards", "stuck_payments"])
def test_loopback_proxy_with_stuck_cursor_on_capped_pages_refuses_truncation(mode):
    server, thread, backend, requests = _loopback_node(mode)
    try:
        with pytest.raises(AppError) as raised:
            lnd._fetch_lnd_forwarding_history(lnd.LndRestClient(backend)) if mode == "stuck_forwards" \
                else lnd._fetch_lnd_payments(lnd.LndRestClient(backend))
        assert raised.value.code == "lightning_history_incomplete"
        assert len(requests) == 1
    finally:
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()


def test_loopback_proxy_with_advancing_cursor_reads_every_capped_page():
    server, thread, backend, requests = _loopback_node("advancing")
    try:
        client = lnd.LndRestClient(backend)
        assert len(lnd._fetch_lnd_forwarding_history(client)) == 2500
        assert len(lnd._fetch_lnd_payments(client)) == 2500
        assert requests.count(("POST", "/v1/switch")) == requests.count(("GET", "/v1/payments")) == 6
    finally:
        server.shutdown()
        thread.join(timeout=5)
        server.server_close()
