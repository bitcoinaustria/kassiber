import json
import os
import sqlite3
import subprocess
from pathlib import Path
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from kassiber.core import ui_snapshot
from kassiber.db import open_db
from crates.reports.scripts.gen_gain_vectors import BASE, calculation, stored, outcome, REF

scratch=ROOT/'target/r5-reports15/reference-repro';scratch.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(dir=scratch) as directory:
    conn=open_db(Path(directory)/'data')
    conn.executescript((ROOT/'crates/reports/tests/data/book.sql').read_text()+BASE)
    conn.execute("UPDATE journal_entries SET capital_gains_type='long_term' WHERE id='z'")
    snapshot=ui_snapshot.build_capital_gains_snapshot(conn,2024)
    lot=next(lot for lot in snapshot['lots'] if lot['explanationReference']['entry_id']=='z')
    calc=calculation();calc['fragments'][0]['lot']['pricing']=None
    conn.executescript(stored(calc,compact=False))
    malformed=outcome(conn,'reports.explain-capital-gain',dict(reference=json.dumps(REF)))
    conn.executescript(BASE+"UPDATE profiles SET journal_input_version=8 WHERE id='p';")
    conn.commit()
    stored_year=ui_snapshot.build_capital_gains_snapshot(conn)['year']
    database=Path(conn.execute('PRAGMA database_list').fetchone()[2])
    before={r[0] for r in conn.execute('SELECT id FROM journal_entries')}
    conn.close()
    from tests.test_rust_report_exports import REFERENCE, STAMP
    owners=Path(directory)/'owners';owners.mkdir(mode=0o700)
    env={**os.environ,'KASSIBER_RUST_TEST_OWNER_ROOT':str(owners),'KASSIBER_RUST_TEST_REPORT_TIME':STAMP,
        'KASSIBER_TEST_NO_EGRESS':'strict','KASSIBER_DISABLE_UPDATE_CHECK':'1','KASSIBER_OFFLINE_PREFERENCE_FILE':str(Path(directory)/'offline.json'),
        'XDG_CONFIG_HOME':str(Path(directory)/'config'),'XDG_DATA_HOME':str(Path(directory)/'xdg-data'),'XDG_STATE_HOME':str(Path(directory)/'state')}
    wire=b'{"request_id":"read","kind":"ui.reports.capital_gains"}\n{"request_id":"done","kind":"daemon.shutdown"}\n'
    process=subprocess.run([sys.executable,'-c',REFERENCE,'--data-root',str(Path(directory)/'data'),'daemon'],input=wire,capture_output=True,env=env,timeout=180,check=True)
    assert not process.stderr,process.stderr
    daemon_year=next(record['data']['year'] for record in map(json.loads,process.stdout.splitlines()) if record.get('kind')=='ui.reports.capital_gains')
    conn=sqlite3.connect(database)
    after={r[0] for r in conn.execute('SELECT id FROM journal_entries')}
    versions=conn.execute("SELECT journal_input_version,last_processed_input_version FROM profiles WHERE id='p'").fetchone()
    conn.close()
    print(json.dumps(dict(stored_long_term='long_term',desktop_type=lot['type'],malformed_retained_pricing=malformed,
        stored_report_year=stored_year,daemon_report_year=daemon_year,daemon_read_rebuilt_journals=before!=after,
        daemon_processed_current_version=versions[0]==versions[1]),sort_keys=True))
