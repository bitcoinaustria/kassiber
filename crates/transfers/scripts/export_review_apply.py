"""Freeze complete atomic-review outcomes and database rows from current Python."""
import argparse
from copy import deepcopy
import datetime
from contextlib import ExitStack
from dataclasses import replace
import json
from pathlib import Path
import sqlite3
import sys
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import _metadata_hooks
from kassiber.core import custody_authored_migration, custody_components, custody_filed_reports
from kassiber.core import custody_journal, custody_review_terms, review_workflow as review, wallets

CLOCK = "2026-10-31T12:00:00Z"
DESTINATION = ROOT / "crates/transfers/tests/review_apply.json"


class FrozenDatetime(datetime.datetime):
    """Dismissal expiry reads datetime.now() (imported at call time), not the
    injected clock; freeze it at CLOCK as the Rust replay's clock is."""

    @classmethod
    def now(cls, tz=None):
        return cls(2026, 10, 31, 12, 0, 0, tzinfo=tz or datetime.timezone.utc)


def snapshot(conn):
    result = {}
    for table, in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        query = 'SELECT * FROM "' + table + '"'
        try:
            rows = conn.execute(query + ' ORDER BY rowid')
        except sqlite3.OperationalError:
            rows = conn.execute(query)
        result[table] = [{key: {"$sqlite_blob": value.hex()} if isinstance(value, bytes) else value
                          for key, value in dict(row).items()} for row in rows]
    return result


def export():
    fixture = json.loads((ROOT / "crates/transfers/tests/review_cases.json").read_text())
    scenarios = {row["name"]: row for row in fixture["scenarios"]}
    plans = json.loads((ROOT / "crates/transfers/tests/review_plans.json").read_text())
    for name in ("mixed-metadata", "pair"):
        plan = deepcopy(next(row for row in plans if row["name"] == name))
        plan["name"] += "-filed"
        for state in ("filed", "saved"):
            plan["setup"].extend([
                f"INSERT INTO filed_report_snapshots(id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,content_sha256,classification_summary_json,gain_summary_json,report_scope_json,authored_source,created_at) VALUES('{state}','w','p','tax','{state}',2020,2022,'{'ab'*32}','{{}}','{{}}','{{}}','cli','2020')",
                f"INSERT INTO custody_filed_report_impacts(id,workspace_id,profile_id,filed_report_snapshot_id,component_id,review_id,gap_id,affected_period_start_year,affected_period_end_year,amendment_warning,created_at) VALUES('impact-{state}','w','p','{state}','historical-component','historical-review','historical-gap',2020,2022,'Review the filed report','2020')",
            ])
        plans.append(plan)
    hooks = review.ReviewHooks(metadata=replace(_metadata_hooks(), now_iso=lambda: CLOCK))
    output = []
    for plan in plans:
        if "result" not in plan:
            continue
        for source in ("user", "cli", "desktop", "ai_tool"):
            conn = sqlite3.connect(":memory:")
            conn.row_factory = sqlite3.Row
            base = plan.get("base_sql", scenarios[plan["base"]]["sql"] if plan["base"] in scenarios else [])
            sql = [*fixture["schema"], *base, *fixture["objects"], *plan["setup"]]
            conn.executescript("\n".join(statement.rstrip(";") + ";" for statement in sql))
            conn.execute("PRAGMA foreign_keys=ON")
            conn.commit()
            profile = conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone()
            before = snapshot(conn)
            with ExitStack() as stack:
                build = lambda c, p: custody_journal.CustodyJournalBuilder(c, p, as_of=CLOCK).build()
                stack.enter_context(patch.object(custody_journal, "build_ledger_state", side_effect=build))
                for module, field in ((review,"now_iso"), (custody_journal,"now_iso"),
                                      (custody_filed_reports,"now_iso"),
                                      (custody_review_terms,"now_iso"), (custody_authored_migration,"now_iso"),
                                      (custody_components,"_now_iso"), (wallets,"now_iso")):
                    stack.enter_context(patch.object(module, field, return_value=CLOCK))
                # Every newly generated identity is declared below; no clock,
                # amount, author or arbitrary field is masked in comparisons.
                stack.enter_context(patch.object(datetime, "datetime", FrozenDatetime))
                counter = iter(range(1, 10000))
                stack.enter_context(patch.object(uuid, "uuid4", side_effect=lambda: uuid.UUID(int=next(counter), version=4)))
                try:
                    receipt = review.apply_review(conn, profile, artifact=plan["result"],
                                                  idempotency_key="review-apply", hooks=hooks, authored_source=source)
                    outcome = dict(result=receipt)
                except Exception as error:
                    outcome = dict(error=dict(code=getattr(error,"code","internal_error"), message=str(error)))
            after = snapshot(conn)
            identities = []
            for table, rows in after.items():
                old = {row.get("id") for row in before[table] if "id" in row}
                for index, row in enumerate(rows):
                    id = row.get("id")
                    if isinstance(id,str) and id not in old:
                        try:
                            parsed = uuid.UUID(id)
                        except ValueError:
                            continue
                        if parsed.version == 4:
                            identities.append(dict(table=table, index=index, id=id))
            output.append(dict(name=plan["name"], source=source, base_sql=base, setup=plan["setup"], artifact=plan["result"],
                               expected=after, identities=identities, **outcome))
            conn.close()
    return json.dumps(dict(schema=fixture["schema"], objects=fixture["objects"], cases=output), ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = export()
    if args.check:
        assert DESTINATION.read_text() == encoded, "Review apply fixtures differ from Python"
    else:
        DESTINATION.write_text(encoded)
    print(f"{len(json.loads(encoded)['cases'])} complete Python atomic apply vectors")
