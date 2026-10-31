"""Keep complete native review-build fixtures tied to the Python reference."""
import json
from pathlib import Path
import uuid
from unittest.mock import patch

from kassiber.core import custody_compute, custody_journal, tax_events
from kassiber.core.engines import ledger_response
from tests.custody_snapshot import to_jsonable
from tests.rust_port.journal_render_oracle import canonical, refusal

PATH = Path(__file__).resolve().parents[1] / "crates/journals/tests/data/review-builds.json"


def test_complete_review_build_vectors_replay_through_python():
    for case in json.loads(PATH.read_text()):
        snapshot = case["snapshot"]
        projection = custody_compute.compute_custody_projection(snapshot)
        builder = custody_journal.CustodyJournalBuilder(
            None, snapshot["profile"], as_of=snapshot["as_of"],
        )
        builder.build_custody_projection = lambda: projection
        ids = iter(case["entry_ids"])
        with patch.object(uuid, "uuid4", lambda: uuid.UUID(next(ids))), patch.object(
            ledger_response, "journal_entry_id", lambda *_: next(ids),
        ):
            try:
                state = builder.build()
            except Exception as error:
                actual = {"error": refusal(error)}
            else:
                live = set(case["live_ids"])
                state["quarantines"] = [
                    row for row in tax_events.dedupe_quarantines(state["quarantines"])
                    if str(row["transaction_id"]) in live
                ]
                actual = {"state": canonical(to_jsonable(state, redact_config=False))}
        assert actual == case["expected"], (case["name"], case["sources"])
        assert next(ids, None) is None, case["name"]
