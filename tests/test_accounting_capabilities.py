"""CLI discovery never enrolls books or changes ledger authority."""
import pytest
from kassiber.core.accounts import create_profile, create_workspace
from kassiber.core.accounting import ledger
from kassiber.core.accounting.commands import ACTIONS
from kassiber.daemon_accounting import dispatch_accounting_ui
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError

def call(conn, **args):
    return dispatch_accounting_ui(conn, kind="ui.accounting.capabilities", args=args)

def profile(conn, workspace, name):
    result = create_profile(conn, workspace["id"], name, "EUR", "FIFO", "generic", 365)
    set_setting(conn, "context_workspace", workspace["id"])
    set_setting(conn, "context_profile", result["id"])
    conn.commit()
    return result["id"]

def test_empty_discovery_has_no_financial_or_ui_state(tmp_path):
    conn = open_db(tmp_path / "empty")
    try:
        assert call(conn) == {
            "workspace_id": None, "profile_id": None, "scope_available": False,
            "configured": False, "requires_encryption": True,
        }
        with pytest.raises(AppError) as exc:
            call(conn, profile_id="stale")
        assert exc.value.code == "accounting_scope_changed"
        with pytest.raises(AppError) as exc:
            call(conn, payload={"unexpected": True})
        assert exc.value.code == "accounting_invalid_fields"
        assert "visibility-set" not in ACTIONS
    finally:
        conn.close()

def test_private_book_discovery_does_not_enroll_or_encrypt(tmp_path):
    conn = open_db(tmp_path / "private")
    try:
        scope = profile(conn, create_workspace(conn, "Private"), "Private")
        state = call(conn)
        assert state["scope_available"] and not state["configured"]
        assert state["requires_encryption"]
        assert conn.execute("SELECT COUNT(*) FROM gl_books").fetchone()[0] == 0
        with pytest.raises(AppError) as exc:
            ledger.configure_book(conn, scope, currency="EUR", timezone="Europe/Vienna")
        assert exc.value.code == "accounting_requires_encryption"
    finally:
        conn.close()

def test_enrollment_is_explicit_scoped_and_legacy_visibility_is_ignored(tmp_path):
    pytest.importorskip("sqlcipher3")
    conn = open_db(tmp_path / "encrypted", passphrase="test-token-placeholder")
    try:
        workspace = create_workspace(conn, "Organization")
        first = profile(conn, workspace, "Organization")
        ledger.configure_book(conn, first, currency="EUR", timezone="Europe/Vienna")
        conn.commit()
        before = ledger.snapshot(conn, first)
        set_setting(conn, "accounting_ui_visible:" + first, "false")
        assert call(conn)["configured"] and not call(conn)["requires_encryption"]
        assert "visible" not in call(conn)
        second = profile(conn, workspace, "Private")
        assert not call(conn)["configured"]
        with pytest.raises(AppError) as exc:
            call(conn, profile_id=first)
        assert exc.value.code == "accounting_scope_changed"
        assert call(conn, profile_id=second)["scope_available"]
        assert ledger.snapshot(conn, first) == before
    finally:
        conn.close()
