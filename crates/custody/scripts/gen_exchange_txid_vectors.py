#!/usr/bin/env python3
"""Freeze whole finalized-stage results for exchange withdrawal books.

Each case is a real-writer book from tests/test_exchange_txid_transfer.py: a
Strike or River export imported through its production importer and an
owned wallet's observed receipt of the same txid. The exact match books an
exchange_txid_exact move; every less exact variant keeps its hold. Books live
in a disposable directory under the system temporary directory (point TMPDIR
at the test root):

    mise exec rust@1.99.1 uv@latest -- uv run --locked python \\
        crates/custody/scripts/gen_exchange_txid_vectors.py [--check]

Each book's ids come from a per-case uuid5 sequence and its writers read one
pinned clock, so regenerating is byte-identical. ``--check`` recomputes
Python's result from each frozen input.
"""
from contextlib import ExitStack
import json
from pathlib import Path
import sys
import tempfile
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))

from kassiber.core.custody_components import activate_component, create_component  # noqa: E402
from kassiber.core.custody_compute import compute_custody_projection  # noqa: E402
from kassiber.core.custody_journal import CustodyJournalBuilder  # noqa: E402
from kassiber.core.custody_review_terms import dismiss_pair_candidate  # noqa: E402
from tests.custody_snapshot import to_jsonable  # noqa: E402
from tests.test_exchange_txid_transfer import (  # noqa: E402
    FEE_SATS,
    OWNED_ADDRESS,
    OWNED_SCRIPT,
    PRINCIPAL_SATS,
    SECOND_ADDRESS,
    SECOND_SCRIPT,
    SOURCES,
    ExchangeTxidBook,
)

OUT = ROOT / "tests/fixtures/custody_exchange_txid_stage.json"
AS_OF = "2026-11-06T00:00:00.000000Z"
CLOCK = "2026-11-05T00:00:00Z"
CLOCK_NAMES = ("now_iso", "_now_iso")


def pinned(stack, case):
    """Give one book deterministic ids and one wall clock.

    Writers bind ``now_iso`` by name at import, so every loaded kassiber
    module's binding is pinned, as well as the source a later import reads.
    """
    identities = (uuid.uuid5(uuid.NAMESPACE_URL, f"exchange-txid-{case}-{i}") for i in range(1_000_000))
    stack.enter_context(patch.object(uuid, "uuid4", side_effect=lambda: next(identities)))
    for name, module in sorted(sys.modules.items()):
        if module is None or not name.startswith(("kassiber", "tests")):
            continue
        for field in CLOCK_NAMES:
            if callable(getattr(module, field, None)):
                stack.enter_context(patch.object(module, field, return_value=CLOCK))


def books():
    """One builder per case; each takes a fresh book and a source."""

    def platform(book, source, **overrides):
        kind, fmt, records = SOURCES[source]
        return book.platform(kind, fmt, records(**overrides))

    def exact(book, source):
        platform(book, source)
        book.receipt(book.cold())

    def gross(book, source):
        platform(book, source, amount_sats=PRINCIPAL_SATS + FEE_SATS)
        book.receipt(book.cold())

    def missing_txid(book, source):
        platform(book, source, txid=None)
        book.receipt(book.cold())

    def not_owned(book, source):
        platform(book, source)
        book.receipt(book.cold(address=SECOND_ADDRESS))

    def owned_input(book, source):
        platform(book, source)
        cold = book.cold()
        book.cold("Savings", SECOND_ADDRESS)
        book.receipt(cold, inputs=[{
            "txid": "aa" * 32, "vout": 0,
            "prevout": {"scriptpubkey": SECOND_SCRIPT, "value": 10_000_000},
        }])

    def two_outputs(book, source):
        platform(book, source, amount_sats=2 * PRINCIPAL_SATS)
        cold = book.wallet(
            "Cold", "address",
            {"addresses": [OWNED_ADDRESS, SECOND_ADDRESS], "chain": "bitcoin", "network": "main"},
        )
        book.receipt(
            cold, amount_sats=2 * PRINCIPAL_SATS,
            outputs=[(OWNED_SCRIPT, PRINCIPAL_SATS), (SECOND_SCRIPT, PRINCIPAL_SATS)],
        )

    def two_wallets(book, source):
        platform(book, source)
        outputs = [(OWNED_SCRIPT, PRINCIPAL_SATS // 2), (SECOND_SCRIPT, PRINCIPAL_SATS // 2)]
        book.receipt(book.cold(), amount_sats=PRINCIPAL_SATS // 2, outputs=outputs)
        book.receipt(
            book.cold("Savings", SECOND_ADDRESS),
            amount_sats=PRINCIPAL_SATS // 2, outputs=outputs,
        )

    def two_platforms(book, source):
        platform(book, source)
        kind, fmt, records = SOURCES[source]
        book.platform(kind, fmt, records(reference="duplicate")[1:], label="Second account")
        book.receipt(book.cold())

    def dismissed(book, source):
        platform(book, source)
        receipt = book.receipt(book.cold())
        [withdrawal] = book.withdrawal_ids()
        dismiss_pair_candidate(
            book.conn, workspace_id=book.workspace_id, profile_id=book.profile_id,
            out_transaction_id=withdrawal, in_transaction_id=receipt, expires_in_days=0,
        )

    def other_kind(book, source):
        # The same export in a wallet that is not a known platform.
        kind, fmt, records = SOURCES[source]
        book.platform("custom", fmt, records(), label="Imported ledger")
        book.receipt(book.cold())

    def foreign_owner(book, source):
        # The paid output belongs to another wallet than the receiving row's.
        platform(book, source)
        book.cold()
        book.receipt(book.cold("Savings", SECOND_ADDRESS))

    def short_receipt(book, source):
        # The wallet's receipt row does not cover its whole owned output.
        platform(book, source)
        book.receipt(
            book.cold(),
            outputs=[(OWNED_SCRIPT, PRINCIPAL_SATS + FEE_SATS), (SECOND_SCRIPT, 3_000_000)],
        )

    def reviewed_component(book, source):
        # A book reviewed before this match keeps its authored component.
        platform(book, source)
        receipt = book.receipt(book.cold())
        [withdrawal] = book.withdrawal_ids()
        rows = {
            row["id"]: row
            for row in book.conn.execute(
                "SELECT id, wallet_id, occurred_at FROM transactions WHERE id IN (?, ?)",
                (withdrawal, receipt),
            )
        }

        def leg(role, amount, transaction_id):
            row = rows[transaction_id]
            return {
                "id": f"exchange-{role}", "role": role, "rail": "bitcoin",
                "chain": "bitcoin", "network": "main", "asset": "BTC",
                "exposure": "bitcoin", "conservation_unit": "msat",
                "amount_msat": amount, "transaction_id": transaction_id,
                "wallet_id": row["wallet_id"], "occurred_at": row["occurred_at"],
            }

        component = create_component(
            book.conn, workspace_id=book.workspace_id, profile_id=book.profile_id,
            component_type="manual_bridge", evidence_kind="manual_claim",
            evidence_grade="reviewed",
            legs=[
                leg("source", (PRINCIPAL_SATS + FEE_SATS) * 1000, withdrawal),
                leg("destination", PRINCIPAL_SATS * 1000, receipt),
                leg("fee", FEE_SATS * 1000, withdrawal),
            ],
        )
        activate_component(book.conn, component["id"])

    for build in (
        exact, gross, missing_txid, not_owned, owned_input, two_outputs,
        two_wallets, two_platforms, dismissed, other_kind, foreign_owner,
        short_receipt, reviewed_component,
    ):
        yield build.__name__, build


def expected(snapshot):
    try:
        result = compute_custody_projection(snapshot).finalized_tax_projection
        return to_jsonable(result, redact_config=False)
    except Exception as exc:
        return {"error": getattr(exc, "code", type(exc).__name__)}


def cases(root):
    for source in sorted(SOURCES):
        for name, build in books():
            case = f"{source}_{name}"
            with ExitStack() as stack:
                pinned(stack, case)
                book = ExchangeTxidBook(root / case)
                try:
                    build(book, source)
                    book.conn.commit()
                    profile = dict(book.conn.execute("SELECT * FROM profiles").fetchone())
                    snapshot = CustodyJournalBuilder(book.conn, profile, as_of=AS_OF).load_snapshot()
                finally:
                    book.close()
            snapshot = json.loads(json.dumps(snapshot))
            yield {"case": case, "input": snapshot, "expected": expected(snapshot)}


def render(items):
    return json.dumps(list(items), ensure_ascii=True, sort_keys=True) + "\n"


if __name__ == "__main__":
    if "--check" in sys.argv:
        frozen = json.loads(OUT.read_text())
        drifted = [item["case"] for item in frozen if expected(item["input"]) != item["expected"]]
        raise SystemExit("exchange txid vectors drifted: " + ", ".join(drifted) if drifted else 0)
    with tempfile.TemporaryDirectory(prefix="exchange-txid-stage-") as root:
        text = render(cases(Path(root)))
    OUT.write_text(text)
    print(f"{len(json.loads(text))} exchange txid stage cases")
