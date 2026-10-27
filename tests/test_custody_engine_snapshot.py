"""The raw R2 request contains enough local facts without private configuration."""
import json

from kassiber.core.custody_snapshot import load_custody_snapshot
from tests.test_custody_observation_consistency import custody_book, _profile  # noqa: F401

AS_OF = "2026-01-01T00:00:00Z"


def test_snapshot_is_raw_json_with_only_configuration_facts(custody_book):
    conn, helper = custody_book
    helper._tx(conn, tx_id="receipt", wallet_id="wallet-a", direction="inbound", amount=100000, external_id="receipt", raw_json="{}")
    conn.execute("UPDATE wallets SET config_json=? WHERE id='wallet-a'", (json.dumps({"chain": "bitcoin", "network": "main", "descriptor_backup": "private-policy-must-not-cross-the-seam", "samourai": {"role": "child", "group_id": "group", "section": "postmix", "xpub": "private-key"}}),))
    snapshot = load_custody_snapshot(conn, _profile(conn), as_of=AS_OF)
    encoded = json.dumps(snapshot)
    assert "private-policy" not in encoded and "private-key" not in encoded
    assert snapshot["schema_version"] == 1
    assert snapshot["as_of"] == AS_OF
    assert snapshot["owned_index"] is None
    assert snapshot["transactions"][0]["id"] == "receipt"
    assert not {"canonical_input", "interpretation", "validation", "finalized_projection"} & snapshot.keys()
