"""Python's quarantine group outputs for the Rust unit tests in
crates/journals/src/quarantine/groups.rs, over groups_fixture.sql.

Each value is stored as Python's ``json.dumps`` text, so the Rust test
compares bytes, key order included.

    uv run --locked python crates/journals/scripts/gen_quarantine_group_vectors.py
"""
import json
import os
import sqlite3
import sys
from unittest.mock import patch

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, REPO)
os.chdir(REPO)

from kassiber.core import custody_journal, quarantine_review as q  # noqa: E402

FIXTURE = open("crates/journals/src/quarantine/groups_fixture.sql").read()


def conn():
    c = sqlite3.connect(":memory:")
    c.row_factory = sqlite3.Row
    c.executescript(FIXTURE)
    return c


FRESH = {"journal_input_version": 3, "last_processed_at": "2024-05-01T00:00:00Z", "needs_processing": False}
PROFILE = {"id": "p", "workspace_id": "w"}
out = {}
with patch.object(custody_journal, "projection_freshness", lambda c, p: FRESH):
    c = conn()
    out["overview"] = q.quarantine_groups(c, PROFILE)
    out["overview_limit_1"] = q.quarantine_groups(c, PROFILE, limit=1)
    pages, cursor = [], None
    while True:
        page = q.quarantine_groups(c, PROFILE, group_id="root:a", cursor=cursor, limit=1)
        pages.append(page)
        cursor = page["next_cursor"]
        if cursor is None:
            break
    out["root_pages"] = pages
    out["reason_group"] = q.quarantine_groups(c, PROFILE, group_id="reason:custody_basis_barrier")
    out["unrooted_group"] = q.quarantine_groups(c, PROFILE, group_id="unrooted:custody_basis_barrier")
    before = q.quarantine_membership(c, "p")
    c.execute("DELETE FROM journal_quarantines WHERE transaction_id IN ('a','b')")
    c.execute("INSERT INTO journal_quarantines VALUES('n','p','missing_spot_price','{}')")
    out["effect"] = q.resolution_effect(c, "p", "a", before, reprocessed=True)
    out["unmeasured"] = q.resolution_effect(c, "p", "a", before, reprocessed=False)
OUTPUT = "crates/journals/tests/data/quarantine_groups.json"
with open(OUTPUT, "w") as handle:
    json.dump({key: json.dumps(value) for key, value in out.items()}, handle, indent=1)
    handle.write("\n")
