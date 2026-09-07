"""Fresh-process CLI accounting from explicit sources to verified close artifact."""
import base64
import copy
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from tests.test_accounting_integration import book  # noqa: F401

_REPO = Path(__file__).resolve().parents[1]


def cli(root, action, payload, *, expected_error=None):
    """One process per call; only synthetic passphrase uses a private inherited fd."""
    if os.name != "posix":
        pytest.skip("Private inherited-fd subprocess harness requires POSIX")
    read_fd, write_fd = os.pipe()
    try:
        os.write(write_fd, b"test-token-placeholder\n")
        os.close(write_fd)
        argv = [sys.executable, "-m", "kassiber", "--data-root", str(root), "--machine"]
        if action != "verify-package":
            argv += ["--db-passphrase-fd", str(read_fd)]
        argv += ["accounting", action]
        if action != "verify-package":
            argv += ["--workspace", "Test", "--profile", "Test book"]
        argv += ["--payload-stdin"]
        result = subprocess.run(argv, input=json.dumps(payload), text=True, capture_output=True,
            cwd=_REPO, pass_fds=(read_fd,), timeout=45)
    finally:
        os.close(read_fd)
    assert "test-token-placeholder" not in result.stdout + result.stderr
    response = json.loads(result.stdout)
    if expected_error:
        assert result.returncode != 0, response
        assert response["kind"] == "error", response
        assert response["error"]["code"] == expected_error, response
    else:
        assert result.returncode == 0, (result.stdout, result.stderr)
        assert response["kind"] == "accounting." + action
        assert response["schema_version"] == 1
    return response


def approve(root, task_id, step):
    preview = cli(root, "task-preview", {"task_id": task_id, "step": step})["data"]
    assert preview["ready"], preview["blockers"]
    payload = {"task_id": task_id, "step": step, "expected_digest": preview["expected_digest"],
        "expected_revision": preview["expected_revision"], "idempotency_key": "cli-" + step, "confirmed": True}
    if step.startswith("export_"):
        cli(root, "task-apply", payload, expected_error="accounting_task_consent_required")
        payload["confirm_plaintext"] = True
    applied = cli(root, "task-apply", payload)
    retried = cli(root, "task-apply", payload)
    assert applied["data"]["already_applied"] is False
    assert retried["data"]["already_applied"] is True
    assert retried["data"]["receipt"]["id"] == applied["data"]["receipt"]["id"]
    if step.startswith("export_"):
        assert retried["data"]["result"] == applied["data"]["result"]
    else:
        # Non-export retries return the durable identity receipt, not a second
        # full close snapshot. Export retries must reproduce actual bytes.
        assert retried["data"]["result"] == applied["data"]["receipt"]["result"]
    return preview, applied


def test_cli_prepare_post_close_export_and_retry_across_processes(book, tmp_path):
    conn, profile_id, root = book
    # Only initial encrypted book/chart/period are seeded by the shared fixture.
    # Source retention, rules and every workflow operation use the actual CLI.
    conn.commit()
    controls = {"format": "kassiber-bank-control-v1", "account_code": "bank", "statement_id": "cli-bank",
        "start_date": "2025-01-01", "end_date": "2025-12-31", "opening_minor": 0, "closing_minor": 300,
        "currency": "EUR", "minor_unit_exponent": 2}
    evidence = cli(root, "evidence-add", {"content_base64": base64.b64encode(json.dumps(controls).encode()).decode(),
        "name": "Synthetic reviewed bank controls", "media_type": "application/json"})["data"]
    statement = cli(root, "bank-import", {"account_code": "bank", "statement_id": "cli-bank", "start_date": "2025-01-01",
        "end_date": "2025-12-31", "opening_minor": "0", "closing_minor": "300", "control_evidence_id": evidence["id"],
        "control_review_reason": "Reviewed synthetic opening and closing controls", "control_locator": "First line",
        "csv_text": "row_id,date,amount_minor,description\na,2025-02-03,100,Membership\nb,2025-02-04,100,Membership\nc,2025-02-05,100,Membership\n"})["data"]
    task = cli(root, "task-create", {"period_id": "2025", "statement_ids": [statement["id"]], "idempotency_key": "cli-task"})["data"]
    assert task["source_count"] == 3 and len(task["exceptions"]) == 3
    assert cli(root, "task-get", {"task_id": task["id"]})["data"]["id"] == task["id"]
    assert cli(root, "task-list", {})["data"]["tasks"][0]["id"] == task["id"]
    cli(root, "rule-create", {"idempotency_key": "cli-rule", "account_code": "bank", "direction": "in",
        "description_exact": "Membership", "counter_account_code": "sales", "reason": "Reviewed synthetic receipts", "confirmed": True})
    preview = cli(root, "task-preview", {"task_id": task["id"], "step": "prepare"})["data"]
    denied = {"task_id": task["id"], "step": "prepare", "expected_digest": preview["expected_digest"],
        "expected_revision": preview["expected_revision"], "idempotency_key": "cli-denied", "confirmed": False}
    cli(root, "task-apply", denied, expected_error="accounting_task_consent_required")
    assert cli(root, "journal", {"period_id": "2025"})["data"]["entries"] == []
    prepared, _ = approve(root, task["id"], "prepare")
    assert len(prepared["proposals"]) == 3
    posted, _ = approve(root, task["id"], "post")
    assert len(posted["detail"]["entries"]) == 3
    state = cli(root, "task-get", {"task_id": task["id"]})["data"]
    assert all(row["status"] == "posted" for row in state["coverage"])
    assert len(cli(root, "journal", {"period_id": "2025"})["data"]["entries"]) == 3
    assert cli(root, "bank-reconcile", {"statement_id": statement["id"]})["data"]["reconciled"] is True
    closed, _ = approve(root, task["id"], "close")
    assert closed["detail"]["trial_balance"]["debit_minor"] == "300"
    _, exported = approve(root, task["id"], "export_close")
    artifact = exported["data"]["result"]
    assert artifact["profile_id"] == profile_id
    assert artifact["artifact_state"] == "prepared"
    assert "snapshot_json" not in exported["data"]["receipt"]["result"]
    assert exported["data"]["task"]["state"] == "completed"
    assert len(exported["data"]["task"]["receipts"]) == 4
    # Explicit local artifact retention, followed by verification without a DB.
    saved = tmp_path / "synthetic-close.json"
    saved.write_text(json.dumps(exported), encoding="utf-8")
    no_book = tmp_path / "verifier-does-not-open-a-book"
    verified = cli(no_book, "verify-package", json.loads(saved.read_text()))["data"]["verification"]
    assert verified["ledger_arithmetic"] == "verified"
    assert verified["entries_checked"] == 3 and verified["lines_checked"] == 6
    # CLI settings bootstrap may create its config directory, but verification
    # must neither require an unlock nor create an accounting database.
    from kassiber.db import resolve_database_path
    assert not resolve_database_path(no_book).exists()
    # Both legacy explicit export envelopes and raw packages still round-trip.
    cli(no_book, "verify-package", {"kind": "accounting.export-close", "schema_version": 1, "data": artifact})
    cli(no_book, "verify-package", artifact)
    for mutation in ("step", "format", "schema", "state", "checksum"):
        invalid = copy.deepcopy(exported)
        if mutation == "step":
            invalid["data"]["receipt"]["step"] = "export_tax"
        elif mutation == "format":
            invalid["data"]["result"]["format"] = "unrelated"
        elif mutation == "schema":
            invalid["schema_version"] = True
        elif mutation == "state":
            invalid["data"]["result"]["artifact_state"] = "saved"
        else:
            invalid["data"]["result"]["snapshot_json"] += " "
        cli(no_book, "verify-package", invalid, expected_error="accounting_package_invalid")


@pytest.mark.parametrize("action,expected", [("task-preview", "expected"), ("task-apply", "confirm_plaintext"),
    ("workbench", "period_id"), ("verify-package", "export_close")])
def test_accounting_task_help(action, expected):
    result = subprocess.run([sys.executable, "-m", "kassiber", "accounting", action, "--help"],
        capture_output=True, text=True, cwd=_REPO, timeout=30)
    assert result.returncode == 0
    assert expected in result.stdout


def test_cli_empty_book_reviewed_k2_task_finalize_and_export(book, tmp_path):
    # Reuse the established synthetic full-review fixture; do not infer legal
    # facts from bank rows, invent tax policy, or claim an actual filing.
    from kassiber.core.accounting.jurisdiction import AT_PACK_ID
    from tests.test_accounting_tax_workpapers import complete_patch

    conn, _, root = book
    conn.commit()
    paper = cli(root, "tax-create", {"period_id": "2025", "pack_id": AT_PACK_ID, "idempotency_key": "cli-tax"})["data"]
    cli(root, "tax-review", {"workpaper_id": paper["id"], "expected_revision": 1, "patch": complete_patch(),
        "reason": "Established synthetic complete review fixture", "idempotency_key": "cli-tax-reviewed"})
    task = cli(root, "task-create", {"period_id": "2025", "statement_ids": [], "tax_workpaper_id": paper["id"],
        "idempotency_key": "cli-tax-task"})["data"]
    approve(root, task["id"], "close")
    preview, finalized = approve(root, task["id"], "tax_finalize")
    assert preview["detail"]["forms"][0]["form_id"] == "K2"
    assert finalized["data"]["receipt"]["step"] == "tax_finalize"
    _, exported = approve(root, task["id"], "export_tax")
    artifact = exported["data"]["result"]
    assert artifact["stale"] is False
    assert artifact["report"]["filed"] is False
    assert artifact["verification_levels"]["tax_liability_certified"] is False
    assert "<table>" in artifact["html"]
    assert exported["data"]["task"]["state"] == "completed"
    assert len(exported["data"]["task"]["receipts"]) == 3
    cli(tmp_path / "no-book", "verify-package", exported, expected_error="accounting_package_invalid")
