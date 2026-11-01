"""Python cached pricing truncates a legacy fractional-msat quantity."""
import sqlite3

from tests.rust_port.journal_prebuild_oracle import capture, rate, seed, transaction


def main():
    conn = sqlite3.connect(':memory:')
    conn.row_factory = sqlite3.Row
    try:
        seed(conn)
        transaction(conn, amount=1.75)
        rate(conn, rate=100000000000, rate_exact='100000000000')
        case = capture(conn, 'fractional-msat-pricing')
        assert case['error'] is None, case['error']
        columns = case['expected']['transactions']['columns']
        row = case['expected']['transactions']['rows'][0]
        assert row[columns.index('amount')] == 1.75
        assert row[columns.index('fiat_value_exact')] == '1.00000000000'
        print('Reproduced: cached pricing values a stored 1.75 msat observation as 1 msat. Python is unchanged.')
    finally:
        conn.close()


if __name__ == '__main__':
    main()
