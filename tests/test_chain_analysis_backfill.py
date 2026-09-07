"""Real durable quota, source-fencing, occurrence and reorg boundaries."""
import hashlib
import io
import json
from unittest.mock import patch

import pytest
from embit import compact
from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from kassiber.core import chain_analysis_backfill as backfill
from kassiber.errors import AppError
from tests import test_privacy_hygiene as hygiene
NOW = hygiene.NOW


@pytest.fixture
def book():
    fixture = hygiene.PrivacyHygieneTests()
    fixture.setUp()
    conn = fixture.conn
    conn.execute("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at) VALUES('core','bitcoinrpc','bitcoin','regtest','http://127.0.0.1:18443',?,?)", (NOW, NOW))
    conn.execute("UPDATE backends SET config_json=?", (json.dumps({"username": "synthetic", "password": "synthetic"}),))
    conn.commit()
    with patch.object(backfill, "_domain", return_value={"domain_id": "lab-a", "environment_id": "lab", "revision": 1}), patch.object(backfill, "_require_protected_book"), patch.dict(backfill.acquisition.GENESIS, {("bitcoin", "regtest"): block()[0]}):
        # Domain and keyed-book boundaries get end-to-end coverage after their prerequisite PRs.
        yield conn
    fixture.tearDown()


def reviewed(conn, **kwargs):
    return backfill.plan(conn, "pf", {"backend": "core", "network": "regtest", "mode": "blocks", "start_height": 0,
                                         "interval_seconds": 30, "max_requests": 100, "max_bytes": 100_000_000, **kwargs})


def grant(conn, **kwargs):
    result = backfill.authorize(conn, "pf", {"plan": reviewed(conn, **kwargs)})
    conn.commit()
    return result["id"]


def block(parent="0" * 64, transactions=None, nonce=0):
    tx = Transaction(vin=[TransactionInput(bytes(32), 0xFFFFFFFF, script_sig=Script(b"\x01\x01"))], vout=[TransactionOutput(1000, Script(bytes.fromhex("0014" + "11" * 20)))])
    transactions = transactions or [tx]
    leaves = [bytes.fromhex(tx.txid().hex())[::-1] for tx in transactions]
    while len(leaves) > 1:
        if len(leaves) % 2:
            leaves.append(leaves[-1])
        leaves = [hashlib.sha256(hashlib.sha256(leaves[i] + leaves[i + 1]).digest()).digest() for i in range(0, len(leaves), 2)]
    header = (1).to_bytes(4, "little") + bytes.fromhex(parent)[::-1] + leaves[0] + bytes(8) + nonce.to_bytes(4, "little")
    raw = header + compact.to_bytes(len(transactions)) + b"".join(tx.serialize() for tx in transactions)
    return hashlib.sha256(hashlib.sha256(header).digest()).digest()[::-1].hex(), raw.hex(), transactions


class Core:
    def __init__(self, blocks, hook=None):
        self.blocks, self.hook, self.calls = blocks, hook, []

    def __call__(self, request, url, timeout, **options):
        assert options["follow_redirects"] is False
        data = json.loads(request.data)
        method, params = data["method"], data["params"]
        self.calls.append((method, params))
        if self.hook:
            self.hook(method, params)
        if method == "getblockhash":
            value = backfill.acquisition.GENESIS[("bitcoin", "regtest")] if params[0] == 0 and len(self.calls) == 1 else self.blocks[params[0]][0]
        elif method == "getblockcount":
            value = len(self.blocks) - 1
        elif method == "getblock":
            value = next(item[1] for item in self.blocks if item[0] == params[0])
        else:
            raise AssertionError(method)
        return io.BytesIO(json.dumps({"id": data["id"], "result": value}).encode())


def run(conn, ident, core):
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=core):
        return backfill.run(conn, "pf", ident)


def test_plan_and_authorization_are_offline_and_bound(book):
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("egress")):
        plan = reviewed(book)
        assert "127.0.0.1" not in json.dumps(plan)
        plan["effects"]["max_requests"] += 1
        with pytest.raises(AppError, match="review"):
            backfill.authorize(book, "pf", {"plan": plan})
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_acquisition_grants").fetchone()[0] == 0


def test_removed_source_pauses_without_starving_other_work(book):
    ident = grant(book)
    book.execute("DELETE FROM backends WHERE name='core'")
    book.commit()
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("egress")):
        backfill.run_due(book, "pf")
    assert backfill.get(book, "pf", ident)["status"] == "paused"
    assert backfill.get(book, "pf", ident)["last_code"] == "acquisition_source_changed"


def test_explicit_backend_instance_cannot_be_reassigned_by_plan(book):
    book.execute("UPDATE backends SET config_json=? WHERE name='core'", (json.dumps({"chain_instance_id": "another-lab"}),))
    with pytest.raises(AppError) as caught:
        reviewed(book, chain_instance_id="this-lab")
    assert caught.value.code == "book_network_mismatch"


def test_tip_refresh_does_not_rewrite_all_transaction_assertions(book):
    first = block()
    ident = grant(book, end_height=0)
    run(book, ident, Core([first]))
    before = [dict(row) for row in book.execute("SELECT * FROM chain_analysis_reference_assertions")]
    second = block(first[0], nonce=7)
    result = run(book, ident, Core([first, second]))
    assert result["verified_tip_height"] == 1
    assert [dict(row) for row in book.execute("SELECT * FROM chain_analysis_reference_assertions")] == before


def test_blocks_resume_preserve_duplicate_txid_occurrences_and_sanitize(book):
    first = block()
    second = block(first[0], first[2], nonce=1)
    ident = grant(book, blocks_per_run=1)
    assert run(book, ident, Core([first, second]))["cursor_height"] == 0
    assert run(book, ident, Core([first, second]))["cursor_height"] == 1
    rows = book.execute("SELECT * FROM chain_analysis_reference_assertions").fetchall()
    assert len(rows) == 2
    assert rows[0]["txid"] == rows[1]["txid"]
    assert rows[0]["occurrence_id"] != rows[1]["occurrence_id"]
    for row in rows:
        raw = json.loads(row["payload_json"])
        assert raw["vin"][0]["is_coinbase"] is True
        assert not {"raw_hex", "witness", "scriptsig", "preimage"}.intersection(raw["vin"][0])
    assert book.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == 0


def test_reorg_retracts_membership_without_erasing_history(book):
    first = block()
    old, replacement = block(first[0], nonce=1), block(first[0], nonce=2)
    ident = grant(book)
    run(book, ident, Core([first, old]))
    result = run(book, ident, Core([first, replacement]))
    assert result["last_code"] == "reorg_reconciling"
    assert result["cursor_height"] == 0
    assert book.execute("SELECT active FROM chain_analysis_reference_blocks WHERE block_hash=?", (old[0],)).fetchone()[0] == 0
    run(book, ident, Core([first, replacement]))
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_blocks").fetchone()[0] == 3
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions WHERE active=1").fetchone()[0] == 2


def test_source_change_mid_fetch_prevents_next_request_and_publication(book):
    ident = grant(book)
    def change(method, params):
        if method == "getblock":
            book.execute("UPDATE backends SET url='http://127.0.0.1:29999'")
            book.commit()
    core = Core([block()], change)
    result = run(book, ident, core)
    assert result["status"] == "paused"
    assert result["last_code"] == "acquisition_source_changed"
    assert core.calls[-1][0] == "getblock"
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions").fetchone()[0] == 0


def test_revocation_during_response_cannot_be_undone(book):
    ident = grant(book)
    def change(method, params):
        if method == "getblock":
            backfill.revoke(book, "pf", {"id": ident, "expected_revision": 1})
            book.commit()
    result = run(book, ident, Core([block()], change))
    assert result["status"] == "revoked"
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions").fetchone()[0] == 0


def test_lifetime_request_quota_stops_before_dispatch(book):
    ident = grant(book, max_requests=4)
    core = Core([block()])
    result = run(book, ident, core)
    assert result["last_code"] == "acquisition_quota"
    assert result["status"] == "paused"
    assert len(core.calls) == result["requests_used"] == 4
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions").fetchone()[0] == 0


def test_durable_byte_reservation_survives_failed_transport(book):
    ident = grant(book)
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=OSError("private server prose")):
        result = backfill.run(book, "pf", ident)
    assert result["bytes_used"] == backfill.acquisition.MAX_RESPONSE_BYTES
    assert result["requests_used"] == 1
    assert "private server prose" not in json.dumps(result)


def test_no_egress_and_pending_edits_are_enforced(book, monkeypatch):
    ident = grant(book)
    monkeypatch.setenv("KASSIBER_NO_EGRESS", "1")
    core = Core([block()])
    result = run(book, ident, core)
    assert result["last_code"] == "network_egress_disabled"
    assert not core.calls
    book.execute("UPDATE profiles SET label='edited'")
    with pytest.raises(AppError) as error:
        backfill.run(book, "pf", ident)
    assert error.value.code == "transaction_active"
    assert book.in_transaction


def test_deep_reorg_withholds_uncertain_prefix_until_ancestor_verified(book):
    first = block()
    old1 = block(first[0], nonce=1)
    old2 = block(old1[0], nonce=2)
    new1 = block(first[0], nonce=3)
    new2 = block(new1[0], nonce=4)
    ident = grant(book)
    run(book, ident, Core([first, old1, old2]))
    assert run(book, ident, Core([first, new1, new2]))["cursor_height"] == 1
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions WHERE active=1").fetchone()[0] == 0
    assert run(book, ident, Core([first, new1, new2]))["cursor_height"] == 0
    assert run(book, ident, Core([first, new1, new2]))["cursor_height"] == 2
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions WHERE active=1").fetchone()[0] == 3


def test_block_inputs_resolve_prior_occurrence_and_actual_output_value(book):
    first = block()
    parent = first[2][0]
    child = Transaction(vin=[TransactionInput(parent.txid(), 0)], vout=[TransactionOutput(900, parent.vout[0].script_pubkey)])
    second = block(first[0], [child], nonce=1)
    ident = grant(book)
    run(book, ident, Core([first, second]))
    raw = json.loads(book.execute("SELECT payload_json FROM chain_analysis_reference_assertions WHERE txid=?", (child.txid().hex(),)).fetchone()[0])
    assert raw["vin"][0]["reference_occurrence_id"] == first[0] + ":0"
    assert raw["vin"][0]["prevout"]["value"] == 1000


def test_tampered_block_payload_is_not_committed(book):
    first = block()
    different = block(transactions=[Transaction(vin=first[2][0].vin, vout=[TransactionOutput(5, first[2][0].vout[0].script_pubkey)])])
    poisoned = (first[0], first[1][:160] + different[1][160:], [])
    ident = grant(book)
    result = run(book, ident, Core([poisoned]))
    assert result["last_code"] == "invalid_observation"
    assert book.execute("SELECT COUNT(*) FROM chain_analysis_reference_assertions").fetchone()[0] == 0


def test_expired_due_grant_pauses_without_request(book):
    ident = grant(book)
    book.execute("UPDATE chain_analysis_acquisition_grants SET expires_at=0 WHERE id=?", (ident,))
    book.commit()
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("egress")):
        backfill.run_due(book, "pf")
    assert backfill.get(book, "pf", ident)["status"] == "paused"


def test_real_cli_and_daemon_queue_share_authorization(book):
    from kassiber.cli.main import build_parser
    from kassiber.cli.chain_analysis import dispatch as cli
    from kassiber.core.chain_analysis_api import dispatch as api
    ident = grant(book)
    args = build_parser().parse_args(["chain-analysis", "sources", "run", ident])
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=AssertionError("not synchronous")):
        assert cli(book, args)["queued"] is True
        listed = api(book, "ui.chain_analysis.sources.list", {})
    assert listed["items"][0]["id"] == ident


def test_remote_model_cannot_manage_or_receive_acquisition_subjects(book):
    import queue
    from kassiber.daemon import AiToolRuntime, _chain_analysis_ai_payload
    from kassiber.ai.tools import get_tool
    runtime = AiToolRuntime("unused", {}, queue.Queue(), {"provider_on_device": False})
    for operation in backfill.READ_OPERATIONS | backfill.WRITE_OPERATIONS:
        kind = "ui.chain_analysis." + operation
        assert get_tool(kind) is not None
        with pytest.raises(AppError) as error:
            _chain_analysis_ai_payload(book, runtime, kind, {})
        assert error.value.code == "local_provider_required"


def test_stale_revocation_and_duplicate_worker_cannot_change_grant(book):
    ident = grant(book)
    with pytest.raises(AppError):
        backfill.revoke(book, "pf", {"id": ident, "expected_revision": 3})
    book.execute("UPDATE chain_analysis_acquisition_grants SET lease_token='other',lease_until=9999999999 WHERE id=?", (ident,))
    book.commit()
    with pytest.raises(AppError) as error:
        backfill.run(book, "pf", ident)
    assert error.value.code == "acquisition_running"
    assert backfill.get(book, "pf", ident)["requests_used"] == 0


def test_wrong_genesis_and_wrong_book_do_not_save(book):
    ident = grant(book)
    def wrong(*args, **kwargs):
        request = json.loads(args[0].data)
        return io.BytesIO(json.dumps({"id": request["id"], "result": "0" * 64}).encode())
    with patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=wrong):
        result = backfill.run(book, "pf", ident)
    assert result["last_code"] == "backend_network_mismatch"
    assert result["status"] == "paused"
    with pytest.raises(AppError) as error:
        backfill.get(book, "different-profile", ident)
    assert error.value.code == "not_found"
