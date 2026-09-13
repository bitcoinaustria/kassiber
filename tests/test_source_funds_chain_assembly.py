"""A spend funded by the wallet's own earlier outputs assembles without manual linking."""
import json

import pytest

from kassiber import daemon
from kassiber.core import source_funds
from kassiber.db import open_db, set_setting
from tests.custody_tax_helpers import persist_authoritative_chain_observation

SCRIPT = "0014" + "ab" * 20
EXTERNAL = "0014" + "cd" * 20
# Mirrors the reported book: several owned inputs consolidating into one spend
# whose inputs sum to amount + fee exactly.
PARENTS = [("a1" * 32, 200_000), ("b2" * 32, 300_000), ("c3" * 32, 500_000)]
SPEND_TXID = "d4" * 32
SPEND_SATS = 999_900
FEE_SATS = 100


def _graph(txid, vin, vout):
    return json.dumps({
        "txid": txid, "chain": "bitcoin", "network": "main",
        "observer_owned_scripts": [SCRIPT],
        "vin": vin, "vout": vout,
    }, sort_keys=True)


@pytest.fixture
def consolidation_book(tmp_path):
    conn = open_db(str(tmp_path))
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','WS','2026')")
    conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at)"
        " VALUES('profile','ws','P','EUR','generic','FIFO','2026')"
    )
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)"
        " VALUES('w','ws','profile','Onchain','descriptor',?,'2026')",
        (json.dumps({"chain": "bitcoin", "network": "main"}),),
    )
    for index, (txid, sats) in enumerate(PARENTS):
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
            "external_id_kind,occurred_at,direction,asset,amount,fee,amount_includes_fee,raw_json,"
            "fiat_rate,created_at)"
            " VALUES(?,'ws','profile','w',?,?,'txid',?,'inbound','BTC',?,0,0,?,50000,'2026')",
            (f"p{index}", f"fp-p{index}", txid, f"2026-01-0{index + 1}T00:00:00Z", sats * 1000,
             _graph(txid, [], [{"n": 0, "scriptpubkey": SCRIPT, "value": sats}])),
        )
        conn.execute(
            "INSERT INTO wallet_utxos(workspace_id,profile_id,wallet_id,chain,network,asset,outpoint,"
            "txid,vout,amount,branch_label,spent_by,confirmation_status,first_seen_at,last_seen_at)"
            " VALUES('ws','profile','w','bitcoin','main','BTC',?,?,0,?,'receive',?,'confirmed',"
            "'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
            (f"{txid}:0", txid, sats * 1000, SPEND_TXID),
        )
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
        "external_id_kind,occurred_at,direction,asset,amount,fee,amount_includes_fee,raw_json,"
        "fiat_rate,created_at)"
        " VALUES('spend','ws','profile','w','fp-spend',?,'txid','2026-02-01T00:00:00Z','outbound',"
        "'BTC',?,?,0,?,50000,'2026')",
        (SPEND_TXID, SPEND_SATS * 1000, FEE_SATS * 1000,
         _graph(SPEND_TXID,
                [{"txid": t, "vout": 0, "prevout": {"scriptpubkey": SCRIPT, "value": s}} for t, s in PARENTS],
                [{"n": 0, "scriptpubkey": EXTERNAL, "value": SPEND_SATS}])),
    )
    conn.commit()
    for tx_id in ("p0", "p1", "p2", "spend"):
        persist_authoritative_chain_observation(conn, tx_id, observer_kind="bdk")
    conn.commit()
    try:
        yield conn
    finally:
        conn.close()


def _report(conn):
    return source_funds.build_report(
        conn, "ws", "profile", daemon._source_funds_hooks(),
        target_transaction_ref="spend", save_case=False,
    )


def test_an_unassembled_spend_reports_missing_history(consolidation_book):
    """The starting state: chain evidence exists but no links do."""
    report = _report(consolidation_book)
    assert report["explain_gates"]["exportable"] is False
    assert "missing_history" in {f["code"] for f in report["explain_gates"]["blockers"]}


def test_assembly_builds_the_history_from_chain_structure_alone(consolidation_book):
    """No manual linking: the spend's own observed inputs are the evidence."""
    conn = consolidation_book
    outcome = source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    assert outcome["auto_reviewed"] == 3, outcome
    assert outcome["awaiting_manual_review"] == 0

    links = source_funds.list_links(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    assert {link["method"] for link in links} == {"utxo_spend"}
    assert all(link["state"] == "reviewed" for link in links)
    # Allocations cover the spend exactly; gross carries the fee.
    assert sum(link["allocation_amount_msat"] for link in links) == SPEND_SATS * 1000
    assert sum(link["from_allocation_amount_msat"] for link in links) == (SPEND_SATS + FEE_SATS) * 1000

    report = _report(conn)
    blockers = report["explain_gates"]["blockers"]
    codes = {f["code"] for f in blockers}
    # Exact cover, so the allocation is never ambiguous.
    assert "ambiguous_allocation" not in codes, blockers
    # The dead end has moved: the spend is explained, and what now needs
    # evidence is the three deposits that funded it.
    unexplained = {f["ref"] for f in blockers if f["code"] == "missing_history"}
    assert unexplained == {"p0", "p1", "p2"}, blockers
    assert "spend" not in unexplained
    # The spend's own node now has reviewed inbound edges.
    assert len(report["graph"]["edges"]) == 3
    # Gross/route-difference only appear once the walk reaches root sources;
    # here the frontier is still transactions, so there is nothing to state yet.
    assert report["allocations"]["gross_source_requirement"] == []
