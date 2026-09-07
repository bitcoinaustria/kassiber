"""Real database/transport/export privacy and exactness contracts."""

import argparse
import hashlib
import io
import json
from unittest.mock import patch

import pytest

from kassiber.backup.pack import _backup_sqlcipher_database
from kassiber.cli.accounting import dispatch_accounting, MAX_PAYLOAD_CHARS
from kassiber.core.accounting import evidence, ledger
from kassiber.core.accounting.commands import execute
from kassiber.core.accounting.package import export_close, verify_package
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.maintenance import reset_current_profile_data
from kassiber.core.repo import resolve_scope
from kassiber.core.sync_replication.schema_allowlist import NEVER_SYNC_TABLES, SYNC_TABLE_MAP, validate_wire_row
from kassiber.daemon_accounting import ACCOUNTING_UI_KINDS, dispatch_accounting_ui
from kassiber.db import open_db, resolve_database_path
from kassiber.diagnostics import collect_public_diagnostics
from kassiber.errors import AppError
from kassiber.secrets.sqlcipher import get_row_class, open_encrypted


@pytest.fixture
def book(tmp_path):
    pytest.importorskip("sqlcipher3")
    root = tmp_path / "data"
    conn = open_db(root, passphrase="test-token-placeholder")
    workspace = create_workspace(conn, "Test")
    profile = create_profile(conn, workspace["id"], "Test book", "EUR", "FIFO", "generic", 365)
    scope = profile["id"]
    ledger.configure_book(conn, scope, currency="EUR", timezone="Europe/Vienna")
    for code, kind in (("bank", "asset"), ("capital", "equity"), ("sales", "income")):
        ledger.create_account(conn, scope, code=code, name=code, kind=kind)
    ledger.create_period(conn, scope, period_id="2025", start_date="2025-01-01", end_date="2025-12-31")
    conn.commit()
    yield conn, scope, root
    conn.close()


def post(conn, scope, amount="9007199254740997"):
    draft = execute(conn, scope, "draft", {
        "idempotency_key": "example", "period_id": "2025", "entry_date": "2025-02-03",
        "description": "synthetic-private-ledger-marker",
        "lines": [{"account_code": "bank", "debit_minor": amount}, {"account_code": "sales", "credit_minor": amount}],
    })
    return execute(conn, scope, "post", {"draft_id": draft["id"], "expected_digest": draft["payload_digest"]})


def close(conn, scope):
    return ledger.close_period(conn, scope, period_id="2025", expected_revision=ledger.book_status(conn, scope)["revision"])


def test_real_bootstrap_preserves_private_opt_out(tmp_path):
    conn = open_db(tmp_path / "private")
    workspace = create_workspace(conn, "Private")
    profile = create_profile(conn, workspace["id"], "Private", "EUR", "FIFO", "generic", 365)
    assert ledger.snapshot(conn, profile["id"])["configured"] is False
    with pytest.raises(AppError) as exc:
        ledger.configure_book(conn, profile["id"], currency="EUR", timezone="Europe/Vienna")
    assert exc.value.code == "accounting_requires_encryption"
    assert conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
    conn.close()


def test_transport_is_exact_and_rejects_changed_book(book):
    conn, scope, _ = book
    result = post(conn, scope)
    assert result["lines"][0]["debit_minor"] == "9007199254740997"
    assert isinstance(result["lines"][0]["debit_minor"], str)
    with pytest.raises(AppError) as exc:
        dispatch_accounting_ui(conn, kind="ui.accounting.account_create", args={"profile_id": "other", "payload": {"code": "bad", "name": "Bad", "kind": "asset"}})
    assert exc.value.code == "accounting_scope_changed"
    assert "ui.accounting.post" in ACCOUNTING_UI_KINDS


def test_cli_bounded_stdin_and_scope(book):
    conn, scope, _ = book
    args = argparse.Namespace(payload_fd=None, payload_stdin=True, payload=None, workspace="Test", profile="Test book", accounting_command="reports")
    with patch("sys.stdin", io.StringIO('{"period_id":"2025"}')):
        result = dispatch_accounting(conn, args)
    assert result["profile_id"] == scope
    assert result["trial_balance"]["debit_minor"] == "0"
    with patch("sys.stdin", io.StringIO("x" * (MAX_PAYLOAD_CHARS + 1))), pytest.raises(AppError) as exc:
        dispatch_accounting(conn, args)
    assert exc.value.code == "accounting_payload_too_large"


def test_unsafe_json_numeric_amounts_rejected_before_rounding_can_hide(book):
    conn, scope, _ = book
    with pytest.raises(AppError) as exc:
        post(conn, scope, 2**53)
    assert exc.value.code == "accounting_invalid_amount"


@pytest.mark.parametrize("suffix", ["minor", "atomic", "msat"])
def test_exact_wire_units_cover_atomic_quantities(suffix):
    from kassiber.core.accounting.commands import _minor_values, wire_values
    key = "quantity_" + suffix
    assert wire_values({key: 9007199254740997}) == {key: "9007199254740997"}
    assert _minor_values({key: "9007199254740997"}) == {key: 9007199254740997}
    for value in (9007199254740997, "1e3", "-0", "01"):
        with pytest.raises(AppError):
            _minor_values({key: value})


def test_large_file_payload_is_digest_bound(book, tmp_path):
    import base64

    conn, scope, _ = book
    payload = json.dumps({"content_base64": base64.b64encode(b"synthetic " * 2000).decode(), "name": "local source", "media_type": "text/plain"}).encode()
    source = tmp_path / "source.json"
    source.write_bytes(payload)
    args = argparse.Namespace(accounting_command="evidence-add", workspace="Test", profile="Test book", payload_fd=None, payload_stdin=False, payload=None, payload_file=str(source), payload_sha256=hashlib.sha256(payload).hexdigest())
    result = dispatch_accounting(conn, args)
    assert result["profile_id"] == scope
    assert result["byte_length"] == 20000
    source.write_bytes(payload + b" ")
    with pytest.raises(AppError) as exc:
        dispatch_accounting(conn, args)
    assert exc.value.code == "accounting_stale_approval"


def test_verifier_rejects_float_and_bool_report_amounts(book):
    conn, scope, _ = book
    post(conn, scope, "10000")
    package = export_close(conn, scope, close_id=close(conn, scope)["id"])
    for value in (10000.0, True):
        state = json.loads(package["snapshot_json"])
        state["trial_balance"]["rows"][0]["debit_minor"] = value
        source = json.dumps(state, sort_keys=True, separators=(",", ":"))
        with pytest.raises(AppError):
            verify_package(dict(package, snapshot_json=source, snapshot_digest=hashlib.sha256(source.encode()).hexdigest()))


def test_retained_document_sync_updates_without_replacing_evidence(book):
    from kassiber.core.sync_replication.merge import _insert_or_update_with_collision_notice

    conn, scope, _ = book
    workspace = conn.execute("SELECT workspace_id FROM profiles WHERE id=?", (scope,)).fetchone()[0]
    conn.execute("""INSERT INTO external_documents
        (id,workspace_id,profile_id,document_type,label,created_at,updated_at)
        VALUES ('retained-source',?,?, 'invoice','Original','2025-01-01','2025-01-01')""", (workspace, scope))
    retained = evidence.retain_evidence(conn, scope, content=b"retained original",
        media_type="text/plain", name="Original", source_document_id="retained-source")
    actual = dict(conn.execute("SELECT * FROM external_documents WHERE id='retained-source'").fetchone())
    actual["label"] = "Reviewed label"
    _insert_or_update_with_collision_notice(conn, book={"profile_id": scope},
        spec=SYNC_TABLE_MAP["external_documents"], actual=actual, event={"id": "synthetic-sync-event"})
    assert conn.execute("SELECT label FROM external_documents WHERE id='retained-source'").fetchone()[0] == "Reviewed label"
    assert evidence.read_evidence_bytes(conn, scope, retained["id"]) == b"retained original"
    with pytest.raises(AppError) as exc:
        _insert_or_update_with_collision_notice(conn, book={"profile_id": scope},
            spec=SYNC_TABLE_MAP["external_documents"], actual=dict(actual, label=None), event={"id": "synthetic-bad-event"})
    assert exc.value.code == "sync_row_constraint"
    with pytest.raises(Exception, match="accounting_evidence_retained"):
        conn.execute("INSERT OR REPLACE INTO external_documents SELECT * FROM external_documents WHERE id='retained-source'")


def test_overflowing_wire_amount_rejected_by_shared_ledger_guard(book):
    conn, scope, _ = book
    with pytest.raises(AppError):
        post(conn, scope, "9300000000000000000")
    assert conn.execute("SELECT COUNT(*) FROM gl_entries WHERE profile_id=?", (scope,)).fetchone()[0] == 0


@pytest.mark.parametrize("target", ["self", "missing"])
def test_verifier_rejects_reversal_chain_or_missing_target(book, target):
    conn, scope, _ = book
    post(conn, scope, "10000")
    package = export_close(conn, scope, close_id=close(conn, scope)["id"])
    state = json.loads(package["snapshot_json"])
    entry = state["journal"][0]
    entry["entry_kind"] = "reversal"
    entry["reversal_of"] = entry["id"] if target == "self" else "missing"
    committed = {key: entry[key] for key in ("idempotency_key", "period_id", "entry_date", "description", "source_ref", "entry_kind", "reversal_of")}
    committed["lines"] = [{key: line[key] for key in ("account_code", "account_name", "account_kind", "debit_minor", "credit_minor")} for line in entry["lines"]]
    entry["payload_digest"] = ledger.digest(committed)
    for row in state["entries"]:
        if row["id"] == entry["id"]:
            row["payload_digest"] = entry["payload_digest"]
    source = ledger.canonical_json(state)
    with pytest.raises(AppError):
        verify_package(dict(package, snapshot_json=source, snapshot_digest=hashlib.sha256(source.encode()).hexdigest()))


def test_verifier_binds_report_currency_year_and_scale(book):
    conn, scope, _ = book
    post(conn, scope, "10000")
    package = export_close(conn, scope, close_id=close(conn, scope)["id"])
    for section in ("trial_balance", "statements"):
        for key, value in (("currency", "USD"), ("minor_unit_exponent", 0), ("profile_id", "other"), ("period_id", "2024"), ("revision", -1)):
            state = json.loads(package["snapshot_json"])
            state[section][key] = value
            source = ledger.canonical_json(state)
            with pytest.raises(AppError):
                verify_package(dict(package, snapshot_json=source, snapshot_digest=hashlib.sha256(source.encode()).hexdigest()))
    state = json.loads(package["snapshot_json"])
    entry = state["journal"][0]
    entry["period_id"] = "2024"
    committed = {key: entry[key] for key in ("idempotency_key", "period_id", "entry_date", "description", "source_ref", "entry_kind", "reversal_of")}
    committed["lines"] = [{key: line[key] for key in ("account_code", "account_name", "account_kind", "debit_minor", "credit_minor")} for line in entry["lines"]]
    entry["payload_digest"] = ledger.digest(committed)
    state["entries"] = []
    state["trial_balance"].update(rows=[], debit_minor=0, credit_minor=0)
    state["statements"].update(profit_and_loss=[], profit_minor=0)
    source = ledger.canonical_json(state)
    with pytest.raises(AppError):
        verify_package(dict(package, snapshot_json=source, snapshot_digest=hashlib.sha256(source.encode()).hexdigest()))


def test_snapshot_package_checks_arithmetic_and_is_immutable(book):
    conn, scope, _ = book
    post(conn, scope, "10000")
    saved = close(conn, scope)
    package = export_close(conn, scope, close_id=saved["id"])
    assert verify_package(package)["ledger_arithmetic"] == "verified"
    ledger.reopen_period(conn, scope, period_id="2025", reason="Correction", expected_revision=ledger.book_status(conn, scope)["revision"])
    assert export_close(conn, scope, close_id=saved["id"])["snapshot_json"] == package["snapshot_json"]
    tampered = dict(package, snapshot_digest="0" * 64)
    with pytest.raises(AppError):
        verify_package(tampered)
    state = json.loads(package["snapshot_json"])
    state["journal"][0]["lines"][0]["debit_minor"] += 1
    changed = json.dumps(state, sort_keys=True, separators=(",", ":"))
    tampered = dict(package, snapshot_json=changed, snapshot_digest=hashlib.sha256(changed.encode()).hexdigest())
    with pytest.raises(AppError):
        verify_package(tampered)


def test_export_requires_explicit_plaintext_ack(book):
    conn, scope, _ = book
    saved = close(conn, scope)
    with pytest.raises(AppError) as exc:
        execute(conn, scope, "export-close", {"close_id": saved["id"], "confirm_plaintext": False})
    assert exc.value.code == "accounting_export_consent_required"


def test_close_cannot_commit_an_unexportable_oversized_snapshot(book, monkeypatch):
    from kassiber.core.accounting import package

    conn, scope, _ = book
    post(conn, scope, "10000")
    monkeypatch.setattr(package, "MAX_SNAPSHOT_BYTES", 100)
    with pytest.raises(AppError) as exc:
        close(conn, scope)
    assert exc.value.code == "accounting_close_too_large"
    assert ledger.snapshot(conn, scope)["periods"][0]["state"] == "open"
    assert conn.execute("SELECT COUNT(*) FROM gl_period_events").fetchone()[0] == 0


def test_verifier_rejects_invalid_unicode_as_typed_error():
    from kassiber.core.accounting.package import FORMAT

    with pytest.raises(AppError) as exc:
        verify_package({"format": FORMAT, "snapshot_json": "\ud800", "snapshot_digest": ""})
    assert exc.value.code == "accounting_package_invalid"


def test_new_tables_never_sync_and_ai_not_implicitly_enabled(book):
    from kassiber.ai import tools

    conn, _, _ = book
    tables = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name GLOB 'gl_*'")}
    assert tables <= NEVER_SYNC_TABLES
    assert not tables & set(SYNC_TABLE_MAP)
    for table in tables:
        with pytest.raises(AppError) as exc:
            validate_wire_row(table, {})
        assert exc.value.code == "sync_schema_forbidden"
    # Only explicitly reviewed, opaque task tools enter ordinary AI.
    allowed = {"ui.accounting.task_get", "ui.accounting.task_preview",
               "ui.accounting.task_apply", "ui.accounting.task_cancel"}
    exposed = {kind for kind in ACCOUNTING_UI_KINDS if tools.get_tool(kind) is not None}
    assert exposed == allowed


def test_diagnostics_do_not_disclose_payload_or_evidence(book):
    conn, scope, root = book
    marker = "secret-bookkeeping-document-483726"
    evidence.retain_evidence(conn, scope, content=marker.encode(), name=marker, media_type="text/plain")
    args = argparse.Namespace(command="accounting", accounting_command="draft", data_root=str(root), payload=json.dumps({"description": marker}))
    assert marker not in json.dumps(collect_public_diagnostics(conn, args))


def test_default_audit_summary_does_not_include_gl_evidence(book):
    from kassiber.core.audit_package import AuditPackageHooks, build_evidence_summary

    conn, scope, root = book
    marker = "private-gl-document-omitted-from-default-audit-623"
    evidence.retain_evidence(conn, scope, content=marker.encode(), name=marker, media_type="text/plain")
    hooks = AuditPackageHooks(resolve_scope=resolve_scope, resolve_transaction=lambda *args: None, now_iso=lambda: "2025-12-31T23:59:00Z")
    summary = build_evidence_summary(conn, str(root), "Test", "Test book", hooks)
    assert marker not in json.dumps(summary)


def test_package_verifier_cli_needs_no_database(book):
    conn, scope, _ = book
    saved = close(conn, scope)
    package = export_close(conn, scope, close_id=saved["id"])
    args = argparse.Namespace(accounting_command="verify-package", payload_fd=None, payload_stdin=True, payload=None)
    with patch("sys.stdin", io.StringIO(json.dumps(package))):
        assert dispatch_accounting(None, args)["verification"]["ledger_arithmetic"] == "verified"


def test_large_real_close_export_roundtrips_through_cli_verifier(book):
    conn, scope, _ = book
    for index in range(4):
        draft = ledger.create_draft(conn, scope, {
            "idempotency_key": f"large-{index}", "period_id": "2025", "entry_date": "2025-03-01",
            "description": "Synthetic large journal",
            "lines": [{"account_code": "bank", "debit_minor": 1}, {"account_code": "sales", "credit_minor": 1}] * 500,
        })
        ledger.post_draft(conn, scope, draft_id=draft["id"], expected_digest=draft["payload_digest"])
    package = export_close(conn, scope, close_id=close(conn, scope)["id"])
    exported = json.dumps({"kind": "accounting.export-close", "schema_version": 1, "data": package})
    assert len(exported) > 1_000_000
    args = argparse.Namespace(accounting_command="verify-package", payload_fd=None, payload_stdin=True, payload=None)
    with patch("sys.stdin", io.StringIO(exported)):
        assert dispatch_accounting(None, args)["verification"]["lines_checked"] == 4000


def test_journal_pages_cover_ties_and_refuse_changed_scope_or_revision(book):
    conn, scope, _ = book
    ids = set()
    for index in range(5):
        draft = execute(conn, scope, "draft", {
            "idempotency_key": f"paged-{index}", "period_id": "2025", "entry_date": "2025-03-01",
            "description": "Same date",
            "lines": [{"account_code": "bank", "debit_minor": "1"}, {"account_code": "sales", "credit_minor": "1"}],
        })
        ids.add(execute(conn, scope, "post", {"draft_id": draft["id"], "expected_digest": draft["payload_digest"]})["id"])
    payload = {"period_id": "2025", "limit": 2}
    first = execute(conn, scope, "journal", payload)
    assert len(first["entries"]) == 2 and first["next_cursor"]
    entries = first["entries"][:]
    cursor = first["next_cursor"]
    while cursor:
        page = execute(conn, scope, "journal", dict(payload, cursor=cursor))
        entries.extend(page["entries"])
        cursor = page["next_cursor"]
    assert len(entries) == 5 and {row["id"] for row in entries} == ids
    for altered in ({"status": "draft"}, {"period_id": None}):
        with pytest.raises(AppError) as exc:
            execute(conn, scope, "journal", dict(payload, cursor=first["next_cursor"], **altered))
        assert exc.value.code == "accounting_stale_cursor"
    ledger.create_account(conn, scope, code="new", name="New", kind="expense")
    with pytest.raises(AppError) as exc:
        execute(conn, scope, "journal", dict(payload, cursor=first["next_cursor"]))
    assert exc.value.code == "accounting_stale_cursor"


def test_bank_evidence_partial_matching_and_close_through_desktop_dispatch(book):
    import base64

    conn, scope, _ = book
    def call(action, payload):
        return dispatch_accounting_ui(conn, kind=f"ui.accounting.{action}", args={"profile_id": scope, "payload": payload})

    receipt = post(conn, scope, "10000")
    retained = call("evidence_add", {"content_base64": base64.b64encode(b"%PDF-1.4 synthetic reviewed statement: opening 0, closing 100.00 EUR").decode(), "name": "Statement", "media_type": "application/pdf"})
    csv_text = "row_id,date,amount_minor,description\na,2025-02-03,4000,Receipt part 1\nb,2025-02-03,6000,Receipt part 2\n"
    imported = call("bank_import", {"csv_text": csv_text, "account_code": "bank", "statement_id": "february", "start_date": "2025-02-01", "end_date": "2025-02-28", "opening_minor": "0", "closing_minor": "10000", "control_evidence_id": retained["id"], "control_review_reason": "Reviewed synthetic opening and closing balances", "control_locator": "Page 1, opening and closing controls"})
    with pytest.raises(AppError):
        call("close", {"period_id": "2025", "expected_revision": ledger.book_status(conn, scope)["revision"]})
    result = call("bank_reconcile", {"statement_id": imported["id"]})
    for row in result["rows"]:
        call("bank_allocate", {"row_id": row["id"], "line_id": receipt["lines"][0]["id"], "amount_minor": row["amount_minor"], "idempotency_key": row["id"]})
    assert call("bank_reconcile", {"statement_id": imported["id"]})["reconciled"]
    saved = call("close", {"period_id": "2025", "expected_revision": ledger.book_status(conn, scope)["revision"]})
    assert call("export_close", {"close_id": saved["id"], "confirm_plaintext": True})["verification"]["ledger_arithmetic"] == "verified"


@pytest.mark.parametrize('action,collection', [('journal', 'entries'), ('evidence_list', 'evidence'),
                                             ('bank_list', 'statements'), ('item_list', 'items'),
                                             ('schedule_list', 'schedules')])
def test_desktop_journal_paging_contract_rejects_duplicate_or_unrelated_cursor(book, action, collection):
    conn, scope, _ = book
    kind = f'ui.accounting.{action}'
    result = dispatch_accounting_ui(conn, kind=kind, args={"profile_id": scope, "payload": {}, "cursor": None})
    assert result[collection] == [] and result["next_cursor"] is None
    for kind, payload in ((kind, {"cursor": None}), ("ui.accounting.snapshot", {})):
        with pytest.raises(AppError) as exc:
            dispatch_accounting_ui(conn, kind=kind, args={"profile_id": scope, "payload": payload, "cursor": None})
        assert exc.value.code == "accounting_invalid_fields"


def test_retained_csv_input_stays_local_and_refuses_ambiguous_sources(book):
    import base64

    conn, scope, _ = book
    raw = b"row_id,date,amount_minor,description\na,2025-02-03,4000,Receipt\n"
    retained = execute(conn, scope, "evidence-add", {"content_base64": base64.b64encode(raw).decode(), "name": "Bank CSV", "media_type": "text/csv"})
    args = {"csv_evidence_id": retained["id"], "start_date": "2025-02-01", "end_date": "2025-02-28", "opening_minor": "0", "closing_minor": "4000"}
    assert execute(conn, scope, "bank-preview", args)["movement_minor"] == "4000"
    assert execute(conn, scope, "bank-import", dict(args, account_code="bank", statement_id="retained", evidence_id=retained["id"]))["movement_minor"] == "4000"
    with pytest.raises(AppError):
        execute(conn, scope, "bank-preview", dict(args, csv_text=raw.decode()))
    ledger.create_account(conn, scope, code="bank2", name="Bank 2", kind="asset")
    with pytest.raises(AppError):
        execute(conn, scope, "bank-import", dict(args, account_code="bank2", statement_id="duplicate"))
    other = execute(conn, scope, "evidence-add", {"content_base64": base64.b64encode(b"Different statement").decode(), "name": "Other", "media_type": "text/plain"})
    with pytest.raises(AppError) as exc:
        execute(conn, scope, "bank-import", dict(args, account_code="bank2", statement_id="forged-source", evidence_id=other["id"]))
    assert exc.value.code == "accounting_evidence_scope"


def test_operator_brokers_accounting_stdin_as_private_payload():
    from kassiber.operator.client import prepare_arguments, wipe_prepared

    source = b'{"description":"private book text"}'
    prepared = prepare_arguments(["accounting", "draft", "--workspace", "Test", "--profile", "Test book", "--payload-stdin"], stdin=io.BytesIO(source))
    try:
        assert "--payload-fd" in prepared.argv
        assert b"private book text" not in json.dumps(prepared.argv).encode()
        assert list(prepared.secrets.values()) == [bytearray(source)]
    finally:
        wipe_prepared(prepared)


def test_large_evidence_roundtrips_real_locked_daemon_with_bounded_messages(book):
    import base64
    from kassiber.daemon import MAX_REQUEST_LINE_CHARS
    from tests.test_daemon_smoke import _start_daemon, _read_until_kind, _read_payload, _write_payload, _close_daemon

    conn, scope, root = book
    conn.commit()
    proc = _start_daemon(root, egress_mode="strict")
    counter = 0
    try:
        ready = _read_until_kind(proc, "daemon.ready")
        assert set(ACCOUNTING_UI_KINDS) <= set(ready["data"]["supported_kinds"])
        def call(kind, args=None):
            nonlocal counter
            counter += 1
            request = {"request_id": f"accounting-{counter}", "kind": kind, "args": args or {}}
            assert len(json.dumps(request)) < MAX_REQUEST_LINE_CHARS
            _write_payload(proc, request)
            while True:
                response = _read_payload(proc)
                if response.get("request_id") == request["request_id"]:
                    return response

        def accounting(action, payload):
            return call(f"ui.accounting.{action}", {"profile_id": scope, "payload": payload})

        locked = accounting("snapshot", {})
        assert locked["kind"] == "auth_required", locked
        assert call("daemon.unlock", {"auth_response": {"passphrase_secret": "test-token-placeholder"}})["kind"] == "daemon.unlock"
        content = b"Synthetic confidential evidence payload\n" * 40_000
        assert len(base64.b64encode(content)) > MAX_REQUEST_LINE_CHARS
        started = accounting("evidence_upload_begin", {"name": "Large source", "media_type": "text/plain", "total_bytes": len(content), "content_sha256": hashlib.sha256(content).hexdigest(), "idempotency_key": "real-daemon-upload"})
        assert started["kind"] == "ui.accounting.evidence_upload_begin"
        upload_id = started["data"]["upload_id"]
        for offset in range(0, len(content), 256 * 1024):
            chunk = content[offset:offset + 256 * 1024]
            result = accounting("evidence_upload_append", {"upload_id": upload_id, "offset": offset, "content_base64": base64.b64encode(chunk).decode(), "chunk_sha256": hashlib.sha256(chunk).hexdigest()})
            assert result["kind"] == "ui.accounting.evidence_upload_append"
        call("daemon.lock")
        assert accounting("evidence_upload_finish", {"upload_id": upload_id})["kind"] == "auth_required"
        assert call("daemon.unlock", {"auth_response": {"passphrase_secret": "test-token-placeholder"}})["kind"] == "daemon.unlock"
        finished = accounting("evidence_upload_finish", {"upload_id": upload_id})
        assert finished["kind"] == "ui.accounting.evidence_upload_finish"
        assert finished["data"]["content_sha256"] == hashlib.sha256(content).hexdigest()
        assert accounting("evidence_upload_list", {})["data"]["uploads"] == []
        assert evidence.read_evidence_bytes(conn, scope, finished["data"]["id"]) == content
    finally:
        _close_daemon(proc)


def test_retention_and_encrypted_backup(book, tmp_path):
    conn, scope, root = book
    document = evidence.retain_evidence(conn, scope, content=b"private retained bytes", name="Evidence", media_type="text/plain")
    post(conn, scope, "10000")
    saved = close(conn, scope)
    conn.commit()
    with pytest.raises(AppError) as exc:
        reset_current_profile_data(conn, str(root))
    assert exc.value.code == "accounting_retention_required"
    destination = tmp_path / "backup.db"
    _backup_sqlcipher_database(resolve_database_path(root), "test-token-placeholder", destination)
    assert b"private retained bytes" not in destination.read_bytes()
    restored = open_encrypted(destination, "test-token-placeholder", row_factory=get_row_class())
    try:
        assert evidence.read_evidence_bytes(restored, scope, document["id"]) == b"private retained bytes"
        assert export_close(restored, scope, close_id=saved["id"])["snapshot_digest"] == saved["snapshot_digest"]
    finally:
        restored.close()
