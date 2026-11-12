"""Offline witnesses for deliberately retained Wasabi reference quirks."""
from pathlib import Path
import tempfile

from kassiber.cli import handlers
from kassiber.core import imports
from kassiber.importers import load_wasabi_bundle, load_wasabi_bundle_payload


def main():
    base = {'datetime': '2025-01-01', 'tx': 'ab' * 32}
    integer = load_wasabi_bundle_payload([{**base, 'amount': 1}])['records'][0]
    floating = load_wasabi_bundle_payload([{**base, 'amount': 1.0}])['records'][0]
    assert integer['amount'] * 100000000 == 1
    assert floating['amount'] == 1
    # Representation, rather than an explicit unit, selects sats versus BTC.
    invalid = load_wasabi_bundle_payload([{**base, 'tx': 'Not-a-chain-ID', 'amount': 1}])['records'][0]
    assert invalid['txid'] == 'not-a-chain-id'
    try:
        load_wasabi_bundle_payload({'wallet_json': {'accounts': [{'keyPath': "m/²'/0'/0'"}]}})
    except ValueError as error:
        assert str(error) == "invalid literal for int() with base 10: '²'"
    else:
        raise AssertionError('Expected the reference digit/coercion inconsistency')
    import json
    import sqlite3
    with tempfile.TemporaryDirectory(prefix='wasabi-reference-repro-') as root:
        missing = Path(root) / 'missing.json'
        try:
            load_wasabi_bundle(str(missing))
        except FileNotFoundError as error:
            assert error.filename == str(missing)
        else:
            raise AssertionError('Expected the uncaught reference file-open refusal')
        file = Path(root) / 'wasabi.json'
        file.write_text(json.dumps([base | {'amount': 1}]))
        with sqlite3.connect(':memory:') as conn:
            conn.row_factory = sqlite3.Row
            for statement in json.loads((Path(__file__).resolve().parents[2] / 'crates/store/fixtures/creation.json').read_text())['statements']:
                conn.execute(statement)
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01')")
            conn.execute("INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01')")
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('w','ws','p','Source','wasabi','{}','2025-01-01')")
            profile = conn.execute("SELECT * FROM profiles").fetchone()
            wallet = conn.execute("SELECT * FROM wallets").fetchone()
            outcome = imports.import_file_into_wallet(conn, profile, wallet, str(file), 'wasabi_bundle', handlers._import_coordinator_hooks())
            assert outcome['imported'] == 1
            assert 'import_batch_id' not in outcome
            assert conn.execute('SELECT count(*) FROM import_batches').fetchone()[0] == 0
    print('Wasabi unit ambiguity, permissive txid, Unicode digit refusal, missing-file exception and missing import run reproduced')


if __name__ == '__main__':
    main()
