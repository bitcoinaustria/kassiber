#!/usr/bin/env python3
"""Reference bug, Python fix pending: late payout-create invalidation commits writes."""
import json
from pathlib import Path
import sys
import tempfile
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import create_direct_swap_payout
from kassiber.db import open_db

with tempfile.TemporaryDirectory(prefix='kassiber-payout-create-rollback-') as root:
    scenario = json.loads((ROOT/'crates/transfers/tests/payout_create.json').read_text())['scenarios'][0]
    c = open_db(root)
    for statement in scenario['sql']: c.execute(statement)
    c.commit()
    version = c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
    c.execute(f"CREATE TEMP TRIGGER refuse_payout_creation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={version+2} BEGIN SELECT RAISE(ABORT,'late payout creation invalidation'); END")
    try:
        create_direct_swap_payout(c,'w','p','d',payout_asset='ETH',payout_amount='0.000000008')
    except Exception as e:
        print(type(e).__name__+': '+str(e))
    else:
        raise AssertionError('Expected late invalidation failure')
    c.rollback()
    count = c.execute("SELECT COUNT(*) FROM custody_components WHERE state='active'").fetchone()[0]
    print('Active components after caller rollback:', count)
    assert count == 1, 'Reference bug no longer reproduces; review the oracle'
    c.close()
