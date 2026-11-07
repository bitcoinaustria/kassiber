"""Repro: a list or object staging purpose crashes the Python daemon.

``_document_import_stage_payload`` (#689) checks
``purpose not in {"document_import", "chat_attachment"}``. A renderer that
sends ``"purpose": ["chat_attachment"]`` or ``{}`` makes the set lookup
raise ``TypeError: unhashable type`` instead of the "Unknown staging
purpose" validation error, so the request ends as an internal error. The
Rust daemon refuses it as validation (crates/daemon/src/documents_import.rs).
Exits non-zero while Python still crashes.

    uv run --locked python tests/rust_port/repro_stage_purpose_unhashable.py
"""
from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber import daemon
    from kassiber.errors import AppError

    failures = 0
    for purpose in (["chat_attachment"], {}, "x"):
        try:
            # The purpose is checked before the context is used.
            daemon._document_import_stage_payload(None, {"source_file": "x.csv", "purpose": purpose})
            outcome = "accepted"
        except AppError as exc:
            outcome = f"{exc.code}: {exc}"
        except TypeError as exc:
            outcome = f"TypeError: {exc}"
            failures += 1
        print(f"{purpose!r}: {outcome}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
