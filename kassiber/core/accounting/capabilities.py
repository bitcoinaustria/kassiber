"""Read-only accounting enrollment and encryption capabilities."""

from ...errors import AppError
from .ledger import require_encrypted


def snapshot(conn, profile_id: str | None) -> dict:
    requires_encryption = False
    try:
        require_encrypted(conn)
    except AppError as exc:
        if exc.code != "accounting_requires_encryption":
            raise
        requires_encryption = True
    if profile_id is None:
        return {
            "scope_available": False, "configured": False,
            "requires_encryption": requires_encryption,
        }
    if not conn.execute("SELECT 1 FROM profiles WHERE id = ?", (profile_id,)).fetchone():
        raise AppError("Select a book first", code="accounting_context_required")
    configured = conn.execute("SELECT 1 FROM gl_books WHERE profile_id = ?", (profile_id,)).fetchone() is not None
    return {
        "scope_available": True, "configured": configured,
        "requires_encryption": requires_encryption,
    }
