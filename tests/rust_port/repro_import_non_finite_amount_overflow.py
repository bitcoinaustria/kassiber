"""Python reference repro: a non-finite imported amount crashes as internal_error.

Since 5a8a786c2 ``insert_wallet_records`` normalizes every record before the
book-network guard, and ``normalize_import_record`` converts the amount with
``btc_to_msat``, whose ``int(Decimal('Infinity'))`` raises ``OverflowError``.
An exchange API row (or any importer row) whose amount is ``Infinity`` or
``NaN`` therefore ends the import as ``internal_error`` with the overflow's
text instead of a validation refusal. Nothing is written. The native port
matches the envelope byte for byte (tests/test_rust_exchange_api.py,
``test_non_finite_amounts_fail_where_python_converts_them``); a validation
refusal is an allowed Rust improvement not yet taken.

    uv run --locked python tests/rust_port/repro_import_non_finite_amount_overflow.py
"""
from decimal import Decimal
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber.core.imports import normalize_import_record

    for amount in ("Infinity", "NaN"):
        record = {"txid": "row", "occurred_at": "2025-01-01T00:00:00Z", "direction": "inbound",
                  "asset": "BTC", "amount": Decimal(amount), "fee": Decimal("0")}
        try:
            normalize_import_record(record, source_label="repro")
        except (OverflowError, ValueError) as error:
            print(f"{amount}: {type(error).__name__}: {error}")
        except Exception as error:  # noqa: BLE001 - report whatever Python raises
            print(f"{amount}: refused as {type(error).__name__}: {error}")
            return 0
        else:
            print(f"{amount}: accepted")
            return 0
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
