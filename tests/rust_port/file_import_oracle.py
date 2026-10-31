"""Test-only per-invocation oracle for native file imports in hybrid scenarios.

Replay the Python core on an in-memory backup of the disposable book, supply
the same generated identities/clock to the harness binary, and compare every
schema object, row and SQLite storage class. The product never imports this.
"""
from contextlib import closing
from datetime import datetime, timezone
import json
from pathlib import Path
import sqlite3
import subprocess
from types import SimpleNamespace
from unittest import mock
import uuid


def run(binary, argv):
    from kassiber.cli.main import build_parser
    from kassiber.cli import handlers
    from kassiber.core import imports, import_batches, metadata, wallets
    from kassiber.envelope import json_ready
    from kassiber.errors import AppError
    from tests.test_rust_store_migrations import normalized_dump

    args = build_parser().parse_args(argv)
    root = Path(args.data_root)
    databases = [root / "kassiber.sqlite3"]
    if not databases[0].is_file():
        databases = list(root.rglob("kassiber.sqlite3"))
    if len(databases) != 1:
        raise AssertionError("Import oracle needs one disposable, initialized book")
    database = databases[0]
    timestamp = datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    identities = []

    def new_id():
        value = str(uuid.uuid4())
        identities.append(value)
        return value

    import os
    with closing(sqlite3.connect(database)) as source, closing(sqlite3.connect(":memory:")) as reference:
        source.backup(reference)
        reference.row_factory = sqlite3.Row
        reference.execute("PRAGMA foreign_keys=ON")
        id_source = SimpleNamespace(uuid4=new_id)
        with mock.patch.object(wallets, "uuid", id_source), mock.patch.object(wallets, "now_iso", return_value=timestamp), mock.patch.object(imports, "uuid", id_source), mock.patch.object(import_batches, "uuid", id_source), mock.patch.object(metadata, "uuid", id_source), mock.patch.object(imports, "now_iso", return_value=timestamp), mock.patch.object(import_batches, "now_iso", return_value=timestamp), mock.patch.object(handlers, "now_iso", return_value=timestamp):
            error = None
            try:
                outcome = handlers.import_into_wallet(reference, args.workspace, args.profile, args.wallet, args.file, {"import-phoenix":"phoenix_csv", "import-json":"json", "import-csv":"csv", "import-river":"river_csv", "import-bull":"bullbitcoin_csv", "import-bullbitcoin":"bullbitcoin_csv", "import-coinfinity":"coinfinity_csv"}[args.wallets_command], getattr(args,"mode",None))
            except Exception as exc:
                reference.rollback()
                error = exc
        expected = normalized_dump(reference)
    env = {**os.environ, "KASSIBER_RUST_TEST_IMPORT_IDS": ",".join(identities) or "unused", "KASSIBER_RUST_TEST_IMPORT_TIME": timestamp}
    result = subprocess.run([binary, *argv], env=env, capture_output=True, check=False)
    with closing(sqlite3.connect(database)) as actual:
        observed = normalized_dump(actual)
    if observed != expected:
        for table in set(expected[1]) | set(observed[1]):
            if expected[1].get(table) != observed[1].get(table):
                raise AssertionError(f"Native import differs from Python in {table}: {expected[1].get(table)!r} != {observed[1].get(table)!r}")
        raise AssertionError("Native import differs from Python in schema objects")
    if args.machine:
        payload = json.loads(result.stdout)
        if error is None:
            assert result.returncode == 0, result.stdout
            assert payload["data"] == json_ready(outcome), (payload, outcome)
        else:
            assert result.returncode == 1, result.stdout
            code = error.code if isinstance(error, AppError) else "internal_error"
            assert payload["error"]["code"] == code, (payload, error)
            if args.wallets_command in {"import-bull", "import-bullbitcoin", "import-coinfinity"}:
                assert payload["error"]["message"] == str(error), (payload,error)
                assert payload["error"].get("hint") == (error.hint if isinstance(error,AppError) else None), (payload,error)
    return result
