"""Real-writer payout books, including fixed partial slices and decimal terms."""
from copy import deepcopy

from kassiber.cli.handlers import create_direct_swap_payout, create_transaction_pair
from kassiber.core.custody_journal import CustodyJournalBuilder
from kassiber.db import open_db
from tests.test_custody_authored_migration import _scope, _tx, NOW

# A review stored before the price limit keeps its oversized value.
LEGACY_HUGE = '123456789012345678901234567890123456789'
CASES = (
    ('taxable', 'EUR', '123.45', 'taxable', 'generic', None),
    ('reviewed_conversion', 'EUR', '1', 'taxable', 'generic', None),
    ('carry', 'LBTC', '12.5', 'carrying-value', 'generic', None),
    ('at_eth', 'ETH', '3.333', 'carrying-value', 'at', None),
    ('no_value', 'LBTC', None, 'carrying-value', 'generic', None),
    # Both the value and the rate for the 50-unit payout must meet the floor.
    ('tiny', 'EUR', '0.000000000005', 'taxable', 'generic', None),
    # The largest value whose payout rate on the 0.000001 BTC source stays
    # below the tax engine's 1e19 limit; larger values are refused at review.
    ('huge', 'EUR', '9999999999999.99', 'taxable', 'generic', None),
    ('partial_taxable', 'EUR', '10', 'taxable', 'generic', '0.0000004'),
    ('partial_carry', 'LBTC', '10', 'carrying-value', 'generic', '0.0000004'),
)




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
        if name == 'huge':
            patched = deepcopy(snapshot)
            for component in patched['components']:
                for term in component['economic_terms']:
                    term['payout_fiat_value_exact'] = LEGACY_HUGE
            yield 'legacy_' + LEGACY_HUGE, patched
        if name == 'carry':
            for legacy in ('NaN', 'inf', 'junk', '１２.５'):
                patched = deepcopy(snapshot)
                # Raw authored payout payloads retain historical value text.
                for component in patched['components']:
                    for term in component['economic_terms']:
                        term['payout_fiat_value_exact'] = legacy
                yield 'legacy_' + legacy, patched
