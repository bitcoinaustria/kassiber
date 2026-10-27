"""Pin removed-storage names, audit text and source drift on disposable data."""
from __future__ import annotations
import argparse
import hashlib
import inspect
import json
from pathlib import Path
import tempfile
from kassiber import db
from kassiber.core import legacy_ledger

DESTINATION=Path(__file__).resolve().parents[1]/"fixtures/retired.json"

def payload():
    with tempfile.TemporaryDirectory(prefix="kassiber-r3-retired-") as root:
        conn=db.open_db(Path(root)/"data")
        conn.execute("CREATE TABLE sync_books(id TEXT)")
        db._drop_device_sync_schema(conn)
        audit=conn.execute("SELECT impact_json FROM schema_migration_audits WHERE migration_name=?",(db.DEVICE_SYNC_REMOVAL_MIGRATION,)).fetchone()[0]
        conn.close()
    return dict(tables=db.RETIRED_DEVICE_SYNC_TABLES,indexes=db.RETIRED_DEVICE_SYNC_INDEXES,
                impact=json.loads(audit),source_sha256={f.__name__:hashlib.sha256(inspect.getsource(f).encode()).hexdigest()
                for f in (db._drop_device_sync_schema,legacy_ledger._objects,legacy_ledger._drop_all,legacy_ledger.populated_tables,legacy_ledger.retire_if_empty)})

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--check",action="store_true");args=parser.parse_args()
    encoded=json.dumps(payload(),indent=2,ensure_ascii=True)+"\n"
    if args.check:assert DESTINATION.read_text()==encoded,"Python retired-storage contract changed; review the Rust port"
    else:DESTINATION.write_text(encoded)

if __name__=="__main__":main()
