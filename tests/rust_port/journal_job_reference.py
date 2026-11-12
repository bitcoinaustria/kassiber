"""Development hybrid journal replay; invoked only after proven native rollback.

The daemon retains its execution slot and owns progress commits. The pipe carries
the temporary database key in RAM, never in argv, environment, or diagnostics.
"""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))


def main():
    from kassiber.daemon_freshness import refresh_journals_step
    from kassiber.db import open_db
    from kassiber.errors import AppError

    request = json.loads(sys.stdin.readline())
    conn = open_db(request["root"], passphrase=request.pop("passphrase", None), require_existing_schema=True)

    def phase(value):
        print(json.dumps(value), flush=True)
        if json.loads(sys.stdin.readline()) != {"continue": True}:
            raise RuntimeError("Hybrid journal replay interrupted")

    try:
        result = refresh_journals_step(
            conn, request["profile_id"], auto_pair=request["auto_pair"],
            skip_rebuild_when_current=request["skip_rebuild_when_current"], phase=phase,
        )
        response = {"result": {"status": "synced", **result}}
    except AppError as error:
        response = {"error": {"code": error.code, "message": str(error),
                              "hint": error.hint, "details": error.details,
                              "retryable": error.retryable}}
    except Exception as error:
        response = {"exception": {"message": str(error),
                                  "class": f"{type(error).__module__}.{type(error).__qualname__}"}}
    finally:
        conn.rollback()
        conn.close()
    print(json.dumps(response), flush=True)


if __name__ == "__main__":
    main()
