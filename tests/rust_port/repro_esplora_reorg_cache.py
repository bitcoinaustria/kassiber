"""Verify the resolved Esplora same-statistics reorg cache bug on disposable books.

Run with an owned loopback socket available:
    uv run --locked python -m tests.rust_port.repro_esplora_reorg_cache

The reference fix is ec49bf25. Incremental sync must refresh changed
confirmation evidence and saved-report impacts; a subsequent force-full sync
adds no duplicate impact. Scratch stays under the worktree's target/.
"""
import json
import os
from pathlib import Path
import tempfile
from unittest.mock import patch

from tests.rust_sync_chain import serve
from tests.rust_sync_parity import (
    connect, create_book, python_apply, python_prefetch, runtime, seed_reviewed_state,
)
from tests.test_rust_wallet_publication import _fixture


def main():
    scratch = Path(__file__).resolve().parents[2] / "target/esplora-reorg-repro"
    scratch.mkdir(parents=True, exist_ok=True)
    config, chain, rounds = _fixture("reorg")
    with tempfile.TemporaryDirectory(dir=scratch) as temporary, patch.dict(os.environ, {
        "KASSIBER_OFFLINE_PREFERENCE_FILE": str(Path(temporary) / "offline.json"),
    }):
        book = Path(temporary) / "book.sqlite"
        create_book(book, config)
        conn = connect(book)
        try:
            with serve(chain, "esplora") as url:
                rt = runtime("esplora", url)

                def sync(index, *, force_full=False):
                    fetch, checkpoints = python_prefetch(conn, rt, force_full=force_full)
                    outcome = python_apply(conn, rt, fetch, checkpoints, force_full=force_full,
                                           prefix=f"r{index}", now=index * 1000)
                    assert "result" in outcome, outcome
                    row = conn.execute("SELECT external_id,confirmed_at FROM transactions WHERE wallet_id='a'").fetchone()
                    return outcome["result"][0], tuple(row)

                _, (txid, initial_confirmation) = sync(0)
                seed_reviewed_state(conn)
                initial_status = chain.status(txid)
                rounds[1]()
                assert chain.status(txid) != initial_status
                incremental, (_, cached_confirmation) = sync(1)
                forced, (_, refreshed_confirmation) = sync(2, force_full=True)
        finally:
            conn.close()
    result = {
        "reference_fix": "ec49bf25: refresh evidence after a tip identity change",
        "incremental_records_fetched": incremental["records_fetched"],
        "incremental_saved_report_impacts": incremental["saved_report_impacts"],
        "incremental_keeps_stale_confirmation": cached_confirmation == initial_confirmation,
        "force_full_records_fetched": forced["records_fetched"],
        "force_full_saved_report_impacts": forced["saved_report_impacts"],
        "force_full_refreshes_confirmation": refreshed_confirmation != cached_confirmation,
    }
    assert result["incremental_records_fetched"] == 1
    assert result["incremental_saved_report_impacts"] == 1
    assert not result["incremental_keeps_stale_confirmation"]
    assert result["force_full_records_fetched"] == 1
    assert result["force_full_saved_report_impacts"] == 0
    assert not result["force_full_refreshes_confirmation"]
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
