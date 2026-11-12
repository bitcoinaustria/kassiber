#!/usr/bin/env python3
"""Freeze review input requests from current Python over the review-case books."""
import argparse
import json
from pathlib import Path
import sqlite3
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import custody_journal, review_workflow as review

CLOCK = "2026-10-31T12:00:00Z"
SOURCE = ROOT / "crates/transfers/tests/review_cases.json"
DESTINATION = ROOT / "crates/transfers/tests/review_inputs.json"


def book(fixture, scenario):
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=OFF")
    for statement in [*fixture["schema"], *scenario["sql"], *fixture["objects"]]:
        conn.executescript(statement)
    conn.execute("PRAGMA foreign_keys=ON")
    conn.commit()
    return conn


def arguments(conn, profile, first):
    version = int(profile["journal_input_version"] or 0)
    with patch.object(custody_journal, "build_ledger_state",
                      lambda c, p: custody_journal.CustodyJournalBuilder(c, p, as_of=CLOCK).build()):
        held = sorted(str(row["transaction_id"]) for row in review._build(conn, profile)["quarantines"])
    live = [row[0] for row in conn.execute("SELECT id FROM transactions WHERE profile_id='p' ORDER BY id")]
    cases = ["quarantine:" + tx for tx in held]
    result = []
    if cases:
        result += [
            dict(action="connect_wallet", case_ids=list(reversed(cases[:20])), expected_input_version=version),
            dict(action="attach_evidence", case_ids=cases[:1], expected_input_version=version),
            dict(action="import_history", case_ids=cases[-1:], expected_input_version=version,
                 explanation="\t Older history\nis missing \n"),
            dict(action="connect_wallet", case_ids=cases[:1], expected_input_version=version + 1),
            dict(action="connect_wallet", case_ids=[*cases[:1], "quarantine:nope"], expected_input_version=version),
        ]
    result += [dict(action="connect_wallet", case_ids=["quarantine:" + tx], expected_input_version=version)
               for tx in live if tx not in held][:1]
    if not first:
        return result
    case = cases[:1] or ["quarantine:nope"]
    for action in ("export", None, ["connect_wallet"], "Connect_wallet"):
        result.append(dict(action=action, case_ids=case, expected_input_version=version))
    for ids in ("quarantine:a", [], [f"quarantine:{i}" for i in range(21)], ["quarantine:a"] * 2,
                ["a"], ["quarantine:" + "x" * 290], ["quarantine:" + "x" * 289], [None], [["quarantine:a"]],
                ["Quarantine:a"]):
        result.append(dict(action="import_history", case_ids=ids, expected_input_version=version))
    result.append(dict(action="attach_evidence", case_ids=["quarantine:a", "quarantine:b"],
                       expected_input_version=version))
    for explanation in ("", " \n ", "x" * 1000, " " + "x" * 1000 + " ", "x" * 1001, "a\rb", "a\x00",
                        "a\x7f", "🍀 ok", 5, ["text"], False):
        result.append(dict(action="connect_wallet", case_ids=case, expected_input_version=version,
                           explanation=explanation))
    for expected in (-1, str(version), True, float(version), None, [version], 2**70):
        result.append(dict(action="connect_wallet", case_ids=case, expected_input_version=expected))
    return result


def outcome(conn, profile, args):
    with patch.object(custody_journal, "build_ledger_state",
                      lambda c, p: custody_journal.CustodyJournalBuilder(c, p, as_of=CLOCK).build()):
        try:
            return dict(result=review.request_input(conn, profile, **args))
        except Exception as error:
            return dict(error=dict(code=getattr(error, "code", type(error).__name__), message=str(error),
                                   details=getattr(error, "details", None)))


def export():
    fixture = json.loads(SOURCE.read_text())
    cases = []
    for index, scenario in enumerate(fixture["scenarios"]):
        conn = book(fixture, scenario)
        profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
        for args in arguments(conn, profile, index == 0):
            cases.append(dict(scenario=scenario["name"], args=args, **outcome(conn, profile, args)))
            assert not conn.in_transaction
        conn.close()
    return json.dumps(cases, ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    encoded = export()
    if parser.parse_args().check:
        assert DESTINATION.read_text() == encoded, "Review input fixtures differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print(f"{len(json.loads(encoded))} Python review input requests")
