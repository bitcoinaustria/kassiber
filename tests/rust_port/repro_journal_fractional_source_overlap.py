"""Python source repair truncates distinct legacy fractional-msat fingerprints."""
import sqlite3

from tests.rust_port.journal_prebuild_oracle import seed
from tests.rust_port.journal_source_overlap_oracle import capture
from tests.test_source_overlap import (
    _descriptor_config,
    _descriptor_target,
    _script,
    _seed_book,
    _tx,
    _wallet,
)


def main():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    try:
        seed(conn)
        _seed_book(conn)
        config = _descriptor_config()
        address = _descriptor_target(config, 0).address
        _wallet(conn, "canonical", "Descriptor", "descriptor", config)
        _wallet(conn, "duplicate", "Address", "address", dict(chain="bitcoin", network="mainnet", addresses=[address]))
        for ident, wallet in (("keep", "canonical"), ("exclude", "duplicate")):
            _tx(conn, ident, wallet, "aa" * 32, script_pubkey=_script(address))
        conn.execute("UPDATE transactions SET amount=CASE id WHEN 'keep' THEN 1.25 ELSE 1.75 END")
        amounts = [row[0] for row in conn.execute("SELECT amount FROM transactions ORDER BY id")]
        assert amounts == [1.75, 1.25]
        record = capture(conn, "fractional-msat-fingerprint")
        assert record["error"] is None, record["error"]
        assert record["result"]["repair"]["duplicates_excluded"] == 1
        print("Reproduced: source repair excludes a distinct fractional-msat observation after int() collapses 1.25 and 1.75 to 1. Python is unchanged.")
    finally:
        conn.close()


if __name__ == "__main__":
    main()
