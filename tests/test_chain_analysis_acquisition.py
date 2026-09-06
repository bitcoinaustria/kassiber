import io
import json
import unittest
from unittest.mock import patch
from urllib import error as urlerror, request as urlrequest, response as urlresponse

from kassiber.core import chain_analysis_acquisition as acquisition
from kassiber.core.chain_analysis import build_index
from kassiber.errors import AppError
from kassiber import proxy
from tests import test_privacy_hygiene as hygiene_fixture

NOW = hygiene_fixture.NOW


def tid(number):
    return f"{number:064x}"


def raw(number, parents=(), *, outputs=1):
    return {"txid": tid(number), "vin": [{"txid": tid(parent), "vout": 0, "sequence": 0xFFFFFFFF} for parent in parents] or [{"is_coinbase": True}], "vout": [{"value": 1000, "scriptpubkey": "0014" + "11" * 20} for _ in range(outputs)], "vsize": 100}


class FakeHTTP:
    def __init__(self, routes):
        self.routes, self.calls = routes, []

    def __call__(self, request, url, timeout, **kwargs):
        self.calls.append((url, timeout, kwargs))
        value = self.routes[url.rsplit("/api", 1)[-1]]
        if isinstance(value, Exception):
            raise value
        if callable(value):
            value = value(request)
        return io.BytesIO(value if isinstance(value, bytes) else json.dumps(value).encode())


class ChainAnalysisAcquisitionTests(unittest.TestCase):
    def setUp(self):
        self.book = hygiene_fixture.PrivacyHygieneTests(); self.book.setUp(); self.conn = self.book.conn
        self.conn.execute("INSERT INTO backends(name,kind,chain,network,url,auth_header,created_at,updated_at) VALUES('node','esplora','bitcoin','main','https://node.example/api','Bearer synthetic-secret',?,?)", (NOW, NOW))

    def tearDown(self):
        self.book.tearDown()

    def plan(self, **kwargs):
        return acquisition.plan_acquisition(self.conn, "pf", {"backend": "node", "subject": tid(2), "chain": "bitcoin", "network": "main", "direction": "backward", **kwargs})

    def http(self, **routes):
        return FakeHTTP({"/block-height/0": acquisition.GENESIS[("bitcoin", "main")].encode(), **routes})

    def test_plan_is_local_and_tampering_is_rejected_before_egress(self):
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("unexpected egress")):
            plan = self.plan()
            self.assertNotIn("synthetic-secret", json.dumps(plan))
            self.assertNotIn("node.example", json.dumps(plan))
            plan["effects"]["max_transactions"] = 999
            with self.assertRaises(AppError) as caught:
                acquisition.apply_acquisition(self.conn, "pf", {"plan": plan})
        self.assertEqual(caught.exception.code, "chain_analysis_stale")

    def test_standard_network_genesis_cannot_be_overridden(self):
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("unexpected egress")):
            with self.assertRaises(AppError) as caught:
                self.plan(genesis_hash=acquisition.GENESIS[("bitcoin", "regtest")])
        self.assertEqual(caught.exception.code, "validation")
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM chain_analysis_observations").fetchone()[0], 0)

    def test_same_timestamp_backend_redirect_invalidates_plan(self):
        plan = self.plan()
        self.conn.execute("UPDATE backends SET url='https://different.example/api'")
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("unexpected egress")):
            with self.assertRaises(AppError) as caught:
                acquisition.apply_acquisition(self.conn, "pf", {"plan": plan})
        self.assertEqual(caught.exception.code, "chain_analysis_stale")

    def test_backward_acquisition_sanitizes_secrets_and_stays_reference_only(self):
        child = {**raw(2, [1]), "descriptor": "xpub-secret", "preimage": "private-preimage", "raw_hex": "private-raw"}
        child["vin"][0]["witness"] = ["private-preimage"]
        http = self.http(**{f"/tx/{tid(2)}": child, f"/tx/{tid(1)}": raw(1)})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertEqual(result["acquired_count"], 2)
        self.assertEqual(result["request_count"], 3)
        stored = self.conn.execute("SELECT payload_json FROM chain_analysis_observations").fetchall()
        self.assertFalse(any("private" in row[0] or "xpub" in row[0] or "witness" in row[0] for row in stored))
        self.assertTrue(all(timeout <= 8 for _, timeout, _ in http.calls))
        self.assertTrue(all(options["follow_redirects"] is False for _, _, options in http.calls))
        index = build_index(self.conn, "pf")
        self.assertTrue(all(not node["wallet_ids"] for node in index.nodes.values()))
        self.assertEqual(index.coverage["complete_transaction_count"], 2)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM transactions").fetchone()[0], 0)
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_observation_provenance").fetchone()[0], 0)

    def test_genesis_mismatch_and_profile_revalidation_save_nothing(self):
        http = self.http(**{"/block-height/0": b"wrong"})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            with self.assertRaises(AppError) as caught:
                acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertEqual(caught.exception.code, "backend_network_mismatch")
        def changed(_request):
            self.conn.execute("UPDATE backends SET url='https://changed.example/api'")
            return raw(2)
        http = self.http(**{f"/tx/{tid(2)}": changed})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            with self.assertRaises(AppError) as caught:
                acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertEqual(caught.exception.code, "chain_analysis_stale")
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_analysis_observations").fetchone()[0], 0)

    def test_depth_budget_and_missing_history_produce_frontier(self):
        http = self.http(**{f"/tx/{tid(2)}": raw(2, [1]), f"/tx/{tid(1)}": OSError("pruned")})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertFalse(result["complete"])
        self.assertEqual(result["frontier"], [{"txid": tid(1), "reason": "transaction_unavailable"}])
        http = self.http(**{f"/tx/{tid(2)}": raw(2, [1])})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(max_transactions=1)})
        self.assertEqual(result["frontier"], [{"txid": tid(1), "reason": "transaction_limit"}])

    def test_reorg_status_changes_snapshot_and_pruned_refresh_keeps_reference(self):
        first_payload = {**raw(2), "status": {"confirmed": True, "block_height": 10, "block_hash": "a" * 64}}
        http = self.http(**{f"/tx/{tid(2)}": first_payload})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            first = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        second_payload = {**first_payload, "status": {"confirmed": True, "block_height": 10, "block_hash": "b" * 64}}
        http = self.http(**{f"/tx/{tid(2)}": second_payload})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            second = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertNotEqual(first["snapshot_id"], second["snapshot_id"])
        before = dict(self.conn.execute("SELECT * FROM chain_analysis_observations").fetchone())
        http = self.http(**{f"/tx/{tid(2)}": OSError("pruned")})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            missing = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
        self.assertFalse(missing["complete"])
        self.assertEqual(missing["acquired_count"], 0)
        self.assertEqual(dict(self.conn.execute("SELECT * FROM chain_analysis_observations").fetchone()), before)

    def test_malformed_graph_or_oversized_response_is_never_saved(self):
        invalid = [raw(3), {"txid": tid(2), "vin": [{}], "vout": [{}]}, {**raw(2), "vout": [{"value": -1}]}, b"x" * (1024 + 1)]
        for payload in invalid:
            with self.subTest(payload_type=type(payload).__name__):
                http = self.http(**{f"/tx/{tid(2)}": payload})
                with patch.object(acquisition, "MAX_RESPONSE_BYTES", 1024), patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
                    result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan()})
                self.assertEqual(result["acquired_count"], 0)
                self.assertFalse(result["complete"])
        self.assertEqual(self.conn.execute("SELECT count(*) FROM chain_analysis_observations").fetchone()[0], 0)

    def test_request_deadline_and_electrum_line_bounds(self):
        with patch.object(acquisition.time, "monotonic", return_value=0):
            budget = acquisition._Budget(2, seconds=1)
            budget.request(); budget.request()
            with self.assertRaises(AppError): budget.request()
        with patch.object(acquisition.time, "monotonic", return_value=2):
            with self.assertRaises(AppError): budget.request()
        class Socket:
            def sendall(self, _payload): pass
            def settimeout(self, _value): pass
            def recv(self, amount): return b"x" * amount
        budget = acquisition._Budget(2)
        client = acquisition._BoundedElectrumClient({"name": "fake"}, budget)
        client.socket = Socket()
        with patch.object(acquisition, "MAX_RESPONSE_BYTES", 1024), patch.object(acquisition.transport, "get_egress_ledger"):
            with self.assertRaises(AppError) as caught:
                client._call_locked("server.version")
        self.assertEqual(caught.exception.code, "invalid_observation")

    def test_total_byte_budget_stops_later_requests(self):
        budget = acquisition._Budget(20)
        with patch.object(acquisition, "MAX_TOTAL_BYTES", 100):
            budget.received(100)
            with self.assertRaises(AppError) as caught:
                budget.request()
        self.assertEqual(caught.exception.code, "acquisition_budget")
        self.assertEqual(budget.count, 0)

    def test_redirect_never_sends_authorization_to_second_host(self):
        calls = []
        class FakeHTTPHandler(urlrequest.HTTPHandler):
            def http_open(self, request):
                calls.append((request.full_url, request.get_header("Authorization")))
                headers = {"Location": "http://second.invalid/stolen"} if len(calls) == 1 else {}
                response = urlresponse.addinfourl(io.BytesIO(b""), headers, request.full_url, 302 if len(calls) == 1 else 200)
                response.msg = "Found"
                return response
        original = urlrequest.build_opener
        with patch.object(proxy.urlrequest, "build_opener", side_effect=lambda *handlers: original(FakeHTTPHandler(), *handlers)):
            with self.assertRaises(urlerror.HTTPError) as caught:
                proxy.urlopen_with_proxy(urlrequest.Request("http://first.invalid/start", headers={"Authorization": "Bearer synthetic-secret"}), follow_redirects=False)
        self.assertEqual(caught.exception.code, 302)
        self.assertEqual(calls, [("http://first.invalid/start", "Bearer synthetic-secret")])

    def test_socks_error_body_is_bounded_before_context_entry(self):
        reads = []
        class Response:
            status, reason, headers = 500, "error", {}
            def read(self, amount):
                reads.append(amount)
                return b"x" * amount
            def close(self): pass
        class Connection:
            def __init__(self, *_args, **_kwargs): pass
            def request(self, *_args, **_kwargs): pass
            def getresponse(self): return Response()
            def close(self): pass
        with patch.object(proxy.http.client, "HTTPConnection", Connection):
            with self.assertRaises(AppError) as caught:
                with proxy.SocksUrlResponse("http://node.invalid", "socks5h://proxy.invalid:9050", 1, {}, max_error_bytes=64):
                    self.fail("oversized error must fail during context entry")
        self.assertEqual(caught.exception.code, "invalid_observation")
        self.assertEqual(reads, [65])

    def test_acquisition_socks_error_uses_deadline_and_byte_accounting(self):
        elapsed, eager_reads, chunks, closed = [0.0], [], [], []
        class Response:
            status, reason, headers = 500, "error", {}
            def read(self, amount):
                eager_reads.append(amount)
                elapsed[0] += 120
                return b"x" * amount
            def read1(self, amount):
                chunks.append(amount)
                elapsed[0] += 8
                return b"x" * amount
            def close(self): closed.append("response")
        class Connection:
            def __init__(self, *_args, **_kwargs): pass
            def request(self, *_args, **_kwargs): pass
            def getresponse(self): return Response()
            def close(self): closed.append("connection")
        backend = {"name": "node", "kind": "esplora", "url": "http://node.invalid", "tor_proxy": "socks5h://proxy.invalid:9050"}
        with patch.object(acquisition.time, "monotonic", side_effect=lambda: elapsed[0]), patch.object(proxy.http.client, "HTTPConnection", Connection):
            budget = acquisition._Budget(10, seconds=45)
            with self.assertRaises(AppError) as caught:
                acquisition._Reader(backend, budget).http("/missing")
        self.assertEqual(caught.exception.code, "acquisition_budget")
        self.assertEqual(eager_reads, [])
        self.assertEqual(elapsed[0], 48)
        self.assertEqual(chunks, [65536] * 6)
        self.assertEqual(budget.bytes_read, 6 * 65536)
        self.assertIn("response", closed)
        self.assertIn("connection", closed)

    def test_direct_http_errors_share_total_byte_budget_and_close_streams(self):
        streams = [io.BytesIO(b"private-error-body" * 3), io.BytesIO(b"private-error-body" * 3)]
        body_size = len(streams[0].getvalue())
        errors = [urlerror.HTTPError("http://node.invalid/missing", 404, "missing", {}, stream) for stream in streams]
        budget = acquisition._Budget(10)
        reader = acquisition._Reader({"url": "http://node.invalid"}, budget)
        with patch.object(acquisition, "MAX_TOTAL_BYTES", body_size + 1), patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=errors):
            with self.assertRaises(AppError) as first:
                reader.http("/missing")
            with self.assertRaises(AppError) as second:
                reader.http("/missing")
        self.assertEqual(first.exception.code, "history_unavailable")
        self.assertNotIn("private-error-body", str(first.exception))
        self.assertEqual(second.exception.code, "acquisition_budget")
        self.assertEqual(budget.bytes_read, body_size * 2)
        self.assertTrue(all(stream.closed for stream in streams))

    def test_core_http500_envelope_is_bounded_counted_and_closed(self):
        body = json.dumps({"id": "chain-analysis-getrawtransaction", "result": None, "error": {"code": -5, "message": "private backend error"}}).encode()
        stream = io.BytesIO(body)
        error = urlerror.HTTPError("http://node.invalid", 500, "error", {}, stream)
        backend = {"name": "node", "url": "http://node.invalid", "username": "test", "password": "synthetic"}
        budget = acquisition._Budget(10)
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=error):
            with self.assertRaises(AppError) as caught:
                acquisition._Reader(backend, budget).rpc("getrawtransaction", [tid(2), True])
        self.assertEqual(caught.exception.code, "history_unavailable")
        self.assertNotIn("private backend error", str(caught.exception))
        self.assertEqual(budget.bytes_read, len(body))
        self.assertTrue(stream.closed)

    def test_false_spender_claim_is_an_explicit_frontier(self):
        http = self.http(**{f"/tx/{tid(2)}": raw(2), f"/tx/{tid(2)}/outspends": [{"spent": True, "txid": tid(3)}], f"/tx/{tid(3)}": raw(3), f"/tx/{tid(3)}/outspends": [{"spent": False}]})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(direction="forward")})
        self.assertFalse(result["complete"])
        self.assertIn({"txid": tid(2), "reason": "spender_claim_not_corroborated"}, result["frontier"])
        status = json.loads(self.conn.execute("SELECT status_json FROM chain_analysis_observations WHERE txid=?", (tid(2),)).fetchone()[0])
        self.assertFalse(status["spenders_checked"])

    def test_electrum_actual_transaction_bytes_and_forward_capability(self):
        from embit.transaction import Transaction, TransactionInput, TransactionOutput
        from embit.script import Script
        tx = Transaction(vin=[TransactionInput(b"\0" * 32, 0xFFFFFFFF, script_sig=Script(b"\x01\x01"))], vout=[TransactionOutput(1000, Script(b"\x51"))])
        identity = tx.txid().hex()
        class Client:
            socket = None
            def __init__(self, *_args): pass
            def __enter__(self): return self
            def __exit__(self, *_args): pass
            def call(self, method, _params=None):
                return {"genesis_hash": acquisition.GENESIS[("bitcoin", "main")]} if method == "server.features" else tx.serialize().hex()
        self.conn.execute("UPDATE backends SET kind='electrum',url='ssl://electrum.example:50002'")
        with patch.object(acquisition, "_BoundedElectrumClient", Client):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(subject=identity, direction="both")})
        self.assertEqual(result["acquired_count"], 1)
        self.assertEqual(result["request_count"], 3)  # version, features, transaction
        self.assertIn({"txid": identity, "reason": "historical_spender_index_unavailable"}, result["frontier"])
        self.assertEqual(build_index(self.conn, "pf").coverage["complete_transaction_count"], 1)

    def test_liquid_electrum_pegin_keeps_boundary_without_false_ancestry(self):
        from embit.liquid.transaction import LTransaction, LTransactionInput, LTransactionOutput
        from embit.script import Script
        tx = LTransaction(vin=[LTransactionInput(bytes.fromhex(tid(1)), 0, is_pegin=True)], vout=[LTransactionOutput(b"\x11" * 32, 1000, Script(b"\x51"))])
        class Client:
            socket = None
            def __init__(self, *_args): pass
            def __enter__(self): return self
            def __exit__(self, *_args): pass
            def call(self, method, _params=None):
                return {"genesis_hash": acquisition.GENESIS[("liquid", "main")]} if method == "server.features" else tx.serialize().hex()
        self.conn.execute("UPDATE backends SET kind='electrum',chain='liquid',url='ssl://elements.example:50002'")
        with patch.object(acquisition, "_BoundedElectrumClient", Client):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(subject=tx.txid().hex(), chain="liquid")})
        self.assertEqual(result["acquired_count"], 1)
        index = build_index(self.conn, "pf")
        self.assertFalse(any(edge["kind"] == "spends" for edge in index.edges.values()))
        self.assertTrue(any(item["code"] == "native_pegin_boundary" for item in index.findings))

    def test_core31_spender_index_capability_and_protocol(self):
        self.conn.execute("UPDATE backends SET kind='bitcoinrpc',config_json=?", (json.dumps({"username": "test", "password": "synthetic"}),))
        calls = []
        def response(request):
            payload = json.loads(request.data)
            calls.append(payload)
            method = payload["method"]
            value = {"getblockhash": acquisition.GENESIS[("bitcoin", "main")], "getindexinfo": {"txospenderindex": {"synced": True}}, "getrawtransaction": {"txid": tid(2), "vin": [{"coinbase": "0101"}], "vout": [{"n": 0, "value": 0.00001, "scriptPubKey": {"hex": "0014" + "11" * 20}}]}, "gettxspendingprevout": [{"txid": tid(2), "vout": 0}]}[method]
            return {"id": payload["id"], "result": value, "error": None}
        http = FakeHTTP({"": response})
        with patch.object(acquisition.transport, "urlopen_with_proxy", side_effect=http):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(direction="forward")})
        self.assertTrue(result["complete"])
        spender = next(item for item in calls if item["method"] == "gettxspendingprevout")
        self.assertEqual(spender["params"], [[{"txid": tid(2), "vout": 0}], {"mempool_only": False}])

    def test_core_missing_spender_index_does_not_claim_forward_completeness(self):
        self.conn.execute("UPDATE backends SET kind='bitcoinrpc'")
        methods = []
        def rpc(_reader, method, _params=None):
            methods.append(method)
            return {"getblockhash": acquisition.GENESIS[("bitcoin", "main")], "getindexinfo": {}, "getrawtransaction": {"txid": tid(2), "vin": [{"coinbase": "0101"}], "vout": [{"value": 0.00001, "scriptPubKey": {"hex": "51"}}]}}[method]
        with patch.object(acquisition._Reader, "rpc", rpc):
            result = acquisition.apply_acquisition(self.conn, "pf", {"plan": self.plan(direction="forward")})
        self.assertFalse(result["complete"])
        self.assertEqual(result["frontier"], [{"txid": tid(2), "reason": "historical_spender_index_unavailable"}])
        self.assertNotIn("gettxspendingprevout", methods)


if __name__ == "__main__":
    unittest.main()
