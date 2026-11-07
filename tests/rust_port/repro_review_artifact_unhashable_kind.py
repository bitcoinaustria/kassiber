"""Check a list or object artifact `kind` is a typed `review apply` refusal.

    uv run --locked python -m tests.rust_port.repro_review_artifact_unhashable_kind

`kassiber review apply --artifact-file` unwrapped a saved plan envelope with
`artifact.get("kind") in {"review.plan", "ui.review.plan"}`. A file whose
`kind` was a list or object raised a bare `TypeError: unhashable type`, which
the CLI answered as `internal_error`. Such a file now reaches the artifact
validator like one with a number or an unknown string as its kind, and is
refused as `review_artifact_invalid`. Exit 0 means fixed. Scratch stays under
target/.
"""
from argparse import Namespace
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main():
    from kassiber.cli import review
    from kassiber.core import accounts
    from kassiber.db import open_db
    from kassiber.errors import AppError

    scratch = ROOT / "target/review-artifact-kind-repro"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch.resolve()) as temporary:
        conn = open_db(Path(temporary) / "book")
        try:
            workspace = accounts.create_workspace(conn, "W")
            profile = accounts.create_profile(conn, workspace["id"], "P", "EUR", "FIFO", "generic", 365)
            for kind in ([], {}, 5, "other"):
                path = Path(temporary) / "artifact.json"
                path.write_text(json.dumps({"kind": kind, "data": {}}))
                args = Namespace(workspace=workspace["id"], profile=profile["id"], review_command="apply",
                                 artifact_file=str(path), idempotency_key="repro")
                try:
                    review.dispatch_review(conn, args)
                except AppError as error:
                    assert error.code == "review_artifact_invalid", (kind, error.code)
                    print(f"kind={kind!r}: {error.code}: {error}")
                except Exception as error:  # noqa: BLE001
                    raise SystemExit(f"kind={kind!r} still crashes: {type(error).__name__}: {error}")
                else:
                    raise SystemExit(f"kind={kind!r} accepted")
        finally:
            conn.close()


if __name__ == "__main__":
    main()
