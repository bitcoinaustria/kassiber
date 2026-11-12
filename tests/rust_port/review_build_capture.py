import itertools
import json
import os
from pathlib import Path
from tests.rust_port.portable_dump import iterdump as _iterdump
from unittest.mock import patch

from kassiber.core import custody_journal, review_workflow
from tests.custody_snapshot import to_jsonable
from tests.rust_port.journal_render_oracle import canonical, capture_build, refusal

CLOCK = "2026-10-31T12:00:00Z"


def pytest_configure(config):
    directory = Path(os.environ["KASSIBER_TEST_REVIEW_BUILD_DIR"]).resolve()
    target = Path(__file__).resolve().parents[2] / "target"
    if not directory.is_relative_to(target):
        raise RuntimeError("review build captures require target/ scratch")
    directory.mkdir(parents=True, exist_ok=True)
    original = review_workflow._build
    sequence = itertools.count()

    def build(conn, profile):
        builder = custody_journal.CustodyJournalBuilder(
            conn, review_workflow._profile(conn, profile), as_of=CLOCK,
        )
        payload = {
            "name": os.environ.get("PYTEST_CURRENT_TEST", "unknown"),
            "sql": "\n".join(_iterdump(conn)), "profile": dict(builder.profile),
            "as_of": CLOCK, "review": True,
        }
        original_load = builder.load_snapshot

        def load():
            snapshot = original_load()
            payload["snapshot"] = snapshot
            return snapshot

        with capture_build(builder) as (record, _), patch.object(
            custody_journal, "build_ledger_state", lambda *_: builder.build(),
        ), patch.object(builder, "load_snapshot", load):
            try:
                result = original(conn, profile)
            except Exception as error:
                payload["expected"] = {"error": refusal(error)}
                raise
            else:
                payload["expected"] = {"state": canonical(to_jsonable(result, redact_config=False))}
                return result
            finally:
                payload["entry_ids"] = record["entry_ids"]
                path = directory / f"{next(sequence):05}.json"
                path.write_text(json.dumps(payload, ensure_ascii=True, allow_nan=False) + "\n")

    review_workflow._build = build
    config.add_cleanup(lambda: setattr(review_workflow, "_build", original))
