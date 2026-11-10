"""BLOBs keep Python bytes identity through the native custody boundary."""
import json
import subprocess
from unittest.mock import patch

import pytest

from crates.journals.scripts.audit_loader_storage import (
    classify, classify_projection, expected, without_affinity,
)
from tests.rust_port.custody_loader_oracle import exception_boundaries
from tests.test_rust_custody_loader import (
    AS_OF, BINARY, _profile, loose_book, custody_book,  # noqa: F401
    test_loader_covers_every_structural_table_column_and_filter as seed_structural,
)
from pathlib import Path

pytestmark = pytest.mark.skipif(not Path(BINARY).is_file(), reason="requires custody-load test-harness example")


def native(conn):
    request = {"sql": "\n".join(conn.iterdump()), "profile": dict(_profile(conn)),
               "as_of": AS_OF, "project": True}
    result = subprocess.run([BINARY], input=json.dumps(request), text=True,
                            capture_output=True, check=True, timeout=120)
    assert not result.stderr
    actual = json.loads(result.stdout)
    assert actual["writes"] == 0
    if "text" in actual.get("projection", {}):
        actual["projection"] = {"value": json.loads(actual["projection"]["text"])}
    return actual


@pytest.mark.parametrize("payload", [b"\xff\0", b"77", b'{"chain":"bitcoin","network":"main"}', b""])
def test_loader_blob_note_refuses_only_at_json_encoding(loose_book, payload):
    conn, helper = loose_book
    helper._tx(conn, tx_id="blob-note", wallet_id="wallet-a", direction="inbound",
               amount=1000, external_id="acq", raw_json="{}")
    conn.execute("UPDATE transactions SET note=?", (payload,))
    reference = expected(conn, dict(_profile(conn)), project=True)
    assert reference["phase"] == "v1"
    assert reference["refusal"]["family"] == "TypeError"
    assert reference["refusal"]["message"] == "Object of type bytes is not JSON serializable"
    actual = native(conn)
    assert "v1" not in actual
    assert actual["boundaries"] == exception_boundaries(TypeError("Object of type bytes is not JSON serializable"))
    assert classify_projection(reference, actual).startswith("equal_"), (reference.get("projection"), actual)


@pytest.mark.parametrize("table,column", [
    ("transactions", "amount"), ("transactions", "fee"),
    ("transactions", "raw_json"), ("transactions", "note"),
    ("wallets", "config_json"), ("wallets", "label"),
    ("custody_components", "created_at"), ("custody_components", "revision"),
    ("custody_component_legs", "amount_msat"),
    ("custody_component_allocations", "source_amount_msat"),
    ("custody_component_economic_terms", "swap_fee_msat"),
    ("custody_component_evidence_commitments", "quantity_hash"),
    ("custody_authored_evidence_snapshots", "payload_json"),
])
@pytest.mark.parametrize("payload", [b"\xff\0", b"77", b'{"chain":"bitcoin","network":"main"}'])
def test_loader_blob_consumers_match_composed_and_transport_outcomes(loose_book, table, column, payload):
    with patch("tests.test_rust_custody_loader.check", lambda _conn: None):
        seed_structural(loose_book)
    conn = without_affinity(loose_book[0])
    try:
        conn.execute(f'UPDATE "{table}" SET "{column}"=? WHERE rowid=(SELECT MIN(rowid) FROM "{table}")', (payload,))
        assert conn.execute(f'SELECT typeof("{column}") FROM "{table}" ORDER BY rowid LIMIT 1').fetchone()[0] == "blob"
        before = conn.total_changes
        reference = expected(conn, dict(_profile(conn)), project=True)
        actual = native(conn)
        assert conn.total_changes == before
        assert classify_projection(reference, actual).startswith("equal_"), (reference.get("projection"), actual)
        assert classify(reference, actual).startswith("equal_"), (reference.get("refusal"), actual)
    finally:
        conn.close()


@pytest.mark.parametrize("table,column", [("profiles", "fiat_currency"), ("transactions", "note")])
def test_loader_gate_accepts_the_matching_bytes_refusal(loose_book, table, column):
    """A snapshot holding a BLOB has no v1 text on either side; the shadow
    gate compares the probe's encoding refusal with Python's, including a
    BLOB profile column the probe receives as the byte tag."""
    from kassiber.core.custody_snapshot import load_custody_snapshot
    from tests.rust_port.custody_loader_oracle import compare
    from tests.test_rust_custody_loader import AS_OF as as_of

    conn, helper = loose_book
    helper._tx(conn, tx_id="blob", wallet_id="wallet-a", direction="inbound",
               amount=1000, external_id="acq", raw_json="{}")
    conn.execute(f'UPDATE "{table}" SET "{column}"=?', (b"EUR",))
    profile = _profile(conn)
    snapshot = load_custody_snapshot(conn, profile, as_of=as_of)
    with pytest.raises(TypeError):
        json.dumps(snapshot)
    actual = compare(conn, profile, snapshot, binary=BINARY)
    assert "v1" not in actual
    assert actual["boundaries"] == exception_boundaries(TypeError("Object of type bytes is not JSON serializable"))
