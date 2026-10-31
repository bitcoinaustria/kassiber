"""Compare native SQLite facts with the live Python loader, byte for byte."""
import json
import os
import sqlite3
import subprocess
from pathlib import Path

import pytest

from kassiber.core.custody_snapshot import load_custody_snapshot
from tests.rust_port.custody_loader_oracle import compare
from tests.rust_port.custody_loader_oracle import exception_boundaries
from tests.test_custody_observation_consistency import (
    SCRIPT_A, SCRIPT_B, _graph, _native_leg, _profile, _source_inventory,
    custody_book,  # noqa: F401
)
from tests.test_source_overlap import _descriptor_config, ADDR_A, ADDR_B

ROOT = Path(__file__).resolve().parents[1]
BINARY = os.environ.get("KASSIBER_TEST_CUSTODY_LOADER_BIN", str(ROOT / "target/debug/examples/custody-load"))
pytestmark = pytest.mark.skipif(not Path(BINARY).is_file(), reason="requires custody-load test-harness example")
AS_OF = "2026-10-31T12:00:00Z"


@pytest.mark.parametrize("error", [
    ValueError("invalid literal for int() with base 10: 'legacy scalar'"),
    TypeError("Object of type bytes is not JSON serializable"),
    sqlite3.OperationalError("Could not decode to UTF-8 column 'note' with text '�'"),
    ValueError("password=synthetic-private-value"),
])
def test_loader_refusal_oracle_matches_cli_and_daemon_envelopes(tmp_path, monkeypatch, capsys, error):
    import importlib
    import io
    from types import SimpleNamespace
    from kassiber import daemon

    main = importlib.import_module("kassiber.cli.main")

    def refuse(*_args, **_kwargs):
        raise error

    monkeypatch.setattr(main, "dispatch", refuse)
    monkeypatch.setattr(main, "write_error_diagnostics", lambda *_a, **_kw: None)
    monkeypatch.setattr(main, "_configure_cli_logging", lambda _a: None)
    assert main.main(["--machine", "--data-root", str(tmp_path), "mcp", "status"]) == 1
    cli = json.loads(capsys.readouterr().out)["error"]
    monkeypatch.setattr(daemon, "handle_request", refuse)
    output = io.StringIO()
    args = SimpleNamespace(data_root=tmp_path, runtime_config={})
    assert daemon.run(None, args, stdin=io.StringIO('{"kind":"status"}\n'), stdout=output) == 0
    rendered = json.loads(output.getvalue().splitlines()[-1])["error"]
    actual = {"cli": {k: cli[k] for k in ("code", "message")},
              "daemon": {k: rendered[k] for k in ("code", "message")}}
    assert actual == exception_boundaries(error)


@pytest.mark.parametrize("column,payload", [
    ("note", b"\xff\x00"),
    ("note", b"a" * 250 + b"\xff"),
    ("note", "钱".encode() * 100 + b"\xff"),
    ("_closed_confirmation_observation", b"\xff"),
    ("钱包", b"\xff"),
])
def test_loader_utf8_refusal_matches_product_boundaries(loose_book, column, payload):
    conn, helper = loose_book
    for tx_id in ("first", "second"):
        helper._tx(conn, tx_id=tx_id, wallet_id="wallet-a", direction="inbound",
                   amount=1000, external_id="acq", raw_json="{}")
    if column != "note":
        conn.execute(f'ALTER TABLE transactions ADD COLUMN "{column}"')
    sql = "\n".join(conn.iterdump())
    mutation = f'UPDATE transactions SET "{column}"=CAST(x\'{payload.hex()}\' AS TEXT) WHERE id=\'second\''
    conn.execute(mutation)
    with pytest.raises(sqlite3.OperationalError) as caught:
        load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    response = subprocess.run(
        [BINARY], input=json.dumps({"sql": sql + ";\n" + mutation + ";",
                                   "profile": dict(_profile(conn)), "as_of": AS_OF}).encode(),
        capture_output=True, check=True, timeout=120,
    )
    assert not response.stderr
    actual = json.loads(response.stdout)
    assert actual["boundaries"] == exception_boundaries(caught.value)
    assert "v1" not in actual


@pytest.mark.parametrize("amount,fee", [
    ("legacy scalar", 0), (100000000, "legacy scalar"),
    ("bad amount", "bad fee"), ("", "bad fee"),
])
def test_loader_refuses_confirmation_quantity_before_output(loose_book, amount, fee):
    conn, helper = loose_book
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE transactions SET amount=?,fee=? WHERE id='wallet-b-leg'", (amount, fee))
    _check_refusal(conn)


@pytest.mark.parametrize("stale", ["version", "graph"])
def test_loader_skips_confirmation_quantity_when_authority_is_stale(loose_book, stale):
    conn, helper = loose_book
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    if stale == "version":
        conn.execute("UPDATE chain_observation_provenance SET authority_version=2")
    else:
        conn.execute("UPDATE chain_observation_provenance SET graph_hash='stale'")
    conn.execute("UPDATE transactions SET amount='legacy scalar'")
    check(conn)


def test_loader_utf8_optional_reviews_preserve_python_fallback(loose_book):
    conn, helper = loose_book
    helper._tx(conn, tx_id="included", wallet_id="wallet-a", direction="inbound",
               amount=1000, external_id="acq", raw_json="{}")
    insert(conn, "custody_gap_reviews", "review", gap_id="gap", revision=2)
    sql = "\n".join(conn.iterdump())
    mutation = "UPDATE custody_gap_reviews SET reason=CAST(x'ff00' AS TEXT)"
    conn.execute(mutation)
    expected = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    assert expected["gap_reviews"] == []
    response = subprocess.run(
        [BINARY], input=json.dumps({"sql": sql + ";\n" + mutation + ";",
                                   "profile": dict(_profile(conn)), "as_of": AS_OF}).encode(),
        capture_output=True, check=True, timeout=120,
    )
    assert not response.stderr
    assert json.loads(response.stdout)["v1"] == json.dumps(expected, ensure_ascii=True, allow_nan=True)


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


@pytest.mark.parametrize("storage", ["NULL", "7", "1.5", "'physical legacy value'", "x'ff00'"])
def test_loader_drops_closed_confirmation_from_safe_rows(loose_book, storage):
    conn, helper = loose_book
    conn.execute('ALTER TABLE transactions ADD COLUMN "_closed_confirmation_observation"')
    conn.execute('ALTER TABLE custody_components ADD COLUMN "_closed_confirmation_observation"')
    for tx_id, excluded in (("included", 0), ("anchor", 1)):
        helper._tx(conn, tx_id=tx_id, wallet_id="wallet-a", direction="inbound",
                   amount=100000, external_id="acq", raw_json="{}")
        conn.execute("UPDATE transactions SET excluded=? WHERE id=?", (excluded, tx_id))
    conn.execute(f'UPDATE transactions SET "_closed_confirmation_observation"={storage}')
    insert(conn, "custody_components", "component", state="active", revision=1,
           supersedes_component_id=None, superseded_by_component_id=None,
           _closed_confirmation_observation="retained raw header")
    insert(conn, "custody_component_legs", "leg", component_id="component", ordinal=0,
           transaction_id="anchor", anchor_transaction_id=None, wallet_id="wallet-a")
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    assert "_closed_confirmation_observation" not in snapshot["transactions"][0]
    assert "_closed_confirmation_observation" not in snapshot["anchors"]["anchor"]
    assert snapshot["components"][0]["header"]["_closed_confirmation_observation"] == "retained raw header"
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


@pytest.mark.parametrize("checkpoint,output_index,inventory_index", [
    ({"0": -99}, 1, None),
    ({"0": 5, "00": 0}, 3, None),
    ({"0": "3", "bad": 999}, 4, None),
    ({"0": 2.9}, 4, None),
    ({"0": None}, 1, None),
    ({"0": -99}, 3, 3),
])
def test_loader_receipt_high_water_precedence(loose_book, checkpoint, output_index, inventory_index):
    from tests.test_source_overlap import _descriptor_target
    conn, helper = loose_book
    config = _descriptor_config(2)
    target = _descriptor_target(config, output_index)
    conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id='wallet-a'", (json.dumps(config),))
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-b'", (json.dumps({"chain": "bitcoin", "network": "main"}),))
    insert(conn, "freshness_source_states", "checkpoint", source_key="onchain_wallet:wallet-a",
           checkpoint_json=json.dumps({"highest_used": checkpoint}))
    if inventory_index is not None:
        insert(conn, "wallet_utxos", "inventory", wallet_id="wallet-a", chain="bitcoin", network="main",
               txid="aa" * 32, vout=0, address=None, script_pubkey=target.script_pubkey,
               branch_index=0, branch_label="receive", address_index=inventory_index, raw_json="{}")
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(target.script_pubkey, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, target.script_pubkey, graph)
    check(conn)


@pytest.mark.parametrize("scan,used,history_floor,index", [
    (-2, None, 502, 502),
    ("505", None, 0, 505),
    (False, 501, 0, 503),
    ("invalid", 501, "invalid", 503),
])
def test_loader_owned_high_water_and_retired_floor(loose_book, scan, used, history_floor, index):
    from tests.test_source_overlap import _descriptor_target
    conn, helper = loose_book
    config = {**_descriptor_config(2), "ownership_scan_to_index": scan}
    target = _descriptor_target(config, index)
    conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id='wallet-a'", (json.dumps(config),))
    for ordinal, status in enumerate(("retired", "active")):
        insert(conn, "wallet_policy_epochs", f"history-{ordinal}", wallet_id="wallet-a", status=status,
               private_material_json=json.dumps({**config, "scan_to_index": history_floor}),
               chain="bitcoin", network="main", created_at=f"2025-0{ordinal+1}-01")
    if used is not None:
        insert(conn, "wallet_utxos", "high-water", wallet_id="wallet-a", chain="bitcoin", network="main",
               txid="aa" * 32, vout=0, address=ADDR_A, script_pubkey=SCRIPT_A,
               branch_label="change", branch_index=1, address_index=used, raw_json="{}")
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, target.script_pubkey, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    check(conn)


@pytest.mark.parametrize("prior", ["receipt", "network", "tax", "component"])
def test_loader_receipt_error_precedence(loose_book, prior):
    conn, helper = loose_book
    config = {"chain": "bitcoin", "network": "main", "addresses": [ADDR_A], "deprecated": "invalid"}
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps(config),))
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(SCRIPT_A, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, SCRIPT_A, graph)
    if prior == "network":
        insert(conn, "book_network_bindings", "binding", environment="test", chain_instance_id=None)
    if prior == "tax":
        conn.execute("UPDATE profiles SET tax_country='invalid'")
    if prior == "component":
        insert(conn, "custody_components", "invalid-component", revision=None,
               component_type="manual_bridge", conservation_mode="quantity", state="active",
               expected_leg_count=0, expected_allocation_count=0, expected_economic_term_count=0,
               expected_evidence_count=0, supersedes_component_id=None, superseded_by_component_id=None,
               conversion_policy="carrying-value", conversion_reviewed=0)
    profile = dict(_profile(conn))
    with pytest.raises(Exception) as caught:
        load_custody_snapshot(conn, profile, as_of=AS_OF)
    expected = {"family": type(caught.value).__name__, "message": str(caught.value),
                "code": getattr(caught.value, "code", None)}
    assert expected["family"] == ("ValueError" if prior == "tax" else "TypeError" if prior == "component" else "AppError")
    request = {"sql": "\n".join(conn.iterdump()), "profile": profile, "as_of": AS_OF}
    result = subprocess.run([BINARY], input=json.dumps(request).encode(), capture_output=True,
                            check=True, timeout=120)
    assert not result.stderr
    actual = json.loads(result.stdout)
    assert {key: actual.get("refusal", {}).get(key) for key in expected} == expected
    assert actual.get("boundaries") == exception_boundaries(caught.value)
    assert "v1" not in actual


@pytest.mark.parametrize("escaped", [False, True])
def test_loader_receipt_identity_from_graph_without_external_txid(loose_book, escaped):
    conn, helper = loose_book
    config = {"chain": "bitcoin", "network": "main", "addresses": [ADDR_A, ADDR_B]}
    conn.execute("UPDATE wallets SET config_json=?", (json.dumps(config),))
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(SCRIPT_A, 100000), (SCRIPT_B, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, SCRIPT_A, graph)
    conn.execute("UPDATE transactions SET external_id='invoice-reference'")
    if escaped:
        for row in conn.execute("SELECT id,raw_json FROM transactions").fetchall():
            # Erase every literal hex run without changing the decoded graph.
            raw = row["raw_json"].replace("d", "\\u0064").replace("c", "\\u0063")
            conn.execute("UPDATE transactions SET raw_json=? WHERE id=?", (raw, row["id"]))
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    assert snapshot["receipt_sources"]
    check(conn)


@pytest.mark.parametrize("script,raw,address,deprecated,expected_count", [
    (None, {}, None, "invalid", 0),
    (None, {"script_pubkey": SCRIPT_A}, None, False, 1),
    ("", {}, ADDR_A, False, 1),
    (" \t", {"script_pubkey": SCRIPT_A}, None, False, 1),
])
def test_loader_receipt_inventory_script_fallback(loose_book, script, raw, address, deprecated, expected_count):
    conn, helper = loose_book
    config = {"chain": "bitcoin", "network": "main", "deprecated": deprecated}
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps(config),))
    insert(conn, "wallet_utxos", "fallback", wallet_id="wallet-a", chain="bitcoin", network="main",
           txid="aa" * 32, vout=0, script_pubkey=script, raw_json=json.dumps(raw), address=address,
           branch_index=0, branch_label="receive", address_index=0)
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(SCRIPT_A, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, SCRIPT_A, graph)
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    assert len(snapshot["receipt_sources"]) == expected_count
    check(conn)


@pytest.mark.parametrize("reverse", [False, True])
def test_loader_descriptor_cache_separates_policies_and_wallet_facts(loose_book, reverse):
    from tests.test_source_overlap import _descriptor_target
    conn, helper = loose_book
    first = _descriptor_config(2)
    second = {**first, "descriptor": first["descriptor"].replace("/0/*", "/2/*"),
              "change_descriptor": first["change_descriptor"].replace("/1/*", "/3/*")}
    if reverse:
        first, second = second, first
    for wallet, config in (("wallet-a", first), ("wallet-b", second)):
        conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id=?", (json.dumps(config), wallet))
    insert(conn, "wallet_policy_epochs", "shared-history", wallet_id="wallet-a", status="retired",
           private_material_json=json.dumps(second), chain="bitcoin", network="main", created_at="2025-01-01")
    a, b = _descriptor_target(first), _descriptor_target(second)
    assert a.script_pubkey != b.script_pubkey
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, a.script_pubkey, 50005),
                                     ("ee" * 32, 0, b.script_pubkey, 50005)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    matches = snapshot["owned_index"]["by_script"][b.script_pubkey]
    assert {(item["wallet_id"], item["source"]) for item in matches} == {
        ("wallet-a", "derived_history"), ("wallet-b", "derived")}
    check(conn)


def _check_refusal(conn):
    profile = dict(_profile(conn))
    with pytest.raises(Exception) as caught:
        load_custody_snapshot(conn, profile, as_of=AS_OF)
    expected = {"family": type(caught.value).__name__, "message": str(caught.value),
                "code": getattr(caught.value, "code", None)}
    request = {"sql": "\n".join(conn.iterdump()), "profile": profile, "as_of": AS_OF}
    result = subprocess.run([BINARY], input=json.dumps(request).encode(), capture_output=True,
                            check=True, timeout=120)
    assert not result.stderr
    actual = json.loads(result.stdout)
    assert {key: actual.get("refusal", {}).get(key) for key in expected} == expected
    assert actual.get("boundaries") == exception_boundaries(caught.value)
    assert "v1" not in actual


@pytest.mark.parametrize("config", ["null", "[]", "[1]", "true", "false", "42", "1.5", '"scalar"'])
def test_loader_ownership_config_scalar_refusals(loose_book, config):
    conn, helper = loose_book
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (config,))
    _check_refusal(conn)


def test_loader_ownership_config_refusals_follow_summary_query_order(loose_book):
    conn, helper = loose_book
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE wallets SET label='Z',config_json='[]' WHERE id='wallet-a'")
    conn.execute("UPDATE wallets SET label='A',config_json='null' WHERE id='wallet-b'")
    _check_refusal(conn)


@pytest.mark.parametrize("vout", ["", "legacy scalar", "'quoted'", "\n钱", "9" * 4301 + "x"])
def test_loader_inventory_outpoint_scalar_refusals(loose_book, vout):
    conn, helper = loose_book
    _source_inventory(conn, helper, "dd" * 32)
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE wallet_utxos SET vout=?", (vout,))
    _check_refusal(conn)


@pytest.mark.parametrize("addresses", [True, False, 7, 1.5])
def test_loader_ownership_address_scalar_refusals(loose_book, addresses):
    conn, helper = loose_book
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'",
                 (json.dumps({"chain": "bitcoin", "network": "main", "addresses": addresses}),))
    _check_refusal(conn)


DESCRIPTOR_CASES = [
    {"gap_limit": -1}, {"gap_limit": 5001}, {"gap_limit": "invalid"},
    {"descriptor": 7}, {"descriptor": ["invalid"]}, {"descriptor": {"invalid": True}},
    {"network": "invalid"}, {"chain": "invalid"},
    {"chain": "BTC", "network": "mainnet"},
    {"change_descriptor": None, "synthesize_change": False},
    {"change_descriptor": None, "synthesize_change": 0},
    {"change_descriptor": None, "descriptor_source": "bsms"},
]


@pytest.mark.parametrize("overrides", DESCRIPTOR_CASES)
def test_loader_adversarial_descriptor_fields(loose_book, overrides):
    conn, helper = loose_book
    config = {**_descriptor_config(2), **overrides}
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, SCRIPT_A, 100010)],
                   outputs=[(SCRIPT_B, 100000)], fee=10)
    _native_leg(conn, helper, "wallet-b", SCRIPT_B, graph)
    conn.execute("UPDATE wallets SET kind='descriptor',config_json=? WHERE id='wallet-a'",
                 (json.dumps(config),))
    check(conn)


def _receipt_index_book(loose_book, deprecated):
    conn, helper = loose_book
    config = {"chain": "bitcoin", "network": "main", "addresses": [ADDR_A, ADDR_B],
              "deprecated": deprecated}
    conn.execute("UPDATE wallets SET config_json=?", (json.dumps(config),))
    graph = _graph("dd" * 32, inputs=[("cc" * 32, 0, "0014" + "ab" * 20, 200000)],
                   outputs=[(SCRIPT_A, 100000), (SCRIPT_B, 100000)])
    for wallet in ("wallet-a", "wallet-b"):
        _native_leg(conn, helper, wallet, SCRIPT_A, graph)
    helper._tx(conn, tx_id="excluded", wallet_id="wallet-a", direction="inbound",
               amount=1, external_id="acq", raw_json="{}")
    conn.execute("UPDATE transactions SET excluded=1 WHERE id='excluded'")
    for row_id, wallet, profile, branch in (
        ("first", "wallet-a", "profile-1", 0),
        ("duplicate", "wallet-a", "profile-1", 0),
        ("different-branch", "wallet-a", "profile-1", 1),
        ("orphan", "missing-wallet", "profile-1", 0),
        ("foreign", "wallet-a", "other", 2),
    ):
        insert(conn, "wallet_utxos", row_id, wallet_id=wallet, profile_id=profile,
               chain="bitcoin", network="main", txid="aa" * 32, vout=0,
               address=None, script_pubkey=SCRIPT_A, branch_index=branch,
               branch_label="receive", address_index=0, raw_json="{}")
    return conn


@pytest.mark.parametrize("deprecated", [False, True, "yes"])
def test_loader_receipt_index_preserves_sources_counts_and_profile_scope(loose_book, deprecated):
    conn = _receipt_index_book(loose_book, deprecated)
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    sources = snapshot["receipt_sources"]
    inventory = [source for source in sources if source["source"] == "inventory"]
    assert len(inventory) == 3
    assert {source["active_transaction_count"] for source in inventory if source["wallet_id"] == "wallet-a"} == {1}
    assert [source["active_transaction_count"] for source in inventory if source["wallet_id"] == "missing-wallet"] == [0]
    check(conn)


@pytest.mark.parametrize("label", [None, ""])
def test_loader_inventory_labels_preserve_source_specific_fallbacks(loose_book, label):
    conn = _receipt_index_book(loose_book, False)
    conn.execute("UPDATE wallets SET label=? WHERE id='wallet-a'", (label,))
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    inventory = [source for source in snapshot["receipt_sources"]
                 if source["source"] == "inventory" and source["wallet_id"] == "wallet-a"]
    assert {source["wallet_label"] for source in inventory} == {str(label)}
    matches = snapshot["owned_index"]["by_script"][SCRIPT_A]
    assert {match["wallet_label"] for match in matches if match["wallet_id"] == "wallet-a" and match["source"] == "inventory"} == {"None" if label is None else "wallet-a"}
    check(conn)
