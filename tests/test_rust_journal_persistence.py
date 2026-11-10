"""Keep the complete R4-12 storage oracle tied to the Python reference."""
import json
from pathlib import Path
import sqlite3

from tests.rust_port.journal_persistence_oracle import PATH,capture,dump
from tests.rust_port.journal_storage_vectors import PATH as STORAGE,vectors


def test_storage_vectors_rebuild_from_python_reference():
    assert json.loads(STORAGE.read_text())==vectors()


def test_python_replays_frozen_complete_persistence_dumps():
    manifest=json.loads((Path(__file__).resolve().parents[1]/'crates/store/fixtures/current-schema.json').read_text())
    for case in json.loads(PATH.read_text()):
        conn=sqlite3.connect(':memory:')
        conn.row_factory=sqlite3.Row
        try:
            for kind,name,table,sql in manifest['objects']:
                if kind=='table': conn.execute(sql)
            for table,data in case['before'].items():
                columns=','.join('"'+c+'"' for c in data['columns'])
                for row in data['rows']:
                    values=[bytes.fromhex(v['$bytes']) if isinstance(v,dict) else v for v in row]
                    conn.execute(f'INSERT INTO "{table}"({columns}) VALUES({",".join("?" for _ in row)})',values)
            for kind in ('index','view','trigger'):
                for object_kind,name,table,sql in manifest['objects']:
                    if kind==object_kind and sql: conn.execute(sql)
            conn.commit();conn.execute('PRAGMA foreign_keys=ON')
            assert dump(conn)==case['before']
            actual=capture(conn,case['name'])
            assert actual['expected']==case['expected'],case['name']
            assert actual['ids']==case['ids'],case['name']
        finally: conn.close()
