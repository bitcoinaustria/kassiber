#!/usr/bin/env python3
"""Publication rounds where reused UTXO membership refreshes confirmations.

Round 5 (7e8c323b9): when a chain extension leaves every script history
unchanged, the compatibility adapter skips the UTXO listing and the apply
step advances the stored outputs' confirmation counts from the checkpoint's
proven tip height. Each sequence runs the merged Python against the
loopback chain of `tests/rust_sync_chain.py`; Rust replays Python's prepared
fetch through publication and must write the same rows. The old full-table
publication vectors predate this round and main's schema, so these cases
live in their own file.

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/wallets/scripts/generate_confirmation_publication_vectors.py [--check]
"""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from generate_wallet_publication_vectors import ROOT, run  # noqa: E402
from tests.test_rust_wallet_publication import _fixture  # noqa: E402

DEST = ROOT / "crates/wallets/tests/vectors/python_confirmation_publication.json"


def rounds(chain):
    def extend():
        chain.tip += 3
    # First sync, a proven three-block extension, then an unchanged tip.
    return [None, extend, None]


def vectors():
    cases = []
    for protocol in ("esplora", "electrum"):
        config, chain, _ = _fixture("checkpoint")
        cases += run("extension", protocol, config, chain, rounds(chain))
    refreshed = 0
    for case in cases:
        meta = case["fetch"]["adapter_meta"]
        if meta.get("utxos_skipped_unchanged") and case["before"].get("wallet_utxos"):
            refreshed += 1
    assert refreshed, "no round reused UTXO membership"
    return {"generator": "crates/wallets/scripts/generate_confirmation_publication_vectors.py", "cases": cases}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), sort_keys=True, ensure_ascii=False, separators=(",", ":")) + "\n"
    if args.check:
        if DEST.read_text(encoding="utf-8") != text:
            raise SystemExit(f"{DEST} is stale")
        return
    DEST.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main()
