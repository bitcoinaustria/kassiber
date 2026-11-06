"""Python reference repro: a list or object artifact `kind` crashes `review apply`.

`kassiber review apply --artifact-file` unwraps a saved plan envelope with
`artifact.get("kind") in {"review.plan", "ui.review.plan"}`. A file whose
`kind` is a list or object raises a bare `TypeError: unhashable type`, which
the CLI answers as `internal_error` instead of refusing the file as
validation. `crates/cli/src/review.rs` mirrors the crash on purpose.
Found while fixing the review operation `type` crash; not fixed here.
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

    review.resolve_scope = lambda conn, workspace, profile: (None, {"id": "profile"})
    with tempfile.TemporaryDirectory(dir=Path.cwd()) as scratch:
        for kind in ([], {}):
            path = Path(scratch) / "artifact.json"
            path.write_text(json.dumps({"kind": kind, "data": {}}))
            args = Namespace(workspace=None, profile=None, review_command="apply",
                             artifact_file=str(path), idempotency_key="repro")
            try:
                review.dispatch_review(None, args)
            except TypeError as error:
                print(f"kind={kind!r}: Python reference bug reproduced: TypeError: {error}")
            except Exception as error:  # noqa: BLE001
                raise SystemExit(f"Reference behavior changed: {type(error).__name__}: {error}")
            else:
                raise SystemExit("Reference behavior changed: accepted")


if __name__ == "__main__":
    main()
