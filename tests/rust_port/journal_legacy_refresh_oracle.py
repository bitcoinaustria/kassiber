"""Freeze full logical dumps for evidence-only legacy pair refresh."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from dataclasses import asdict
import json
import sqlite3
from unittest.mock import patch

from kassiber.core import custody_authored_migration as migration
from kassiber.core import custody_components as components
from tests.rust_port.journal_prebuild_oracle import ROOT, CLOCK, refusal, seed, transaction
from tests.rust_port.journal_persistence_oracle import dump

PATH = ROOT / "crates/journals/tests/data/prebuild-legacy.json"


def pair(conn, id="pair", out="out", target="in", amount=900):
    conn.execute("""INSERT INTO transaction_pairs(id,workspace_id,profile_id,
        out_transaction_id,in_transaction_id,kind,policy,out_amount,pair_source,created_at)
        VALUES(?,'w','p',?,?,'manual','carrying-value',?,'manual',?)""",
        (id, out, target, amount, CLOCK))


def capture(conn, name):
    conn.commit()
    before = dump(conn)
    profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
    initial_profile = profile
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("SAVEPOINT legacy_oracle")
    ticks = []
    def clock():
        now = CLOCK if name != "changing-clock" else f"2026-10-31T13:00:{len(ticks):02}Z"
        ticks.append(now)
        return now
    result = error = None
    with ExitStack() as stack:
        stack.enter_context(patch.object(migration, "now_iso", side_effect=clock))
        stack.enter_context(patch("kassiber.core.custody_components._now_iso", side_effect=clock))
        for target in ("socket.getaddrinfo", "socket.socket.connect", "socket.socket.connect_ex", "socket.socket.sendto"):
            stack.enter_context(patch(target, side_effect=AssertionError("legacy refresh attempted network I/O")))
        try:
            outcome = migration.refresh_drifted_legacy_pair_components(conn, profile_id="p")
            if outcome.changed:
                profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
            result = dict(consolidation=asdict(outcome), profile=profile)
        except Exception as exc:
            error = refusal(exc)
            conn.execute("ROLLBACK TO legacy_oracle")
    conn.execute("RELEASE legacy_oracle")
    expected = dump(conn)
    conn.rollback()
    return dict(name=name, before=before, expected=expected, result=result, error=error, ticks=ticks, profile=initial_profile)


def vectors():
    names = (
        "no-pairs", "unlinked", "unchanged", "kind-drift", "raw-drift", "confirmed-drift",
        "external-drift", "config-drift", "fee-invalid", "amount-invalid", "excluded",
        "asset-drift", "time-drift", "authored-user", "reviewed-evidence", "superseded",
        "draft", "deleted", "dangling", "cross-profile", "invalid-reviewed-amount",
        "zero-reviewed-amount", "unknown-policy", "connected-split", "connected-conversion",
        "mixed-linked-group", "unlinked-neighbor", "changing-clock", "resolved-issue",
        "repeated-issue", "two-groups", "legacy-real-amount", "legacy-text-amount",
        "invalid-created-at", "empty-created-at", "offset-created-at",
        "membership-refusal", "draft-exists",
        "reviewed-null", "reviewed-empty", "reviewed-invalid-text", "reviewed-fraction-text",
        "reviewed-exponent-text", "reviewed-unicode-text", "reviewed-i128-text",
        "reviewed-negative-i128-text", "reviewed-max-digits", "reviewed-too-many-digits",
        "reviewed-leading-zero-limit", "reviewed-real-negative", "missing-outbound",
        "missing-inbound", "missing-commitments", "resolved-issue-recurs",
        "two-groups-invalid-text", "two-groups-oversized", "reviewed-below-i128-text",
        "repeated-drift", "repeated-drift-refusal",
        "inactive-exact-with-active-revision",
    )
    output = []
    for name in names:
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        seed(conn)
        if name == "no-pairs":
            output.append(capture(conn, name))
            conn.close()
            continue
        transaction(conn, "out", direction="outbound", amount=1000, fee=10)
        transaction(conn, "in", amount=900)
        pair(conn)
        if name in {"connected-split", "repeated-drift"}:
            conn.execute("UPDATE transaction_pairs SET out_amount=400")
            transaction(conn, "in-two", amount=500)
            pair(conn, "pair-two", target="in-two", amount=500)
        if name in {"two-groups", "changing-clock", "two-groups-invalid-text", "two-groups-oversized"}:
            transaction(conn, "out-two", direction="outbound", amount=800)
            transaction(conn, "in-two", amount=800)
            pair(conn, "pair-two", "out-two", "in-two", 800)
        if name != "unlinked":
            with patch.object(migration, "now_iso", return_value=CLOCK), patch("kassiber.core.custody_components._now_iso", return_value=CLOCK):
                migration.refresh_legacy_authored_components(conn)
        pair_guards = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name='transaction_pairs'").fetchall()
        for guard in pair_guards:
            conn.execute(f'DROP TRIGGER "{guard[0]}"')
        if name == "inactive-exact-with-active-revision":
            original = conn.execute("SELECT component_id FROM transaction_pairs").fetchone()[0]
            plan = migration._retarget_revision_spec(
                migration._pair_group_spec(migration._pair_rows(conn)),
                supersedes_component_id="fixture-revision",
            )
            with patch("kassiber.core.custody_components._now_iso", return_value=CLOCK):
                components.update_component(
                    conn, original, new_component_id="000-active-revision",
                    legs=plan["legs"], allocations=plan["allocations"],
                    economic_terms=plan["terms"], preserve_planned_row_ids=True,
                    authored_source="migration", created_at=CLOCK,
                )
                components.activate_component(conn, "000-active-revision")
            conn.execute("UPDATE transaction_pairs SET component_id='000-active-revision'")
        if name not in {"unlinked", "unchanged"}:
            conn.execute("UPDATE transactions SET kind='withdrawal' WHERE direction='outbound'")
        changes = {
            "raw-drift": ("UPDATE transactions SET raw_json='{}' WHERE id='out'", ()),
            "confirmed-drift": ("UPDATE transactions SET confirmed_at=? WHERE id='out'", (CLOCK,)),
            "external-drift": ("UPDATE transactions SET external_id='new' WHERE id='out'", ()),
            "config-drift": ("UPDATE wallets SET config_json='{}' WHERE id='p'", ()),
            "fee-invalid": ("UPDATE transactions SET fee=1000 WHERE id='out'", ()),
            "amount-invalid": ("UPDATE transactions SET amount=100 WHERE id='out'", ()),
            "excluded": ("UPDATE transactions SET excluded=1 WHERE id='out'", ()),
            "asset-drift": ("UPDATE transactions SET asset='LBTC' WHERE id='out'", ()),
            "time-drift": ("UPDATE transactions SET occurred_at='2025-02-01T00:00:00Z' WHERE id='out'", ()),
            "authored-user": ("UPDATE custody_components SET authored_source='user'", ()),
            "reviewed-evidence": ("UPDATE custody_components SET evidence_kind='reviewed_custody'", ()),
            "superseded": ("UPDATE custody_components SET state='superseded'", ()),
            "draft": ("UPDATE custody_components SET state='draft'", ()),
            "deleted": ("UPDATE transaction_pairs SET deleted_at=?", (CLOCK,)),
            "dangling": ("UPDATE transaction_pairs SET component_id='absent'", ()),
            "cross-profile": ("UPDATE custody_components SET profile_id='other'", ()),
            "invalid-reviewed-amount": ("UPDATE transaction_pairs SET out_amount=1001", ()),
            "zero-reviewed-amount": ("UPDATE transaction_pairs SET out_amount=0", ()),
            "unknown-policy": ("UPDATE transaction_pairs SET policy='unknown'", ()),
            "legacy-real-amount": ("UPDATE transaction_pairs SET out_amount=900.5", ()),
            "legacy-text-amount": ("UPDATE transaction_pairs SET out_amount='9_00'", ()),
            "invalid-created-at": ("UPDATE transaction_pairs SET created_at='bad'", ()),
            "empty-created-at": ("UPDATE transaction_pairs SET created_at=''", ()),
            "offset-created-at": ("UPDATE transaction_pairs SET created_at='2026-10-31T13:00:00+01:00'", ()),
        }
        if name in changes:
            guards = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name='custody_components'").fetchall()
            if changes[name][0].startswith("UPDATE custody_components"):
                for guard in guards:
                    conn.execute(f'DROP TRIGGER "{guard[0]}"')
            conn.execute(*changes[name])
            if changes[name][0].startswith("UPDATE custody_components"):
                for guard in guards:
                    conn.execute(guard[1])
        reviewed = {
            'reviewed-null': None, 'reviewed-empty': '', 'reviewed-invalid-text': 'invalid',
            'reviewed-fraction-text': '900.5', 'reviewed-exponent-text': '9e2',
            'reviewed-unicode-text': '\u2003\u0669\u0660\u0660\u2003',
            'reviewed-i128-text': '170_141183460469231731687303715884105727',
            'reviewed-negative-i128-text': '-170_141183460469231731687303715884105728',
            'reviewed-max-digits': '1_' + '0' * 4299,
            'reviewed-too-many-digits': '1_' + '0' * 4300,
            'reviewed-leading-zero-limit': '0_' + '0' * 4300 + '900',
            'reviewed-real-negative': -900.5,
            'reviewed-below-i128-text': '-170_141183460469231731687303715884105729',
        }
        if name in {'two-groups-invalid-text', 'two-groups-oversized'}:
            conn.execute("UPDATE transaction_pairs SET out_amount=? WHERE id='pair-two'",
                         ('bad' if name == 'two-groups-invalid-text' else '1_' + '0' * 4299,))
        if name in reviewed:
            conn.execute('UPDATE transaction_pairs SET out_amount=?', (reviewed[name],))
        if name in {'missing-outbound', 'missing-inbound'}:
            conn.execute('PRAGMA foreign_keys=OFF')
            conn.execute('UPDATE transaction_pairs SET ' + ('out' if name == 'missing-outbound' else 'in') + "_transaction_id='absent'")
        if name == 'missing-commitments':
            guards = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name='custody_component_evidence_commitments'").fetchall()
            for guard, _ in guards:
                conn.execute(f'DROP TRIGGER "{guard}"')
            conn.execute('DELETE FROM custody_component_evidence_commitments')
            for _, sql in guards:
                conn.execute(sql)
        if name == 'resolved-issue-recurs':
            conn.execute("UPDATE transaction_pairs SET out_amount=1001")
            conn.execute("INSERT INTO custody_authored_migration_issues(id,workspace_id,profile_id,legacy_table,legacy_source_id,issue_code,transaction_ids_json,details_json,resolved_at,created_at,updated_at) VALUES('old-issue','w','p','transaction_pairs','pair','prior','[]','{}',?,?,?)", (CLOCK, CLOCK, CLOCK))
        if name in {"unlinked-neighbor", "connected-conversion", "mixed-linked-group"}:
            transaction(conn, "in-two", amount=100)
            pair(conn, "pair-two", target="in-two", amount=100)
            if name == "connected-conversion":
                conn.execute("UPDATE transaction_pairs SET policy='taxable' WHERE id='pair-two'")
            if name == "mixed-linked-group":
                conn.execute("UPDATE transaction_pairs SET component_id='other-component' WHERE id='pair-two'")
                conn.execute("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,authored_source,created_at) VALUES('other-component','other-lineage','w','p',1,'manual_bridge','draft','user',?)", (CLOCK,))
        if name in {"resolved-issue", "repeated-issue"}:
            conn.execute("INSERT INTO custody_authored_migration_issues(id,workspace_id,profile_id,legacy_table,legacy_source_id,issue_code,transaction_ids_json,details_json,created_at,updated_at) VALUES('old-issue','w','p','transaction_pairs','pair','prior','[]','{}',?,?)", (CLOCK, CLOCK))
            if name == "repeated-issue":
                conn.execute("UPDATE transaction_pairs SET out_amount=1001")
        if name == "membership-refusal":
            conn.execute("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,authored_source,created_at) VALUES('membership-owner','membership-lineage','w','p',1,'manual_bridge','active','user',?)", (CLOCK,))
            conn.execute("DELETE FROM custody_component_transaction_memberships WHERE transaction_id='out'")
            conn.execute("INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES('membership-owner','p','out',?)", (CLOCK,))
        if name == "draft-exists":
            conn.execute("INSERT INTO custody_components(id,lineage_id,workspace_id,profile_id,revision,component_type,state,authored_source,created_at) SELECT 'existing-draft',lineage_id,workspace_id,profile_id,2,component_type,'draft','user',created_at FROM custody_components LIMIT 1")
        for guard in pair_guards:
            conn.execute(guard[1])
        if name in {"repeated-drift", "repeated-drift-refusal"}:
            with patch.object(migration, "now_iso", return_value=CLOCK), patch("kassiber.core.custody_components._now_iso", return_value=CLOCK):
                first = migration.refresh_drifted_legacy_pair_components(conn, profile_id="p")
            assert first.activated == 1 and first.skipped == 0
            conn.execute("UPDATE transactions SET kind='withdrawal-revised' WHERE id='out'")
        record = capture(conn, name)
        if name in {"repeated-drift", "repeated-drift-refusal"}:
            assert record["error"] is None
            counts = record["result"]["consolidation"]
            assert counts["activated"] == int(name == "repeated-drift")
            assert counts["skipped"] == int(name == "repeated-drift-refusal")
        output.append(record)
        conn.close()
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--write", action="store_true")
    args = parser.parse_args()
    cases = vectors()
    base = cases[0]["before"]
    packed = dict(base=base, cases=[dict(case, **{
        side: {table: item["rows"] for table, item in case[side].items() if item != base[table]}
        for side in ("before", "expected")}) for case in cases])
    text = json.dumps(packed, ensure_ascii=True, separators=(",", ":"), allow_nan=False) + "\n"
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f"{len(cases)} complete legacy-refresh dumps match Python")


if __name__ == "__main__":
    main()
