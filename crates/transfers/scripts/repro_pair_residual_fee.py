#!/usr/bin/env python3
"""Reproduce Python pair authoring omitting a separate fee for a small residual.

A 1000 msat principal plus a separately stored 10 msat miner fee can cover
a 998 msat receipt, a 2 msat suspense and that fee. The reference instead
constructs a 1000 msat source and no fee leg, then refuses activation. This
also occurs above 2**53; it is not a floating-point conversion failure.
Run with TMPDIR under this worktree's target/. No Python product fix here.
"""
import json
from pathlib import Path
import sqlite3
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from export_gap_reads import template
from export_pairs import seed
from kassiber.cli import handlers
from kassiber.errors import AppError


def main():
    for amount in (1000, 9007199254740993):
        conn = template()
        conn.row_factory = sqlite3.Row
        seed(conn)
        conn.execute("DELETE FROM transactions WHERE id NOT IN ('a','b')")
        conn.execute("UPDATE transactions SET amount=?,fee=10,amount_includes_fee=0 WHERE id='a'", (amount,))
        conn.execute("UPDATE transactions SET amount=? WHERE id='b'", (amount-2,))
        conn.commit()
        try:
            handlers.create_transaction_pair(conn, "w", "p", "a", "b")
        except AppError as error:
            validation = error.details["validation"]
            assert any(issue["code"] == "custody_suspense_fee_coverage_mismatch"
                       for issue in validation["issues"])
            assert validation["source_msat"] == amount
            fee = sum(row["fee_msat"] for row in validation["by_asset"])
            assert fee == 0
            print(json.dumps(dict(amount=amount, observed_debit_msat=amount+10,
                                  code=error.code, message=str(error),
                                  constructed_source_msat=validation["source_msat"],
                                  constructed_fee_msat=fee)))
        else:
            raise AssertionError("reference changed; review the retained repro")
        finally:
            conn.close()


if __name__ == "__main__":
    main()
