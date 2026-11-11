"""Pytest plugin: keep every plaintext book a test opens with a pending legacy migration.

Freezes the inputs of `gen_legacy_migration_vectors.py` while the Python
oracle still exists. Run the legacy-custody test modules with

    KASSIBER_CAPTURE_LEGACY_BOOKS=<dir> python -m pytest -p tests.rust_port.capture_legacy_books ...

Before each `open_db` of an existing plaintext book that holds a step the
Rust store hands to the custody layer (a legacy reviewed pair or payout
without a component, an unresolved migration issue, a legacy pair fee
candidate without its audit, or a componentless dismissal without a complete
relation set), the book is copied page for page with SQLite's backup API
(rowids and all) to `<dir>/<test id>-<n>.sqlite3`. Nothing about the test
changes.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import sqlite3

from kassiber import db

_DIRECTORY = os.environ.get("KASSIBER_CAPTURE_LEGACY_BOOKS")
_current = {"test": "unknown", "count": 0}

_PROBES = {
    # Older books lack some of these tables; each probe stands alone.
    "authored": "SELECT EXISTS(SELECT 1 FROM transaction_pairs WHERE component_id IS NULL)",
    "authored_payout": "SELECT EXISTS(SELECT 1 FROM direct_swap_payouts WHERE component_id IS NULL)",
    "authored_issue": "SELECT EXISTS(SELECT 1 FROM custody_authored_migration_issues WHERE resolved_at IS NULL)",
    "pair_fee": """
        SELECT NOT EXISTS(SELECT 1 FROM schema_migration_audits
                          WHERE migration_name = 'custody-legacy-pair-fee-review-v1')
           AND EXISTS(SELECT 1 FROM custody_components c
                      JOIN custody_component_legs l ON l.component_id = c.id
                      WHERE c.state = 'active' AND c.conservation_mode = 'quantity'
                        AND l.role = 'fee' AND l.notes = 'reviewed_residual:network_fee')
    """,
    "componentless": """
        SELECT EXISTS(SELECT 1 FROM custody_gap_reviews r
            WHERE r.component_id IS NULL AND r.action = 'dismissed'
              AND COALESCE(r.event_kind, 'review_decision') = 'review_decision'
              AND NOT EXISTS(SELECT 1 FROM custody_gap_review_relation_sets s WHERE s.review_id = r.id))
    """,
}


def _pending(path: Path) -> list[str]:
    try:
        conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    except sqlite3.Error:
        return []
    found = []
    try:
        for name, sql in _PROBES.items():
            try:
                if conn.execute(sql).fetchone()[0]:
                    found.append(name)
            except sqlite3.Error:
                continue
    except sqlite3.DatabaseError:
        return []
    finally:
        conn.close()
    return found


def _capture(data_root) -> None:
    path = db.resolve_database_path(data_root)
    if not path.is_file() or path.stat().st_size == 0:
        return
    with open(path, "rb") as handle:
        if handle.read(16) != b"SQLite format 3\x00":
            return
    reasons = _pending(path)
    if not reasons:
        return
    _current["count"] += 1
    target = Path(_DIRECTORY) / f"{_current['test']}-{_current['count']}.sqlite3"
    source = sqlite3.connect(path)
    destination = sqlite3.connect(target)
    try:
        source.backup(destination)
    finally:
        destination.close()
        source.close()
    (target.with_suffix(".reasons")).write_text(",".join(reasons) + "\n")


if _DIRECTORY:
    Path(_DIRECTORY).mkdir(parents=True, exist_ok=True)
    _open_db = db.open_db

    def open_db(data_root, *args, **kwargs):
        if kwargs.get("passphrase") is None:
            _capture(data_root)
        return _open_db(data_root, *args, **kwargs)

    db.open_db = open_db


def pytest_runtest_setup(item):
    _current["test"] = re.sub(r"[^A-Za-z0-9_.-]+", "_", item.nodeid.replace("tests/", ""))[:150]
    _current["count"] = 0
