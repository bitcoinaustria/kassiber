"""Compare native SQLite facts with the live Python loader, byte for byte."""
import json
import os
import sqlite3
import subprocess
from pathlib import Path

import pytest

from kassiber.core.custody_snapshot import load_custody_snapshot
from tests.rust_port.custody_loader_oracle import compare
from tests.test_custody_observation_consistency import (
    SCRIPT_A, SCRIPT_B, _graph, _native_leg, _profile, _source_inventory,
    custody_book,  # noqa: F401
)
from tests.test_source_overlap import _descriptor_config, ADDR_A, ADDR_B

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get("KASSIBER_TEST_CUSTODY_LOADER_BIN", str(ROOT / "target/debug/examples/custody-load"))
pytestmark = pytest.mark.skipif(not Path(BINARY).is_file(), reason="requires custody-load test-harness example")
AS_OF = "2026-10-31T12:00:00Z"


def check(conn):
    profile = _profile(conn)
    expected = load_custody_snapshot(conn, profile, as_of=AS_OF)
    return compare(conn, profile, expected, binary=BINARY)


CONFIGS = [
    "", "{broken", "null", "[]", "42", "false",
    '{"private":"samourai","network":"main","chain":"bitcoin"}',
    '{"samourai":{"section":"postmix","secret":"hidden","group_id":"钱","role":"x"},"chain":"bitcoin"}',
    '{"samourai":[1],"network":"main"}',
    '{"chain":"bitcoin","network":"main","chain_instance_id":"é","bitcoin_network":1.0,"chain_network":null}',
    'samourai {broken', '"samourai"',
]


@pytest.mark.parametrize("config", CONFIGS)
def test_loader_normalizes_config_and_preserves_storage(custody_book, config):
    conn, helper = custody_book
    helper._tx(conn, tx_id="observed", wallet_id="wallet-a", direction="inbound",
               amount=9_007_199_254_740_993, external_id="acq", raw_json="{}")
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (config,))
    conn.execute("UPDATE transactions SET fiat_rate=1.234e-7,fiat_value_exact='12.3400',note='钱 🦀' WHERE id='observed'")
    check(conn)


@pytest.mark.parametrize("descriptor", [False, True])
def test_loader_owns_inventory_addresses_and_descriptors(custody_book, descriptor):
    conn, helper = custody_book
    txid = "dd" * 32
    _source_inventory(conn, helper, txid)
    if descriptor:
        conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id='wallet-a'",
                     (json.dumps(_descriptor_config(2)),))
    graph = _graph(txid, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    check(conn)


@pytest.mark.parametrize("descriptor", [False, True])
def test_loader_reads_receipt_sources(custody_book, descriptor):
    conn, helper = custody_book
    config = {"chain": "bitcoin", "network": "main", "addresses": [ADDR_A, ADDR_B]}
    for wallet in ("wallet-a", "wallet-b"):
        conn.execute("UPDATE wallets SET kind='address',config_json=? WHERE id=?", (json.dumps(config), wallet))
    if descriptor:
        conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id='wallet-a'", (json.dumps(_descriptor_config(2)),))
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(SCRIPT_A, 100000), (SCRIPT_B, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, SCRIPT_B, graph)
    check(conn)


@pytest.fixture
def loose_book(custody_book):
    """Current column order/affinities, with permissive historical storage.

    No migration or authoring API is under test here. Removing constraints lets
    the loader oracle exercise dangling references and legacy storage classes.
    """
    source, helper = custody_book
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    for (table,) in source.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"):
        columns = source.execute(f'PRAGMA table_info("{table}")').fetchall()
        # Keep defaults such as transactions.excluded=0. Removing them makes
        # helper-authored observations disappear from both loaders and turns
        # ownership/receipt comparisons into vacuous empty-snapshot matches.
        ddl = ",".join(
            f'"{column["name"]}" {column["type"]}'
            + (f' DEFAULT {column["dflt_value"]}' if column["dflt_value"] is not None else "")
            for column in columns
        )
        conn.execute(f'CREATE TABLE "{table}" ({ddl})')
    conn.execute("CREATE UNIQUE INDEX fixture_provenance_identity ON chain_observation_provenance(transaction_id)")
    helper._seed(conn)
    try:
        yield conn, helper
    finally:
        conn.close()


def insert(conn, table, row_id, **overrides):
    columns = conn.execute(f'PRAGMA table_info("{table}")').fetchall()
    row = {}
    for ordinal, column in enumerate(columns):
        kind = column["type"].upper()
        row[column["name"]] = ordinal + 1 if "INT" in kind else ordinal + 0.25 if any(t in kind for t in ("REAL", "FLOAT", "DOUBLE")) else f"{row_id}:{column['name']}:钱"
    row.update(id=row_id, profile_id="profile-1", workspace_id="ws-1")
    row = {key: value for key, value in row.items() if key in {c["name"] for c in columns}}
    row.update(overrides)
    names = ",".join(f'"{name}"' for name in row)
    conn.execute(f'INSERT INTO "{table}" ({names}) VALUES ({",".join("?" for _ in row)})', tuple(row.values()))


def test_loader_covers_every_structural_table_column_and_filter(loose_book):
    conn, helper = loose_book
    for tx_id, excluded in (("included", 0), ("excluded", 1)):
        helper._tx(conn, tx_id=tx_id, wallet_id="wallet-a", direction="inbound",
                   amount=100000, external_id="acq", raw_json="{}")
        conn.execute("UPDATE transactions SET excluded=? WHERE id=?", (excluded, tx_id))
    insert(conn, "book_network_bindings", "binding", environment="main", chain_instance_id=None)
    for table in ("chain_analysis_observations", "chain_observer_instances", "chain_observation_provenance", "wallet_policy_epochs", "wallet_utxos"):
        insert(conn, table, "reference-" + table, chain="bitcoin", network="main",
               **({"transaction_id": "unrelated", "authority_version": None} if table == "chain_observation_provenance" else {}))
    conn.execute("UPDATE wallet_policy_epochs SET private_material_json='{}'")
    insert(conn, "wallet_policy_sources", "source", epoch_id="reference-wallet_policy_epochs")
    for name, created in (("active-b", "2026-02-01"), ("active-a", "2026-01-01"), ("inactive", "2025-01-01")):
        insert(conn, "custody_components", name, state="superseded" if name == "inactive" else "active",
               created_at=created, revision=2, supersedes_component_id="inactive", superseded_by_component_id=None)
    for ordinal, tx in enumerate(("included", "excluded", "missing")):
        insert(conn, "custody_component_legs", f"leg-{ordinal}", component_id="active-a", ordinal=ordinal,
               transaction_id=tx, anchor_transaction_id="included", wallet_id="wallet-a")
    insert(conn, "custody_component_legs", "orphan-wallet", component_id="active-b", ordinal=0,
           transaction_id=None, anchor_transaction_id=None, wallet_id="missing")
    for table in ("custody_component_allocations", "custody_component_economic_terms", "custody_component_evidence_commitments"):
        insert(conn, table, "second", component_id="active-a", ordinal=2)
        insert(conn, table, "first", component_id="active-a", ordinal=1)
    insert(conn, "custody_component_evidence_commitments", "foreign", component_id="active-a", profile_id="other", ordinal=0)
    insert(conn, "backends", "backend", name="node", chain="bitcoin", network="main")
    insert(conn, "lightning_node_records", "channel", wallet_id="wallet-a", backend_name="node", record_type="channel")
    insert(conn, "lightning_node_records", "payment", wallet_id="wallet-a", backend_name="node", record_type="payment")
    for deleted in (None, "2026-01-01"):
        insert(conn, "loan_legs", "loan-" + str(deleted), transaction_id="included", deleted_at=deleted)
        insert(conn, "custody_authored_migration_issues", "issue-" + str(deleted), resolved_at=deleted)
    for subject in ("custody_component", "other"):
        insert(conn, "custody_authored_evidence_snapshots", "snapshot-" + subject, subject_kind=subject)
    for revision in (1, 2, 2):
        insert(conn, "custody_gap_reviews", f"review-{revision}-{conn.total_changes}", gap_id="gap", revision=revision)
    insert(conn, "transaction_pair_dismissals", "dismissal")
    check(conn)


def test_loader_supports_missing_network_table_and_legacy_rate_columns(loose_book):
    conn, helper = loose_book
    conn.execute("DROP TABLE book_network_bindings")
    conn.execute("ALTER TABLE transactions DROP COLUMN fiat_rate_exact")
    conn.execute("ALTER TABLE transactions DROP COLUMN fiat_value_exact")
    helper._tx(conn, tx_id="legacy", wallet_id="wallet-a", direction="inbound", amount=10,
               external_id="acq", raw_json="{}")
    conn.execute("UPDATE transactions SET fiat_rate='legacy-text',fee='9',amount=10.5,note=7")
    check(conn)


@pytest.mark.parametrize("structured", [False, True])
def test_loader_bounds_large_gap_worklist(loose_book, structured):
    conn, helper = loose_book
    # An orphaned observation still counts toward Python's threshold, but is
    # absent from its joined worklist. Keep the wide transactions load small.
    conn.executemany("INSERT INTO transactions(id,profile_id,wallet_id,excluded) VALUES(?,?,?,0)",
                     ((f"orphan-{i}", "profile-1", "missing") for i in range(50_001)))
    for i in range(270):
        helper._tx(conn, tx_id=f"out-{i:03}", wallet_id="wallet-a", direction="outbound", amount=1000000+i,
                   external_id="acq", raw_json="{}")
        helper._tx(conn, tx_id=f"in-{i:03}", wallet_id="wallet-b", direction="inbound", amount=1000000+i,
                   external_id="acq", raw_json="{}")
    if structured:
        conn.execute("UPDATE transactions SET privacy_boundary='WhIrLpOoL' WHERE direction='outbound'")
    check(conn)


def test_loader_preserves_retired_policies_warnings_and_core_outpoints(loose_book):
    conn, helper = loose_book
    txid = "dd" * 32
    _source_inventory(conn, helper, txid)
    config = {"chain": "bitcoin", "network": "main", "ownership_scan_to_index": "invalid",
              "descriptor": "not-a-descriptor", "addresses": [ADDR_A, ADDR_A]}
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps(config),))
    for number, historic in enumerate((
        {"chain": "bitcoin", "network": "main", "addresses": [ADDR_A]},
        {**_descriptor_config(2), "scan_to_index": "invalid"},
        _descriptor_config(2),
    )):
        insert(conn, "wallet_policy_epochs", f"history-{number}", wallet_id="wallet-a", status="retired",
               private_material_json=json.dumps(historic), created_at=f"2025-0{number+1}-01")
    # Core's exact receive output, absent from inventory, is only recoverable
    # when the address index resolves exclusively to this wallet.
    helper._tx(conn, tx_id="core", wallet_id="wallet-a", direction="inbound", amount=100000,
               external_id="ee" * 32, raw_json="{}")
    conn.execute("UPDATE transactions SET external_id=?,raw_json=? WHERE id='core'",
                 ("ee" * 32, json.dumps([{"txid": "ee" * 32, "vout": "7", "address": ADDR_A}])))
    graph = _graph(txid, inputs=[("ee" * 32, 7, SCRIPT_A, 100010)], outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    check(conn)


def test_loader_opens_sqlcipher_through_store(tmp_path):
    from kassiber.db import open_db
    from tests.test_rp2_ownership_transfers import OwnershipDeriverHandlerTest
    root = tmp_path / "cipher"
    passphrase = "synthetic native-loader fixture passphrase"
    conn = open_db(root, passphrase=passphrase)
    try:
        helper = OwnershipDeriverHandlerTest()
        helper._seed(conn)
        helper._tx(conn, tx_id="cipher", wallet_id="wallet-a", direction="inbound",
                   amount=9_007_199_254_740_993, external_id="acq", raw_json="{}")
        conn.commit()
        expected = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
        request = {"root": str(root), "passphrase": passphrase,
                   "profile": dict(_profile(conn)), "as_of": AS_OF}
        result = subprocess.run([BINARY], input=json.dumps(request).encode(), capture_output=True,
                                check=True, timeout=120)
        assert result.stderr == b""
        actual = json.loads(result.stdout)
        assert "error" not in actual
        assert actual["v1"] == json.dumps(expected, ensure_ascii=True, allow_nan=True)
        # The shadow copy also includes the connection's uncommitted state.
        conn.execute("UPDATE transactions SET note='uncommitted' WHERE id='cipher'")
        check(conn)
        conn.rollback()
    finally:
        conn.close()
