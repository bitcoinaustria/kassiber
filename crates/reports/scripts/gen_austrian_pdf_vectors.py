#!/usr/bin/env python3
"""Freeze Python's Austrian E 1kv PDF model for the shared report design.

``model`` holds exactly what ``export_austrian_e1kv_pdf_report`` hands to its
writer: the E 1kv report, the current portfolio rows, the tax year's
transaction rows and the report clock, with the profile settings the writer
reads. ``title`` is the envelope's title; refusals are kept as errors. The
fixtures are the Austrian workbook's.
"""
from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.cli.handlers import _report_hooks
from kassiber.core import reports
from kassiber.db import open_db
from kassiber.errors import AppError

OUT = ROOT / "crates/reports/tests/data/austrian_pdf.json"
STAMP = "2026-01-02T03:04:05Z"
PROFILE_KEYS = ("label", "gains_algorithm", "fiat_currency", "tax_country", "journal_input_version")


def austrian_model(conn, hooks, context, year):
    """The Austrian report's values for the shared design."""
    report = reports.report_austrian_e1kv(conn, None, None, hooks, tax_year=year, report_context=context)
    profile = context.profile
    return dict(
        report=report,
        portfolio=reports.report_portfolio_summary(conn, None, None, hooks, report_context=context),
        transactions=reports._austrian_e1kv_transaction_rows(conn, profile, report["tax_year"]),
        generated_at=hooks.now_iso(),
        workspace=context.workspace["label"],
        profile={key: profile[key] for key in PROFILE_KEYS},
    )


def build():
    cases = json.loads((ROOT / "crates/reports/tests/data/austrian_xlsx.json").read_text())
    result = []
    for case in cases:
        with tempfile.TemporaryDirectory(prefix="austrian-pdf-") as root:
            conn = open_db(Path(root) / "data")
            conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + case["sql"])
            args = json.loads(case["args_json"])
            hooks = replace(_report_hooks(), now_iso=lambda: STAMP)
            try:
                context = reports._require_report_context(conn, args.get("workspace"), args.get("profile"), hooks)
                model = json.loads(json.dumps(austrian_model(conn, hooks, context, args.get("year"))))
                title = f"Kassiber Steuerbericht {model['report']['tax_year']} Österreich"
                expected = dict(ok=dict(title=title, model=model))
            except AppError as e:
                expected = dict(error=dict(code=e.code, message=str(e), hint=e.hint, details=e.details,
                                           retryable=e.retryable))
            except (ValueError, TypeError, OverflowError, AttributeError) as e:
                expected = dict(error=dict(code="internal_error", message=str(e), hint=None, details=None,
                                           retryable=False))
            result.append(dict(name=case["name"], sql=case["sql"], args_json=case["args_json"], expected=expected))
            conn.close()
    return "[\n" + ",\n".join(json.dumps(case, ensure_ascii=True, separators=(",", ":")) for case in result) + "\n]\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "Austrian PDF vectors drifted"
    else:
        OUT.write_text(content)
