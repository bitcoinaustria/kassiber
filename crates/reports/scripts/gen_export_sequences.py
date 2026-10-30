#!/usr/bin/env python3
"""Freeze whole CSV export/file sequences, including every SQLite table.

Only the disposable artifact root is normalized. Clock and identities are
injected before execution; stored JSON, hashes and artifact bytes stay literal.
"""
from pathlib import Path
from dataclasses import replace
import json
import sys
import tempfile
from uuid import UUID
from unittest.mock import patch

import kassiber.cli.main as cli
from kassiber.core import reports, custody_filed_reports as snapshots
from kassiber.db import open_db
from kassiber.errors import AppError
from gen_completion_vectors import tx, status, TX, WALLETS, RELATION
from gen_artifact_vectors import FixedDateTime

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/export_sequences.json"
STAMP = "2026-01-02T03:04:05Z"
CHAIN = WALLETS + tx("c1", "chain", "BTC", "inbound", 1000, status(TX[1])) + tx(
    "c9", "liq", "LBTC", "outbound", 9000,
    dict(txid=TX[9], chain="liquid", network="liquidv1", vin=[], vout=[], status={"confirmed": True}),
) + RELATION + "UPDATE profiles SET last_processed_tx_count=6 WHERE id='p';"


def build():
    cases = []
    for country, algorithm in (("generic", "FIFO"), ("generic", "HIFO"), ("at", "MOVING_AVERAGE_AT")):
        for variant in ("ready", "capture-failure", "stale", "raw-hex"):
            # Inject a common initial migration clock before any export action.
            sql = ("DROP TRIGGER trg_schema_migration_audits_immutable;"
                   "UPDATE schema_migration_audits SET created_at='seeded';"
                   "CREATE TRIGGER trg_schema_migration_audits_immutable BEFORE UPDATE ON schema_migration_audits "
                   "BEGIN SELECT RAISE(ABORT, 'schema_migration_audits_immutable'); END;" +
                   f"UPDATE profiles SET tax_country='{country}',gains_algorithm='{algorithm}' WHERE id='p';" + CHAIN)
            if variant == "capture-failure":
                sql += "CREATE TRIGGER reject_capture BEFORE INSERT ON filed_report_chain_captures BEGIN SELECT RAISE(ABORT,'capture_rejected'); END;"
            if variant == "stale":
                sql += "UPDATE profiles SET journal_input_version=8 WHERE id='p';"
            if variant == "raw-hex":
                from gen_liquid_confirmation_vectors import raw_transactions
                from kassiber.core.chain_observer.provenance import canonical_graph_hash
                payload = json.dumps(dict(chain='liquid', network='liquidv1',
                    raw_hex=raw_transactions()['explicit'].hex(), status=dict(confirmed=True)))
                sql += f"UPDATE transactions SET raw_json='{payload}' WHERE id='c9';"
                sql += f"UPDATE chain_observation_provenance SET graph_hash='{canonical_graph_hash(payload)}' WHERE transaction_id='c9';"
            actions = [
                dict(command="reports.export-csv", args={"history_limit": 0}, target="full.csv"),
                dict(command="reports.export-csv", args={"wallet": "Cold", "history_limit": 2}, target="wallet.csv"),
                dict(command="reports.export-transactions-csv", args={}, target="transactions.csv"),
            ]
            if country == "at":
                actions.append(dict(command="reports.export-austrian-e1kv-csv", args={"year": 2024}, target="bundle"))
            if variant in ("ready", "raw-hex"):
                actions += [dict(command="reports.filed-snapshots.create", args={"state": "filed", "saved_snapshot_id": str(UUID(int=1))}) for _ in range(2)]
            with tempfile.TemporaryDirectory(prefix="export-sequence-") as root_text:
                root = Path(root_text)
                conn = open_db(root / "data")
                conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + sql)
                # Store/open compatibility is outside this slice: retain its exact starting state.
                before = dump(conn)
                for index, action in enumerate(actions):
                    args = action["args"]
                    try:
                        with patch.object(reports, "datetime", FixedDateTime), patch.object(snapshots, "now_iso", return_value=STAMP), patch(
                            "kassiber.core.filed_report_chain.now_iso", return_value=STAMP
                        ), patch.object(snapshots.uuid, "uuid4", return_value=UUID(int=index + 1)):
                            if action["command"] == "reports.export-csv":
                                result = reports.export_csv_report(conn, None, None, root / action["target"], replace(cli._report_hooks(), now_iso=lambda: STAMP),
                                                                   wallet_ref=args.get("wallet"), history_limit=args.get("history_limit"))
                            elif action["command"] == "reports.export-transactions-csv":
                                result = reports.export_transactions_csv_report(conn, None, None, root / action["target"], cli._report_hooks())
                            elif action["command"] == "reports.export-austrian-e1kv-csv":
                                result = reports.export_austrian_e1kv_csv_bundle(conn, None, None, root / action["target"], cli._report_hooks(), tax_year=args["year"])
                            else:
                                result = snapshots.create_filed_report_snapshot(conn, workspace_id="ws", profile_id="p",
                                    report_state="filed", saved_snapshot_id=args["saved_snapshot_id"], authored_source="cli")
                        conn.commit()
                        expected = {"ok": result}
                    except Exception as exc:  # CLI's observable error projection
                        conn.rollback()
                        expected = {"error": dict(code=exc.code if isinstance(exc, AppError) else "internal_error",
                            message=str(exc), hint=getattr(exc, "hint", None), details=getattr(exc, "details", None),
                            retryable=getattr(exc, "retryable", False))}
                    action["expected"] = json.loads(json.dumps(expected).replace(root_text, "<root>"))
                    action["tables"] = dump(conn)
                    action["files"] = {str(p.relative_to(root)): p.read_bytes().decode("utf-8")
                                       for p in sorted(root.rglob("*.csv"))}
                conn.close()
                cases.append(dict(name=f"{country}-{algorithm}-{variant}", sql=sql, before=before, actions=actions))
    return json.dumps(cases, ensure_ascii=True, indent=2) + "\n"


def dump(conn):
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    return {t: sorted([dict(r) for r in conn.execute(f'SELECT * FROM "{t}"')], key=lambda r: json.dumps(r, sort_keys=True)) for t in tables}


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "export sequence vectors drifted"
    else:
        OUT.write_text(content)
