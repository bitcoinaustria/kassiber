#!/usr/bin/env python3
"""Freeze Python's source-of-funds PDF model for the shared report design.

Each case saves a source-of-funds case with Python (fixed identities and
clock), keeps the saved case row as SQL, then runs Python's ``export_pdf``
with its writer replaced by a recorder: ``model`` is exactly the report,
snapshot hash and clock the writer would receive, and refusals are kept as
errors. Rust replays ``book.sql``, ``base_sql`` and each case's SQL (the
saved row included) and must reproduce both.

    uv run --locked python -m crates.reports.scripts.gen_source_funds_pdf_vectors [--check]
"""
import json
import sqlite3
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

from kassiber import daemon
from kassiber.core import source_funds as sof
from kassiber.db import open_db
from kassiber.errors import AppError
from crates.reports.scripts.gen_source_funds_cases import ROOT, STAMP
from crates.reports.scripts.gen_source_funds_reads import BASE

OUT = ROOT / "crates/reports/tests/data/source_funds_pdf.json"
BOOK = (ROOT / "crates/reports/tests/data/book.sql").read_text() + BASE
READY = "DELETE FROM source_funds_links WHERE id='l-z';UPDATE source_funds_links SET from_allocation_amount=allocation_amount WHERE id='l-a';"
AUSTRIA = "UPDATE profiles SET tax_country='at' WHERE id='p';"


def save(conn, args):
    """Save one case through Python's report builder; its row as SQL."""
    ids = iter(["case-1", "snapshot-1"])
    with patch.object(sof.uuid, "uuid4", lambda: next(ids)), patch.object(sof, "_now", lambda: STAMP):
        sof.build_report(
            conn, None, None, daemon._source_funds_hooks(),
            target_transaction_ref="in", target_amount=args.get("target_amount"),
            report_purpose=args.get("purpose") or "existing_transaction",
            planned_destination=args.get("planned_destination"), planned_note=args.get("planned_note"),
            reveal_mode=args.get("reveal_mode"), max_depth=8, recipient_ref=args.get("recipient"),
            include_diagrams=True, save_case=True, case_label="Case Ω",
            report_options=args.get("options") or {})
    row = conn.execute("SELECT * FROM source_funds_cases WHERE id='case-1'").fetchone()
    columns = list(row.keys())
    values = ",".join("NULL" if row[c] is None else str(row[c]) if isinstance(row[c], int)
                      else "'" + str(row[c]).replace("'", "''") + "'" for c in columns)
    return f"DELETE FROM source_funds_snapshots;DELETE FROM source_funds_cases WHERE id='case-1';" \
           f"INSERT INTO source_funds_cases({','.join(columns)}) VALUES({values});"


def export(conn, case):
    captured = {}

    def writer(path, *, report, generated_at, snapshot_hash):
        captured.update(report=report, snapshot_hash=snapshot_hash, generated_at=generated_at)
        return {"file": str(path), "pages": 1, "bytes": 0, "title": "", "renderer": "reportlab"}

    with patch.object(sof, "write_source_funds_pdf", writer), patch.object(sof, "_now", lambda: STAMP):
        try:
            sof.export_pdf(conn, None, None, "/unused.pdf", daemon._source_funds_hooks(), case_ref=case)
            return dict(ok=dict(model=json.loads(json.dumps(captured))))
        except AppError as e:
            return dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details, retryable=e.retryable))
        except (ValueError, TypeError, KeyError) as e:
            return dict(error=dict(code="internal_error", message=str(e), hint=None, details=None, retryable=False))


def specs():
    saved = [
        ("standard", "", {}),
        ("minimal-recipient", "", dict(reveal_mode="minimal", recipient="r-a")),
        ("full", "", dict(reveal_mode="full")),
        ("labels-only", "", dict(reveal_mode="labels_only")),
        ("planned-sats-masked", "", dict(purpose="planned_exchange_sale", planned_destination=" Exchange Ω ",
                                         planned_note=" note ", recipient="r-a", target_amount="0.1",
                                         options=dict(diagram_detail="detailed", amount_precision="sats",
                                                      mask_recipient=True, omit_sections=["flow_links"]))),
        ("omit-all", "", dict(options=dict(omit_sections=["flow_levels", "transaction_details", "flow_links",
                                                          "graph_nodes"]))),
        ("austria", AUSTRIA, {}),
        ("blocked", "", {}),
    ]
    out = [(name, sql, args, "case-1") for name, sql, args in saved]
    out += [("no-case", "", None, None), ("empty-case", "", None, ""), ("unknown-case", "", None, "missing"),
            ("broken-json", "", None, "c-a"), ("hash-mismatch", "", None, "c-z"), ("other-book", "", None, "c-other")]
    return out


def build():
    result = []
    with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
        template = open_db(Path(directory) / "data")
        template.executescript(BOOK)
        template.commit()
        for name, sql, args, case in specs():
            conn = sqlite3.connect(":memory:")
            conn.row_factory = sqlite3.Row
            template.backup(conn)
            setup = sql + ("" if name == "blocked" else READY)
            conn.executescript(setup)
            conn.commit()
            if args is not None:
                setup += save(conn, args)
                conn.commit()
            result.append(dict(name=name, sql=setup, args_json=json.dumps({"case": case}),
                               expected=export(conn, case)))
            conn.close()
        template.close()
    return json.dumps(dict(base_sql=BASE, cases=result), ensure_ascii=True, indent=1) + "\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "source-of-funds PDF vectors drifted"
    else:
        OUT.write_text(content)
