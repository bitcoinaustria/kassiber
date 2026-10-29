"""Real-writer payout books. Reference bugs R1/R2/R3 are expected to change.

These books are compared live, never frozen as correct accounting. Partial
payouts exercise R1/R2; every writer value exercises float-storage bug R3.
"""
from copy import deepcopy

from kassiber.cli.handlers import create_direct_swap_payout, create_transaction_pair
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope, _tx, NOW

CASES = (
    ('taxable', 'EUR', '123.45', 'taxable', 'generic', None),
    ('reviewed_conversion', 'EUR', '1', 'taxable', 'generic', None),
    ('carry', 'LBTC', '12.5', 'carrying-value', 'generic', None),
    ('at_eth', 'ETH', '3.333', 'carrying-value', 'at', None),
    ('no_value', 'LBTC', None, 'carrying-value', 'generic', None),
    ('tiny', 'EUR', '0.00000000000000000000000000000001', 'taxable', 'generic', None),
    ('huge', 'EUR', '123456789012345678901234567890123456789', 'taxable', 'generic', None),
    ('partial_taxable_reference_bug_expected_to_change', 'EUR', '10', 'taxable', 'generic', '0.0000004'),
    ('partial_carry_reference_bug_expected_to_change', 'LBTC', '10', 'carrying-value', 'generic', '0.0000004'),
)


# Reference bug, expected to change. Metadata stays outside raw snapshot v1.
REFERENCE_BUGS = {
    name: (('R1', 'R3') if name.startswith('partial_taxable_') else
           ('R2', 'R3') if name.startswith('partial_carry_') else ('R3',))
    for name, _, value, _, _, _ in CASES if value is not None
}


def books(root):
    for name, asset, value, policy, country, out_amount in CASES:
        conn = open_db(root / name)
        try:
            _scope(conn)
            conn.execute('UPDATE profiles SET tax_country=?', (country,))
            _tx(conn, 'out', 'btc', 'outbound', 'BTC', 100000, NOW, fee=1000)
            conn.commit()
            create_direct_swap_payout(conn, 'Books', 'Book', 'out', payout_asset=asset,
                payout_amount='0.0000005' if asset in {'BTC','LBTC'} else '50', policy=policy, payout_fiat_value=value, out_amount=out_amount)
            if name == 'reviewed_conversion':
                _tx(conn, 'conversion-out', 'btc', 'outbound', 'BTC', 10000, NOW)
                _tx(conn, 'conversion-in', 'liquid', 'inbound', 'LBTC', 20000, NOW)
                create_transaction_pair(conn, 'Books', 'Book', 'conversion-out', 'conversion-in', policy='taxable')
            conn.commit()
            profile = dict(conn.execute('SELECT * FROM profiles').fetchone())
            snapshot = CustodyJournalBuilder(conn, profile).load_snapshot()
        finally:
            conn.close()
        yield name, snapshot
        if name == 'carry':
            for legacy in ('NaN', 'inf', 'junk', '１２.５'):
                patched = deepcopy(snapshot)
                # Raw authored payout payloads retain historical value text.
                for component in patched['components']:
                    for term in component['economic_terms']:
                        term['payout_fiat_value_exact'] = legacy
                yield 'legacy_' + legacy, patched
