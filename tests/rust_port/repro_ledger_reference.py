from decimal import Decimal
import json
from pathlib import Path
import sqlite3
import tempfile

from kassiber.errors import AppError
from kassiber.importers import load_generic_ledger_records, preview_generic_ledger_records
from kassiber.cli.handlers import import_into_wallet


def main():
    with tempfile.TemporaryDirectory(prefix='ledger-reference-repro-') as root:
        file=Path(root) / 'ledger.csv'
        file.write_text('Date,Sats Amount,Price\n2025-01-02,100000,0.2\n')
        row=load_generic_ledger_records(file)[0]
        assert row['amount']==Decimal('0.001')
        # 5d9a8076d: the sats quantity is converted to BTC before the per-BTC
        # rate prices it (no longer multiplied by 10^8).
        assert row['fiat_value']==Decimal('0.0002')
        assert row['fiat_rate']==Decimal('0.2')
        file.write_text('Person,Example\nType,Date,Received Amount,Received Asset\nDeposit,2025-01-02,0.002,BTC\n')
        assert not preview_generic_ledger_records(file)['confident']
        try:
            load_generic_ledger_records(file)
        except AppError as exc:
            assert exc.code=='ledger_unrecognized'
        else:
            raise AssertionError('Python unexpectedly skipped the preamble')
        file.write_text('Type,Date,Received Amount,Received Asset\n')
        assert preview_generic_ledger_records(file)['amount_max']=='0.000000'
        file.write_text('Type,Date,Received Amount,Received Asset\nDeposit,2025-01-02,1 LBTC,LBTC\n')
        # 11b553bca: an LBTC amount suffix is stripped like the BTC one.
        preview=preview_generic_ledger_records(file)
        assert preview['errors']==0
        file.write_text('Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Tx-ID\nBuy,2025-01-02,0.002,BTC,190,EUR,repeat\n')
        creation = Path(__file__).resolve().parents[2] / 'crates/store/fixtures/creation.json'
        with sqlite3.connect(':memory:') as conn:
            conn.row_factory = sqlite3.Row
            conn.execute('PRAGMA foreign_keys=ON')
            for statement in json.loads(creation.read_text())['statements']:
                conn.executescript(statement)
            conn.execute("INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01')")
            conn.execute("INSERT INTO profiles(id,workspace_id,label,fiat_currency,created_at) VALUES('p','ws','Book','EUR','2025-01-01')")
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('wallet','ws','p','Source','custom','{}','2025-01-01')")
            conn.commit()
            first = import_into_wallet(conn, 'Workspace', 'Book', 'Source', str(file), 'generic_ledger')
            second = import_into_wallet(conn, 'Workspace', 'Book', 'Source', str(file), 'generic_ledger')
            assert first['imported'] == 1
            assert second['imported'] == 0
            assert second['journal_invalidated']
            assert conn.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0] == 2
    print('Preserved ledger reference behaviour: sats-column BTC pricing, preamble refusal, empty-preview amount scale, LBTC suffix parsing, priced-reimport invalidation')


if __name__=='__main__':
    main()
