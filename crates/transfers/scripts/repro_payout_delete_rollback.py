#!/usr/bin/env python3
"""Reproduce the Python reference's late payout-invalidation atomicity bug."""
import json
from pathlib import Path
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import delete_direct_swap_payout
from kassiber.db import open_db

with tempfile.TemporaryDirectory(prefix='kassiber-payout-rollback-repro-') as root:
    fixture = json.loads((ROOT/'crates/transfers/tests/payout_delete.json').read_text())['scenarios'][0]
    c = open_db(root)
    for statement in fixture['sql']: c.execute(statement)
    c.commit()
    version = c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
    c.execute(f"CREATE TEMP TRIGGER refuse_payout_invalidation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={version+2} BEGIN SELECT RAISE(ABORT,'late payout invalidation'); END")
    try: delete_direct_swap_payout(c,'w','p',fixture['cases'][0]['args']['payout_id'])
    except Exception as e: print(type(e).__name__+': '+str(e))
    c.rollback()
    state = c.execute('SELECT state FROM custody_components').fetchone()[0]
    print('State after caller rollback:',state)
    assert state == 'superseded', 'Reference bug no longer reproduces; review the oracle'
    c.close()
