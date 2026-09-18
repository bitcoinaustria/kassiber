"""A spend funded by the wallet's own earlier outputs assembles without manual linking."""
import json

import pytest

from kassiber import daemon
from kassiber.core import source_funds, source_funds_review
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


def test_reopening_the_book_does_not_retire_fresh_structural_links(consolidation_book):
    """The legacy method retirement must not relabel edges derived today.

    ensure_schema_compat runs on every open_db. It used to rewrite every
    `utxo_spend` row to `custody_component`, which would make each freshly
    derived edge look like a stale custody projection on the next app start.
    """
    from kassiber.db import ensure_schema_compat

    conn = consolidation_book
    source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    ensure_schema_compat(conn)

    links = source_funds.list_links(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    assert {link["method"] for link in links} == {"utxo_spend"}

    report = _report(conn)
    codes = {f["code"] for f in report["explain_gates"]["blockers"]}
    assert "stale_custody_component_lineage" not in codes
    assert "ambiguous_allocation" not in codes


def test_a_legacy_invented_link_is_refused_rather_than_relabelled(consolidation_book):
    """Pre-projection rows carried no authority gate and must not become trusted.

    They are not migrated to custody_component -- that would assert they came
    from the projection, which is its own untruth. Re-derivation refuses them.
    """
    from kassiber.db import ensure_schema_compat

    conn = consolidation_book
    # An allocation the old ungated deriver could have invented: p0 does not
    # fund p1, and no observed structure says it does.
    conn.execute(
        "INSERT INTO source_funds_links(id,workspace_id,profile_id,from_transaction_id,"
        "to_transaction_id,link_type,state,confidence,method,asset,from_asset,"
        "allocation_amount,from_allocation_amount,allocation_policy,created_at,updated_at)"
        " VALUES('legacy','ws','profile','p0','spend','self_transfer','reviewed','exact',"
        "'utxo_spend','BTC','BTC',12345,12345,'explicit','2026','2026')"
    )
    conn.commit()
    ensure_schema_compat(conn)

    assert conn.execute(
        "SELECT method FROM source_funds_links WHERE id='legacy'"
    ).fetchone()[0] == "utxo_spend"
    codes = {f["code"] for f in _report(conn)["explain_gates"]["blockers"]}
    assert "stale_structural_lineage" in codes


def test_a_reviewed_edge_stops_being_trusted_when_its_evidence_disappears(consolidation_book):
    """Auto-promotion is not permanent: the report re-checks the structure."""
    conn = consolidation_book
    source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    assert "stale_structural_lineage" not in {
        f["code"] for f in _report(conn)["explain_gates"]["blockers"]
    }

    # The observation commitment is withdrawn; the edges were only ever as good
    # as the evidence behind them.
    conn.execute("DELETE FROM chain_observation_provenance")
    conn.commit()

    codes = {f["code"] for f in _report(conn)["explain_gates"]["blockers"]}
    assert "stale_structural_lineage" in codes


def test_a_reviewed_edge_stops_being_trusted_when_its_parent_leaves_the_book(
    consolidation_book,
):
    """Re-derivation runs against current rows, not the ones that were there."""
    conn = consolidation_book
    source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    assert "stale_structural_lineage" not in {
        f["code"] for f in _report(conn)["explain_gates"]["blockers"]
    }

    # The user excludes one funding deposit; the edge that claimed it is no
    # longer supported.
    conn.execute("UPDATE transactions SET excluded = 1 WHERE id = 'p0'")
    conn.commit()

    codes = {f["code"] for f in _report(conn)["explain_gates"]["blockers"]}
    assert "stale_structural_lineage" in codes


def test_lineage_resolves_for_a_wallet_imported_after_the_outputs_were_spent(
    consolidation_book,
):
    """The reported book's shape: the wallet was first synced days AFTER the
    spend, so the backend's unspent view never contained these outputs."""
    conn = consolidation_book
    conn.execute("DELETE FROM wallet_utxos")
    conn.commit()

    outcome = source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )

    assert outcome["auto_reviewed"] == 3, outcome
    report = _report(conn)
    assert "ambiguous_allocation" not in {
        f["code"] for f in report["explain_gates"]["blockers"]
    }
    assert len(report["graph"]["edges"]) == 3


def _assembled_report(conn):
    source_funds.assemble_history(
        conn, "ws", "profile", daemon._source_funds_hooks(), target_transaction_ref="spend",
    )
    hooks = daemon._source_funds_hooks()
    source = source_funds.create_source(
        conn, "ws", "profile", hooks, source_type="fiat_purchase", label="Purchase",
        amount="0.01", acquired_at="2025-12-01T00:00:00Z", fiat_value="500",
    )
    for index, (_txid, sats) in enumerate(PARENTS):
        source_funds.create_link(
            conn, "ws", "profile", hooks, from_source_ref=source["id"],
            to_transaction_ref=f"p{index}",
            link_type="manual_source", allocation_amount=f"{sats / 100_000_000:.8f}",
        )
    return _report(conn)


def test_the_route_difference_is_named_as_the_fee_the_row_records(consolidation_book):
    """The difference IS the network fee; saying otherwise reads as evasive."""
    report = _assembled_report(consolidation_book)
    allocations = report["allocations"]

    assert allocations["route_difference_msat"] == FEE_SATS * 1000
    explanation = allocations["route_difference_explanation"]
    assert explanation["kind"] == "network_fees"
    assert explanation["fee_msat"] == FEE_SATS * 1000
    assert explanation["hop_count"] == 1

    narrative = " ".join(report["narrative"]["paragraphs"])
    assert "network fees recorded on 1 disclosed transaction" in narrative
    assert "not automatically classified as a fee" not in narrative


def _envelope(*, consumed, fee_msat, difference=None, privacy=False):
    """Minimal envelope for the route-difference licensing rule."""
    return {
        "allocations": {
            "route_difference_msat": difference if difference is not None else consumed,
        },
        "graph": {
            "nodes": [{
                "id": "tx:spend", "node_type": "transaction", "fee_msat": fee_msat,
                **({"privacy_boundary": "coinjoin"} if privacy else {}),
            }],
            "edges": [{
                "to": "tx:spend",
                "allocation_amount_msat": 1_000_000,
                "from_allocation_amount_msat": 1_000_000 + consumed,
            }],
        },
    }


def test_a_difference_is_only_named_when_the_rows_actually_account_for_it():
    """Numerical coincidence is not an explanation."""
    explain = source_funds._route_difference_explanation

    # The hop consumed exactly what it recorded as fee.
    assert explain(_envelope(consumed=1_413, fee_msat=1_413))["kind"] == "network_fees"
    # Consumed more than the row records: unexplained.
    assert explain(_envelope(consumed=1_413, fee_msat=900)) is None
    # The row records a fee, but a different amount moved.
    assert explain(_envelope(consumed=1_413, fee_msat=2_000)) is None
    # Per-hop arithmetic closes but the totals disagree.
    assert explain(_envelope(consumed=1_413, fee_msat=1_413, difference=2_000)) is None
    # Nothing to explain.
    assert explain(_envelope(consumed=0, fee_msat=0, difference=0)) is None
    # A privacy boundary disqualifies the route: Kassiber does not claim to
    # know what happened inside one.
    assert explain(_envelope(consumed=1_413, fee_msat=1_413, privacy=True)) is None


@pytest.mark.parametrize("insertion_order", ["a_first", "b_first"])
def test_two_wallets_paid_by_one_transaction_each_keep_their_own_ancestry(tmp_path, insertion_order):
    """Astra's P1, through the production path: one batched transaction pays two
    wallets; assembling each spend must never adopt the OTHER wallet's parent.

    A txid-only memo handed wallet B the ancestry cached for wallet A, and both
    bulk review and report re-verification repeated the same wrong derivation.
    """
    from kassiber.db import set_setting

    conn = open_db(str(tmp_path))
    set_setting(conn, "context_workspace", "ws")
    set_setting(conn, "context_profile", "profile")
    conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','WS','2026')")
    conn.execute(
        "INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,gains_algorithm,created_at)"
        " VALUES('profile','ws','P','EUR','generic','FIFO','2026')"
    )
    script = {"A": "0014" + "aa" * 20, "B": "0014" + "bb" * 20}
    for wallet in ("A", "B"):
        conn.execute(
            "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)"
            " VALUES(?,'ws','profile',?,'descriptor',?,'2026')",
            (wallet, f"Wallet {wallet}", json.dumps({"chain": "bitcoin", "network": "main"})),
        )
    parent = {"A": "a1" * 32, "B": "b1" * 32}
    batch, spend = "cc" * 32, {"A": "da" * 32, "B": "db" * 32}
    sats = {"A": (500_000, 499_000, 498_000), "B": (700_000, 699_000, 698_000)}

    def graph(txid, vin, vout, wallet):
        return json.dumps({"txid": txid, "chain": "bitcoin", "network": "main",
                           "observer_owned_scripts": [script[wallet]], "vin": vin, "vout": vout},
                          sort_keys=True)

    def insert(rid, wallet, txid, direction, amount_sats, fee_sats, raw, occurred):
        conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,"
            "external_id_kind,occurred_at,direction,asset,amount,fee,amount_includes_fee,raw_json,"
            "fiat_rate,created_at) VALUES(?,'ws','profile',?,?,?,'txid',?,?,'BTC',?,?,0,?,50000,'2026')",
            (rid, wallet, f"fp-{rid}", txid, occurred, direction, amount_sats * 1000, fee_sats * 1000, raw),
        )

    batch_vout = [{"n": 0, "scriptpubkey": script["A"], "value": sats["A"][1]},
                  {"n": 1, "scriptpubkey": script["B"], "value": sats["B"][1]}]
    order = ("A", "B") if insertion_order == "a_first" else ("B", "A")
    for w in order:
        deposit, received, spent = sats[w]
        vout_index = 0 if w == "A" else 1
        insert(f"parent-{w}", w, parent[w], "inbound", deposit, 0,
               graph(parent[w], [], [{"n": 0, "scriptpubkey": script[w], "value": deposit}], w),
               "2026-01-01T00:00:00Z")
        insert(f"batch-{w}", w, batch, "inbound", received, 0,
               graph(batch, [{"txid": parent[w], "vout": 0,
                              "prevout": {"scriptpubkey": script[w], "value": deposit}}], batch_vout, w),
               "2026-01-02T00:00:00Z")
        insert(f"spend-{w}", w, spend[w], "outbound", spent, received - spent,
               graph(spend[w], [{"txid": batch, "vout": vout_index,
                                 "prevout": {"scriptpubkey": script[w], "value": received}}],
                     [{"n": 0, "scriptpubkey": "0014" + "ff" * 20, "value": spent}], w),
               "2026-02-01T00:00:00Z")
    conn.commit()
    for rid in ("parent-A", "batch-A", "spend-A", "parent-B", "batch-B", "spend-B"):
        persist_authoritative_chain_observation(conn, rid, observer_kind="bdk")
    conn.commit()
    hooks = daemon._source_funds_hooks()
    try:
        for w in order:
            outcome = source_funds.assemble_history(
                conn, "ws", "profile", hooks, target_transaction_ref=f"spend-{w}",
            )
            assert outcome["auto_reviewed"] == 1, (w, outcome)
        wallet_of = {row[0]: row[1] for row in conn.execute("SELECT id, wallet_id FROM transactions")}
        links = conn.execute(
            "SELECT from_transaction_id, to_transaction_id, state FROM source_funds_links"
        ).fetchall()
        assert {(f, t) for f, t, _s in links} == {("batch-A", "spend-A"), ("batch-B", "spend-B")}, links
        for from_id, to_id, _state in links:
            assert wallet_of[from_id] == wallet_of[to_id], f"cross-wallet link {from_id} -> {to_id}"
        # Report re-verification must agree with what was persisted.
        for w in order:
            report = source_funds.build_report(
                conn, "ws", "profile", hooks, target_transaction_ref=f"spend-{w}", save_case=False,
            )
            assert "stale_structural_lineage" not in {
                f["code"] for f in report["explain_gates"]["blockers"]
            }
            assert [e["from"] for e in report["graph"]["edges"]] == [f"tx:batch-{w}"]
    finally:
        conn.close()


def test_a_legacy_book_names_the_rescan_and_heals_after_one(consolidation_book):
    """The reported book's real shape: rows synced before the observer recorded
    which inputs were its own. Assembly must not guess; it must say what fixes
    it, and an authoritative refresh must then make the lineage appear."""
    from unittest.mock import Mock
    from kassiber.core.imports import ImportCoordinatorHooks, insert_wallet_records

    conn = consolidation_book
    attested_raw = conn.execute("SELECT raw_json FROM transactions WHERE id='spend'").fetchone()[0]
    legacy = json.loads(attested_raw); legacy.pop("observer_owned_scripts")
    conn.execute("UPDATE transactions SET raw_json=? WHERE id='spend'", (json.dumps(legacy, sort_keys=True),))
    conn.commit()
    persist_authoritative_chain_observation(conn, "spend", observer_kind="bdk")  # legacy rows ARE authoritative
    conn.commit()
    hooks = daemon._source_funds_hooks()

    outcome = source_funds.assemble_history(conn, "ws", "profile", hooks, target_transaction_ref="spend")
    assert outcome["auto_reviewed"] == 0
    assert outcome["chain_attestation_missing"] == ["spend"]
    assert "chain_attestation_missing" in source_funds_review.review_context(
        conn, conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone(), hooks,
        target_transaction="spend",
    )["input_needs"]

    # What a full rescan does: the observer re-emits the row, now attesting its inputs.
    profile = conn.execute("SELECT * FROM profiles WHERE id='profile'").fetchone()
    wallet = conn.execute("SELECT * FROM wallets WHERE id='w'").fetchone()
    attested = json.loads(attested_raw)
    refreshed = insert_wallet_records(
        conn, profile, wallet,
        [{"txid": SPEND_TXID, "occurred_at": "2026-02-01T00:00:00Z", "direction": "outbound", "asset": "BTC",
          "amount": f"{SPEND_SATS / 1e8:.8f}", "fee": f"{FEE_SATS / 1e8:.8f}", "raw_json": attested}],
        "backend:fixture", ImportCoordinatorHooks(ensure_tag_row=Mock(), invalidate_journals=Mock()),
        authoritative_chain_observer=True, report_updates=True,
    )
    assert refreshed["updated"] == 1 and "raw_json" in refreshed["updated_records"][0]["changed_fields"]
    persist_authoritative_chain_observation(conn, "spend", observer_kind="bdk")
    conn.commit()

    healed = source_funds.assemble_history(conn, "ws", "profile", hooks, target_transaction_ref="spend")
    assert healed["auto_reviewed"] == 3, healed
    assert healed["chain_attestation_missing"] == []
