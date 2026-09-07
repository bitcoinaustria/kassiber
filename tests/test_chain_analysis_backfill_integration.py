"""Actual keyed book, domain, acquisition, incremental index and watch lifecycle.

Only the explicitly selected Core transport is synthetic.
"""
from unittest.mock import patch

from embit.script import Script
from embit.transaction import Transaction, TransactionInput, TransactionOutput

from kassiber.core import chain_analysis_backfill as backfill
from kassiber.core.chain_analysis import run_analysis
from kassiber.core.chain_analysis_api import dispatch
from kassiber.daemon_chain_analysis_watches import worker_tick
from tests.test_chain_analysis_backfill import Core, block
from tests.test_chain_analysis_watch_integration import encrypted_book  # noqa: F401


def test_worker_acquires_then_notifies_once_without_touching_accounting(encrypted_book):
    conn, _ = encrypted_book
    conn.execute("UPDATE backends SET kind='bitcoinrpc',config_json='{\"username\":\"synthetic\",\"password\":\"synthetic\"}' WHERE name='node'")
    conn.commit()
    first = block()
    spec = {"backend": "node", "network": "main", "mode": "blocks", "start_height": 0,
            "interval_seconds": 30, "max_requests": 100, "max_bytes": 100_000_000}
    core = Core([first])
    before = [tuple(row) for row in conn.execute("SELECT * FROM transactions")]
    with patch.dict(backfill.acquisition.GENESIS, {("bitcoin", "main"): first[0], ("bitcoin", "regtest"): first[0]}), patch.object(backfill.acquisition.transport, "urlopen_with_proxy", side_effect=core):
        plan = backfill.plan(conn, "profile", spec)
        source = backfill.authorize(conn, "profile", {"plan": plan})
        conn.commit()
        assert core.calls == []
        worker_tick(conn)  # Sources run even when no watch exists yet.
        state = backfill.get(conn, "profile", source["id"])
        assert state["cursor_height"] == 0, state
        txid = first[2][0].txid().hex()
        definition = {"rule": "output_spent", "query": {"subject": f"{txid}:0", "chain": "bitcoin", "network": "main", "observer": "public"}}
        preview = dispatch(conn, "ui.chain_analysis.watches.preview", definition)
        dispatch(conn, "ui.chain_analysis.watches.create", {"plan": preview})
        spend = Transaction(vin=[TransactionInput(first[2][0].txid(), 0)], vout=[TransactionOutput(900, Script(bytes.fromhex("0014" + "22" * 20)))])
        core.blocks.append(block(first[0], [spend], nonce=1))
        conn.execute("UPDATE chain_analysis_acquisition_grants SET next_run_at=0")
        conn.commit()
        worker_tick(conn)
        inbox = dispatch(conn, "ui.chain_analysis.watches.inbox", {})
        assert [item["code"] for item in inbox["items"]] == ["spend_observed"]
        result = run_analysis(conn, "profile", definition["query"])
        assert any(node.get("txid") == spend.txid().hex() for node in result["nodes"])
        conn.commit()
        calls = len(core.calls)
        worker_tick(conn)
        assert len(core.calls) == calls  # Approved cadence is honored.
        assert dispatch(conn, "ui.chain_analysis.watches.inbox", {}) == inbox
        backfill.revoke(conn, "profile", {"id": source["id"], "expected_revision": 1})
        conn.commit()
        worker_tick(conn)
        assert len(core.calls) == calls
    assert [tuple(row) for row in conn.execute("SELECT * FROM transactions")] == before
