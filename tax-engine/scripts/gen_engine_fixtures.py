#!/usr/bin/env python3
"""Generate RP2-oracle fixtures for the native tax engine's lot methods.

Each case is an engine request built with ``kassiber/core/engines/
native_request.py`` plus the response RP2 gives for the same input, run as
Kassiber's adapter (``kassiber/core/engines/rp2.py``) drives it:

* ``compute`` cases run ``rp2.tax_engine.compute_tax`` for one asset with
  fifo, lifo, hifo, or lofo under the generic or Austrian country, and
  record every ``ComputedData`` value the adapter reads, or the error.
* ``check_entry`` cases run one transaction constructor and record the
  values it stores and derives, or the error it raises.

The inputs are the adapter's constructor arguments: inbound rows with
``fiat_in_no_fee == fiat_in_with_fee`` and a zero fiat fee, SELL or FEE
outbound rows with ``fiat_fee = fee * spot`` computed in Kassiber's 32-digit
context, and moves with ``spot_price`` 0 when unknown. The histories cover
every lot method, partial lots, fees, earn types, moves, same-instant ties,
near-tie unit costs, holding-period boundaries, offsets and microseconds,
pre-1970 rows, and error cases. Every entry's ``text`` is checked against
RP2's own ``str(transaction)``; random histories keep it only where an
expected message embeds it, to stay within the size budget.

RP2 is imported on the main thread, which sets that thread's decimal context
to 32 digits; every value here is computed on that thread. Run it from a
scratch directory (RP2 may create a log directory in the working directory):

    cd "$(mktemp -d)" && uv run --locked --project <repo> \\
        python <repo>/tax-engine/scripts/gen_engine_fixtures.py

It writes tax-engine/core/tests/data/generic_fixtures.json. The output
depends only on the seed and the pinned RP2. For a stress run, write a
larger set elsewhere and point the Rust test at it:

    ... gen_engine_fixtures.py --seed 7 --histories 5000 --max-bytes 0 \
        --output /tmp/engine_stress.json
    KASSIBER_ENGINE_FIXTURES=/tmp/engine_stress.json \
        cargo test -p kassiber-tax-core --test engine_fixtures
"""

from __future__ import annotations

import argparse
import decimal
import json
import random
import tempfile
from datetime import datetime, timedelta, timezone
from decimal import ROUND_HALF_EVEN, Context, Decimal, localcontext
from importlib import import_module, util
from pathlib import Path
from typing import Any, Callable, Optional
from zoneinfo import ZoneInfo

from prezzemolo.avl_tree import AVLTree
from rp2.abstract_country import AbstractCountry
from rp2.accounting_engine import AccountingEngine
from rp2.configuration import Configuration
from rp2.in_transaction import InTransaction
from rp2.input_data import InputData
from rp2.intra_transaction import IntraTransaction
from rp2.out_transaction import OutTransaction
from rp2.rp2_decimal import RP2Decimal
from rp2.tax_engine import compute_tax
from rp2.transaction_set import TransactionSet

REPO = Path(__file__).resolve().parents[2]
OUTPUT = REPO / "tax-engine" / "core" / "tests" / "data" / "generic_fixtures.json"
SEED = 20261002
RANDOM_HISTORIES = 440
MAX_BYTES = 3_200_000
HOLDER = "Profile"
ASSET = "BTC"
METHODS = ("fifo", "lifo", "hifo", "lofo")
EARN_TYPES = ("INCOME", "MINING", "STAKING", "INTEREST", "AIRDROP", "HARDFORK")
CTX = Context(prec=32, rounding=ROUND_HALF_EVEN)


def _load_native_request():
    # Loaded by path: importing the kassiber package would pull in the adapter.
    path = REPO / "kassiber" / "core" / "engines" / "native_request.py"
    spec = util.spec_from_file_location("native_request", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {path}")
    module = util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


NR = _load_native_request()


# ---------------------------------------------------------------------------
# Constructor arguments, as the adapter passes them (decimals as strings)


def kassiber_product(a: str, b: str) -> str:
    """The adapter's ``fee * spot``, in Kassiber's 32-digit context."""
    with localcontext(CTX):
        return str(Decimal(a) * Decimal(b))


def buy(uid, ts, wallet, spot, amount, fiat, type_="BUY", notes=""):
    return {
        "kind": "in",
        "unique_id": uid,
        "timestamp": ts,
        "exchange": wallet,
        "transaction_type": type_,
        "spot_price": spot,
        "crypto_in": amount,
        "fiat_in_no_fee": fiat,
        "fiat_in_with_fee": fiat,
        "fiat_fee": "0",
        "notes": notes,
    }


def sell(uid, ts, wallet, amount, spot, fiat, fee="0", notes=""):
    return {
        "kind": "out",
        "unique_id": uid,
        "timestamp": ts,
        "exchange": wallet,
        "transaction_type": "SELL",
        "spot_price": spot,
        "crypto_out_no_fee": amount,
        "crypto_fee": fee,
        "fiat_out_no_fee": fiat,
        "fiat_fee": kassiber_product(fee, spot),
        "notes": notes,
    }


def fee_out(uid, ts, wallet, fee, spot, notes=""):
    """A FEE row: a fee-only outbound or a channel-open fee."""
    return {
        "kind": "out",
        "unique_id": uid,
        "timestamp": ts,
        "exchange": wallet,
        "transaction_type": "FEE",
        "spot_price": spot,
        "crypto_out_no_fee": "0",
        "crypto_fee": fee,
        "fiat_out_no_fee": None,
        "fiat_fee": kassiber_product(fee, spot),
        "notes": notes,
    }


def move(uid, ts, src, dst, sent, received, spot, notes=""):
    return {
        "kind": "intra",
        "unique_id": uid,
        "timestamp": ts,
        "from_exchange": src,
        "to_exchange": dst,
        "spot_price": spot,
        "crypto_sent": sent,
        "crypto_received": received,
        "notes": notes,
    }


def scenario(name, method, wallets, rows, *, long_term_days=365, country="generic"):
    return {
        "name": name,
        "method": method,
        "country": country,
        "long_term_days": long_term_days,
        "wallets": wallets,
        "rows": rows,
    }


# Every B-scenario of the black-box spec (adapter-reachable ones and the
# error, tolerance, and pre-1970 cases), in adapter emission order.
B_SCENARIOS = [
    scenario('B01_fifo_partial_lots_long_short_split', 'fifo', ['A'], [
        buy('b1', '2023-01-01T00:00:00Z', 'A', '16500', '0.5', '8250.00'),
        buy('b2', '2023-06-15T10:00:00Z', 'A', '23000', '0.3', '6900.00'),
        buy('b3', '2023-11-01T00:00:00Z', 'A', '31000', '0.4', '12400'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.6', '42000', '25200.00'),
        sell('s2', '2024-07-01T00:00:00Z', 'A', '0.35', '60000', '21000'),
    ]),
    scenario('B02_lifo_newest_first_with_late_arrival', 'lifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-01-02T00:00:00Z', 'A', '300', '1', '300'),
        buy('b3', '2024-01-03T00:00:00Z', 'A', '200', '1', '200'),
        sell('s1', '2024-01-04T00:00:00Z', 'A', '0.4', '250', '100'),
        buy('b4', '2024-01-05T00:00:00Z', 'A', '150', '1', '150'),
        sell('s2', '2024-01-06T00:00:00Z', 'A', '1.2', '250', '300'),
        sell('s3', '2024-01-07T00:00:00Z', 'A', '1.4', '250', '350'),
    ]),
    scenario('B03_hifo_highest_unit_cost_with_late_arrival', 'hifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-01-02T00:00:00Z', 'A', '90', '0.5', '200'),
        buy('b3', '2024-01-03T00:00:00Z', 'A', '300', '1', '300'),
        sell('s1', '2024-01-04T00:00:00Z', 'A', '0.7', '250', '175'),
        buy('b4', '2024-01-05T00:00:00Z', 'A', '350', '0.2', '70'),
        sell('s2', '2024-01-06T00:00:00Z', 'A', '1.5', '250', '375'),
    ]),
    scenario('B04_lofo_lowest_unit_cost_with_late_arrival', 'lofo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-01-02T00:00:00Z', 'A', '300', '1', '60'),
        buy('b3', '2024-01-03T00:00:00Z', 'A', '200', '1', '200'),
        sell('s1', '2024-01-04T00:00:00Z', 'A', '1.2', '250', '300'),
        buy('b4', '2024-01-05T00:00:00Z', 'A', '50', '0.3', '15'),
        sell('s2', '2024-01-06T00:00:00Z', 'A', '1.0', '250', '250'),
    ]),
    scenario('B05_hifo_lofo_tie_breaks_equal_and_13dp_near_equal', 'hifo', ['A'], [
        buy('e1', '2024-01-03T00:00:00Z', 'A', '100', '1', '100'),
        buy('e3', '2024-01-03T00:00:00Z', 'A', '100', '0.5', '50.0'),
        buy('n1', '2024-01-04T00:00:00Z', 'A', '100', '1', '100.00000000000005'),
        buy('n2', '2024-01-05T00:00:00Z', 'A', '100', '1', '100.00000000000006'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '3.2', '250', '800'),
    ]),
    scenario('B06_lofo_same_inputs_as_B05', 'lofo', ['A'], [
        buy('e1', '2024-01-03T00:00:00Z', 'A', '100', '1', '100'),
        buy('e3', '2024-01-03T00:00:00Z', 'A', '100', '0.5', '50.0'),
        buy('n1', '2024-01-04T00:00:00Z', 'A', '100', '1', '100.00000000000005'),
        buy('n2', '2024-01-05T00:00:00Z', 'A', '100', '1', '100.00000000000006'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '3.2', '250', '800'),
    ]),
    scenario('B07_lifo_same_timestamp_lots_highest_row_first', 'lifo', ['A'], [
        buy('x1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('x2', '2024-01-01T00:00:00Z', 'A', '300', '1', '300'),
        buy('x3', '2024-01-01T00:00:00Z', 'A', '200', '1', '200'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '1.5', '250', '375'),
    ]),
    scenario('B08_fifo_same_timestamp_lots_lowest_row_first', 'fifo', ['A'], [
        buy('x1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('x2', '2024-01-01T00:00:00Z', 'A', '300', '1', '300'),
        buy('x3', '2024-01-01T00:00:00Z', 'A', '200', '1', '200'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '1.5', '250', '375'),
    ]),
    scenario('B09_same_timestamp_buy_and_sell', 'fifo', ['A'], [
        buy('b0', '2024-03-01T12:00:00Z', 'A', '60000', '0.25', '15000'),
        sell('s0', '2024-03-01T12:00:00Z', 'A', '0.1', '60000', '6000'),
    ]),
    scenario('B10_same_timestamp_sell_processed_before_move_fee', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-02-01T00:00:00Z', 'A', '200', '1', '200'),
        move('m1', '2024-03-01T12:00:00Z', 'A', 'B', '0.5', '0.4', '300', notes='Transfer A -> B'),
        sell('s1', '2024-03-01T12:00:00Z', 'A', '0.95', '300', '285'),
    ]),
    scenario('B11_same_time_move_chain_A_B_C', 'fifo', ['A', 'B', 'C'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        move('m1', '2024-02-01T00:00:00Z', 'A', 'B', '1', '0.9999', '200'),
        move('m2', '2024-02-01T00:00:00Z', 'B', 'C', '0.9999', '0.9997', '200'),
        sell('s1', '2024-03-01T00:00:00Z', 'C', '0.5', '210', '105'),
    ]),
    scenario('B12_earn_types_and_same_time_sale', 'fifo', ['A'], [
        buy('e_airdrop', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'AIRDROP'),
        buy('e_hardfork', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'HARDFORK'),
        buy('e_income', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'INCOME'),
        buy('e_interest', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'INTEREST'),
        buy('e_mining', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'MINING'),
        buy('e_staking', '2024-05-01T00:00:00Z', 'A', '60000', '0.001', '60.00', 'STAKING'),
        sell('s1', '2024-05-01T00:00:00Z', 'A', '0.0025', '61000', '152.50'),
    ]),
    scenario('B13_sell_with_crypto_fee', 'fifo', ['A'], [
        buy('b1', '2023-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2023-12-01T00:00:00Z', 'A', '300', '1', '300.50'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '1.2', '300', '360', fee='0.0001'),
    ]),
    scenario('B14_fee_only_and_channel_open_fee', 'fifo', ['A'], [
        buy('b1', '2023-01-01T00:00:00Z', 'A', '16500', '1', '16500'),
        fee_out('f1', '2024-01-01T00:00:00Z', 'A', '0.00001234', '42000.37'),
        fee_out('c1', '2024-01-02T00:00:00Z', 'A', '0.00000321', '43000'),
    ]),
    scenario('B15_move_zero_fee_tiny_fee_and_cross_wallet_sale', 'fifo', ['A', 'B', 'C'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-01-02T00:00:00Z', 'C', '300', '1', '300'),
        move('m0', '2024-02-01T00:00:00Z', 'C', 'B', '1', '1', '0'),
        move('m5', '2024-02-02T00:00:00Z', 'A', 'B', '0.1', '0.09999999999', '0.005'),
        move('m6', '2024-02-03T00:00:00Z', 'A', 'B', '0.1', '0.09999999999', '0.006'),
        sell('s1', '2024-03-01T00:00:00Z', 'B', '0.5', '200', '100'),
    ]),
    scenario('B16_long_term_boundary_generic_365', 'fifo', ['A'], [
        buy('b1', '2023-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2023-01-02T00:00:00Z', 'A', '100', '1', '100'),
        buy('b3', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '1', '200', '200'),
        sell('s2', '2024-01-01T23:59:59Z', 'A', '1', '200', '200'),
        sell('s3', '2024-12-31T00:00:00Z', 'A', '1', '200', '200'),
    ]),
    scenario('B17_long_term_days_zero_and_one', 'lifo', ['A'], [
        buy('i1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100', 'INCOME'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '200', '100'),
    ], long_term_days=0),
    scenario('B18_at_country_fifo_never_long_term', 'fifo', ['A'], [
        buy('b1', '2015-01-01T00:00:00Z', 'A', '250', '1', '250', notes='at_regime=alt at_pool=default'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '40000', '20000', notes='at_regime=alt at_pool=default'),
    ], country='at'),
    scenario('B19_precision_32_digit_operation_order', 'fifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '110440.45', '0.77661975935', '85770065013.3'),
        buy('b2', '2024-01-02T00:00:00Z', 'A', '100', '3', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '0.70009036886', '1', '85770065013.3'),
        sell('s2', '2024-02-02T00:00:00Z', 'A', '1.07652939049', '41000.12', '44138.70'),
        sell('s3', '2024-02-03T00:00:00Z', 'A', '1', '41000', '41000', fee='0.00000700'),
    ]),
    scenario('B20_year_boundary_partial_lot', 'hifo', ['A'], [
        buy('b1', '2022-12-31T23:59:59Z', 'A', '15000', '0.4', '6000'),
        buy('b2', '2023-03-01T00:00:00Z', 'A', '21000', '0.4', '8400'),
        sell('s1', '2023-12-31T23:59:59Z', 'A', '0.5', '42000', '21000'),
        sell('s2', '2024-01-01T00:00:00Z', 'A', '0.3', '42100', '12630'),
    ]),
    scenario('B21_balance_order_by_exchange_underscore_holder', 'fifo', ['A', 'A-b', 'A_x', 'B', 'a'], [
        buy('b0', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b1', '2024-01-01T00:00:00Z', 'a', '100', '1', '100'),
        move('m1', '2024-01-02T00:00:00Z', 'A', 'A-b', '1', '1', '0'),
        move('m2', '2024-01-03T00:00:00Z', 'a', 'A_x', '1', '0.999', '100'),
        move('m3', '2024-01-04T00:00:00Z', 'A_x', 'B', '0.999', '0.999', '0'),
    ]),
    scenario('B22_income_lot_partial_with_earn_between', 'lifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '40000', '0.01', '400.00'),
        buy('i1', '2024-01-10T00:00:00Z', 'A', '41000', '0.002', '82.00', 'MINING'),
        sell('s1', '2024-01-11T00:00:00Z', 'A', '0.0015', '42000', '63.00'),
        buy('i2', '2024-01-12T00:00:00Z', 'A', '43000', '0.001', '43.00', 'STAKING'),
        sell('s2', '2024-01-13T00:00:00Z', 'A', '0.003', '44000', '132.00'),
    ]),
    scenario('B28_sold_percentage_thirds_not_exactly_one', 'fifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '3', '300'),
        buy('b2', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '1', '200', '200'),
        sell('s2', '2024-02-02T00:00:00Z', 'A', '1', '200', '200'),
        sell('s3', '2024-02-03T00:00:00Z', 'A', '1', '200', '200'),
    ]),
    scenario('B29_same_timestamp_earn_sell_move_full_tie_order', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '0.3', '30'),
        buy('i1', '2024-04-01T08:00:00Z', 'A', '500', '0.2', '100', 'INTEREST'),
        move('m1', '2024-04-01T08:00:00Z', 'A', 'B', '0.2', '0.19', '500'),
        sell('s1', '2024-04-01T08:00:00Z', 'A', '0.25', '500', '125'),
    ]),
    scenario('B23_error_lot_exhaustion', 'fifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        buy('b2', '2024-03-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '1.5', '200', '300'),
    ]),
    scenario('B24_error_account_balance_negative', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'B', '0.5', '200', '100'),
    ]),
    scenario('B25_negative_balance_tolerance_5_msat', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'B', '5E-11', '200', '1E-8'),
    ]),
    scenario('B26_error_move_fee_without_spot', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        move('m1', '2024-02-01T00:00:00Z', 'A', 'B', '0.5', '0.4', '0'),
    ]),
    scenario('B27_pre_1970_rows_are_invisible', 'fifo', ['A'], [
        buy('old', '1969-12-31T00:00:00Z', 'A', '1', '1', '1'),
        buy('b1', '1970-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '200', '100'),
    ]),
]


# Hand-written cases for behavior the B-scenarios do not pin.
EXTRA_SCENARIOS = [
    scenario('X01_offsets_microseconds_and_local_years', 'fifo', ['A'], [
        buy('b1', '2023-12-31T23:30:00-01:00', 'A', '100', '1', '100'),
        buy('b2', '2024-01-01T00:00:00.250000Z', 'A', '200', '1', '200'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '300', '150'),
        sell('s2', '2024-12-31T23:30:00-01:00', 'A', '1', '300', '300'),
        sell('s3', '2025-01-01T00:30:00+01:00', 'A', '0.25', '300', '75'),
    ], long_term_days=366),
    scenario('X02_offset_holding_period_boundary', 'lifo', ['A'], [
        buy('b1', '2023-01-01T00:00:00+01:00', 'A', '100', '1', '100'),
        buy('b2', '2023-01-01T00:00:00-01:00', 'A', '100', '1', '100'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '1.5', '300', '450'),
    ]),
    scenario('X03_lot_visible_by_offset_before_utc_1970', 'hifo', ['A'], [
        buy('b0', '1970-01-01T00:30:00+01:00', 'A', '10', '1', '10'),
        buy('b1', '1969-12-31T23:00:00Z', 'A', '1', '1', '1'),
        buy('b2', '1971-01-01T00:00:00Z', 'A', '20', '1', '20'),
        sell('s0', '1969-12-31T23:59:59Z', 'A', '5', '1', '5'),
        sell('s1', '1972-01-01T00:00:00Z', 'A', '1.5', '30', '45'),
    ]),
    scenario('X04_all_lots_before_1970', 'fifo', ['A'], [
        buy('b1', '1969-06-01T00:00:00Z', 'A', '1', '1', '1'),
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '200', '100'),
    ]),
    scenario('X05_row_shared_by_lot_and_disposal', 'fifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'A', '0.5', '200', '100'),
    ]),
    scenario('X06_self_transfer_and_negative_move', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        move('m1', '2024-01-02T00:00:00Z', 'A', 'A', '0.4', '0.39', '150'),
        move('m2', '2024-01-03T00:00:00Z', 'B', 'A', '0.5', '0.5', '0'),
    ]),
    scenario('X07_sale_before_any_lot', 'lofo', ['A'], [
        sell('s1', '2024-01-01T00:00:00Z', 'A', '0.5', '200', '100'),
        buy('b1', '2024-02-01T00:00:00Z', 'A', '100', '1', '100'),
    ]),
    scenario('X08_overdraft_six_msat', 'fifo', ['A', 'B'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '1', '100'),
        sell('s1', '2024-02-01T00:00:00Z', 'B', '6E-11', '200', '1E-8'),
    ]),
    scenario('X09_lifo_lots_after_last_disposal', 'lifo', ['A'], [
        buy('b1', '2024-01-01T00:00:00Z', 'A', '100', '2', '200'),
        sell('s1', '2024-01-02T00:00:00Z', 'A', '0.5', '120', '60'),
        buy('b2', '2024-01-03T00:00:00Z', 'A', '130', '1', '130'),
        buy('b3', '2024-01-04T00:00:00Z', 'A', '140', '1', '140'),
    ]),
    scenario('X10_wages_type_and_at_country', 'hifo', ['A', 'B'], [
        buy('w1', '2021-02-28T23:30:00Z', 'A', '40000', '0.01', '400', 'WAGES'),
        buy('b1', '2021-03-01T00:00:00Z', 'A', '41000', '0.01', '410'),
        move('m1', '2022-01-01T00:00:00Z', 'A', 'B', '0.015', '0.0149', '45000'),
        sell('s1', '2023-01-01T00:00:00Z', 'B', '0.0149', '20000', '298'),
    ], country='at'),
]

# Rows of X05 collide across sets: the disposal reuses the lot's row.
ROW_OVERRIDES = {"X05_row_shared_by_lot_and_disposal": [1, 1]}


# ---------------------------------------------------------------------------
# Running RP2


class _Country(AbstractCountry):
    """Kassiber's generic country, as the adapter builds it."""

    def __init__(self, long_term_days: int) -> None:
        super().__init__("generic", "EUR")
        self._long_term_days = long_term_days

    def get_long_term_capital_gain_period(self) -> int:
        return self._long_term_days

    def get_default_accounting_method(self) -> str:
        return "fifo"

    def get_accounting_methods(self):
        return {"fifo", "lifo", "hifo", "lofo", "moving_average"}

    def get_report_generators(self):
        return {"open_positions", "rp2_full_report"}

    def get_default_generation_language(self) -> str:
        return "en"


INI_TEMPLATE = """[general]
assets = {assets}
exchanges = {exchanges}
holders = {holder}

[in_header]
timestamp = 0
asset = 1
exchange = 2
holder = 3
transaction_type = 4
spot_price = 5
crypto_in = 6
crypto_fee = 7
fiat_in_no_fee = 8
fiat_in_with_fee = 9
fiat_fee = 10
unique_id = 11
notes = 12

[out_header]
timestamp = 0
asset = 1
exchange = 2
holder = 3
transaction_type = 4
spot_price = 5
crypto_out_no_fee = 6
crypto_fee = 7
crypto_out_with_fee = 8
fiat_out_no_fee = 9
fiat_fee = 10
unique_id = 11
notes = 12

[intra_header]
timestamp = 0
asset = 1
from_exchange = 2
from_holder = 3
to_exchange = 4
to_holder = 5
spot_price = 6
crypto_sent = 7
crypto_received = 8
unique_id = 9
notes = 10
"""


def configuration(scn: dict[str, Any], scratch: Path) -> Configuration:
    country = import_module("rp2.plugin.country.at").AT() if scn["country"] == "at" else _Country(scn["long_term_days"])
    path = scratch / "rp2.ini"
    path.write_text(
        INI_TEMPLATE.format(assets=ASSET, exchanges=", ".join(sorted(scn["wallets"])), holder=HOLDER),
        encoding="utf-8",
    )
    try:
        return Configuration(str(path), country)
    finally:
        path.unlink()


def accounting_engine(method: str) -> AccountingEngine:
    tree = AVLTree()
    tree.insert_node(1970, import_module(f"rp2.plugin.accounting_method.{method}").AccountingMethod())
    return AccountingEngine(years_2_methods=tree)


DECIMAL_KEYS = {
    "spot_price", "crypto_in", "crypto_fee", "fiat_in_no_fee", "fiat_in_with_fee", "fiat_fee",
    "crypto_out_no_fee", "crypto_out_with_fee", "fiat_out_no_fee", "crypto_sent", "crypto_received",
}


def constructor_arguments(row: dict[str, Any], row_number: int, decimal_type: type) -> tuple[str, dict[str, Any]]:
    """The keyword arguments the adapter passes, with decimals of ``decimal_type``."""
    kwargs: dict[str, Any] = {"asset": ASSET, "row": row_number}
    for key, value in row.items():
        if key == "kind":
            continue
        kwargs[key] = decimal_type(value) if key in DECIMAL_KEYS and value is not None else value
    if row["kind"] == "intra":
        kwargs["from_holder"] = kwargs["to_holder"] = HOLDER
    else:
        kwargs["holder"] = HOLDER
    return row["kind"], kwargs


RP2_CLASSES = {"in": InTransaction, "out": OutTransaction, "intra": IntraTransaction}
NATIVE_BUILDERS = {"in": NR.in_entry, "out": NR.out_entry, "intra": NR.intra_entry}


def error_document(exc: BaseException) -> dict[str, Any]:
    classes = {
        "RP2ValueError": "ValueError",
        "RP2TypeError": "TypeError",
        "RP2RuntimeError": "RuntimeError",
        "InvalidOperation": "InvalidOperation",
        "DivisionByZero": "DivisionByZero",
        "Overflow": "Overflow",
    }
    name = type(exc).__name__
    if name not in classes:
        raise RuntimeError(f"unexpected exception from RP2: {name}: {exc}") from exc
    return {"schema_version": 1, "ok": False, "error": {"class": classes[name], "message": str(exc)}}


def enc(value: Optional[Decimal]) -> Optional[str]:
    return NR.encode_optional_decimal(value)


def derived_values(kind: str, tx: Any) -> dict[str, Any]:
    """The stored and derived values ``check_entry`` reports, read from RP2."""
    try:
        taxable: Optional[bool] = tx.is_taxable()
    except decimal.InvalidOperation:
        taxable = None
    out = {
        "kind": kind,
        "row": tx.row,
        "unique_id": tx.unique_id,
        "notes": tx.notes,
        "transaction_type": tx.transaction_type.value,
        "spot_price": enc(tx.spot_price),
        "crypto_in": None,
        "crypto_fee": enc(tx.crypto_fee),
        "fiat_fee": enc(tx.fiat_fee),
        "fiat_in_no_fee": None,
        "fiat_in_with_fee": None,
        "crypto_out_no_fee": None,
        "crypto_out_with_fee": None,
        "fiat_out_no_fee": None,
        "fiat_out_with_fee": None,
        "crypto_sent": None,
        "crypto_received": None,
        "crypto_balance_change": enc(tx.crypto_balance_change),
        "crypto_taxable_amount": enc(tx.crypto_taxable_amount),
        "fiat_taxable_amount": enc(tx.fiat_taxable_amount),
        "is_taxable": taxable,
        "is_earn": tx.is_earning(),
    }
    if kind == "in":
        out.update(crypto_in=enc(tx.crypto_in), fiat_in_no_fee=enc(tx.fiat_in_no_fee), fiat_in_with_fee=enc(tx.fiat_in_with_fee))
    elif kind == "out":
        out.update(
            crypto_out_no_fee=enc(tx.crypto_out_no_fee),
            crypto_out_with_fee=enc(tx.crypto_out_with_fee),
            fiat_out_no_fee=enc(tx.fiat_out_no_fee),
            fiat_out_with_fee=enc(tx.fiat_out_with_fee),
        )
    else:
        out.update(crypto_sent=enc(tx.crypto_sent), crypto_received=enc(tx.crypto_received))
    return out


def check_timestamp(where: str, fields: dict[str, Any], parsed: datetime) -> None:
    """``native_request``'s timestamp fields must equal RP2's own parse."""
    vienna = parsed.astimezone(ZoneInfo("Europe/Vienna")).date().isoformat()
    expected = {
        "raw": fields["raw"],
        "us": (parsed - datetime(1970, 1, 1, tzinfo=timezone.utc)) // timedelta(microseconds=1),
        "offset_s": parsed.utcoffset() // timedelta(seconds=1),
        "date": parsed.date().isoformat(),
        "year": parsed.year,
        "vienna_date": vienna,
        "display": str(parsed),
        "iso": parsed.isoformat(),
    }
    if fields != expected:
        raise SystemExit(f"{where}: timestamp fields {fields} differ from RP2's parse {expected}")


def computed_document(computed: Any) -> dict[str, Any]:
    """Every ``ComputedData`` value the adapter reads, in the engine's form."""
    kinds = {InTransaction: "in", OutTransaction: "out", IntraTransaction: "intra"}
    lots = list(computed.in_transaction_set)
    gain_losses = [
        {
            "event": {"kind": kinds[type(gl.taxable_event)], "row": gl.taxable_event.row},
            "lot": gl.acquired_lot.row if gl.acquired_lot is not None else None,
            "crypto_amount": enc(gl.crypto_amount),
            "fiat_cost_basis": enc(gl.fiat_cost_basis),
            "proceeds": enc(gl.taxable_event_fiat_amount_with_fee_fraction),
            "fiat_gain": enc(gl.fiat_gain),
            "unit_cost_basis_override": enc(gl.unit_cost_basis_override),
            "long_term": gl.is_long_term_capital_gains(),
        }
        for gl in computed.gain_loss_set
    ]
    yearly = [
        {
            "year": y.year,
            "transaction_type": y.transaction_type.value,
            "long_term": y.is_long_term_capital_gains,
            "crypto_amount": enc(y.crypto_amount),
            "fiat_amount": enc(y.fiat_amount),
            "fiat_cost_basis": enc(y.fiat_cost_basis),
            "fiat_gain_loss": enc(y.fiat_gain_loss),
        }
        for y in computed.yearly_gain_loss_list
    ]
    open_positions = [
        {
            "row": tx.row,
            "sold_percentage": enc(computed.get_open_position_in_lot_sold_percentage(tx)),
            "fiat_in_with_fee": enc(computed.get_open_position_in_transaction_fiat_in_with_fee(tx)),
        }
        for tx in computed.open_position_in_transaction_set
    ]
    balances = [
        {
            "exchange": b.exchange,
            "holder": b.holder,
            "final_balance": enc(b.final_balance),
            "acquired_balance": enc(b.acquired_balance),
            "sent_balance": enc(b.sent_balance),
            "received_balance": enc(b.received_balance),
        }
        for b in computed.balance_set
    ]
    asset = {
        "asset": computed.asset,
        "in_transactions": [tx.row for tx in lots],
        "in_fiat_in_with_fee": [enc(computed.get_in_transaction_fiat_in_with_fee(tx)) for tx in lots],
        "gain_losses": gain_losses,
        "yearly": yearly,
        "open_positions": open_positions,
        "balances": balances,
    }
    return {"schema_version": 1, "ok": True, "assets": [asset]}


class Cases:
    """Collects fixture cases and counts what they cover."""

    def __init__(self) -> None:
        self.cases: list[dict[str, Any]] = []
        self.stats: dict[str, int] = {}

    def add(self, name: str, operation: str, request: dict[str, Any], expected: dict[str, Any]) -> None:
        self.cases.append({"name": name, "operation": operation, "request": request, "expected": expected})
        outcome = "ok" if expected["ok"] else "error"
        key = f"{operation}:{outcome}"
        self.stats[key] = self.stats.get(key, 0) + 1


def membership(scn: dict[str, Any]) -> dict[str, Any]:
    return {"assets": [ASSET], "exchanges": sorted(scn["wallets"]), "holders": [HOLDER]}


def run_scenario(
    scn: dict[str, Any],
    cases: Cases,
    scratch: Path,
    *,
    check_entries: Callable[[int], bool] = lambda index: False,
    keep_text: bool = True,
) -> None:
    """Runs ``scn`` through RP2 and records a compute case, or a check_entry
    case for the first constructor that raises, plus check_entry cases for
    the rows ``check_entries`` selects."""
    conf = configuration(scn, scratch)
    sets = {kind: TransactionSet(conf, kind.upper(), ASSET) for kind in RP2_CLASSES}
    entries = []
    rows = ROW_OVERRIDES.get(scn["name"]) or list(range(1, len(scn["rows"]) + 1))
    for index, (row, row_number) in enumerate(zip(scn["rows"], rows)):
        kind, rp2_kwargs = constructor_arguments(row, row_number, RP2Decimal)
        _, native_kwargs = constructor_arguments(row, row_number, Decimal)
        entry = NATIVE_BUILDERS[kind](**native_kwargs)
        request = NR.check_entry_request(entry, **membership(scn))
        try:
            tx = RP2_CLASSES[kind](configuration=conf, **rp2_kwargs)
        except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
            cases.add(f"{scn['name']}/row{row_number}", "check_entry", request, error_document(exc))
            return
        check_timestamp(f"{scn['name']} row {row_number}", entry["ts"], tx.timestamp)
        derived = derived_values(kind, tx)
        text = str(tx)
        rendered = NR.entry_text(entry, derived)
        if rendered != text:
            raise SystemExit(f"{scn['name']} row {row_number}: entry_text differs from RP2:\n{rendered}\n---\n{text}")
        if check_entries(index):
            cases.add(
                f"{scn['name']}/row{row_number}",
                "check_entry",
                request,
                {"schema_version": 1, "ok": True, "entry": derived},
            )
        entries.append(NR.with_text(entry, text))
        sets[kind].add_entry(tx)
    country = NR.country_spec(scn["country"], scn["long_term_days"] if scn["country"] == "generic" else None)
    request = NR.compute_request(method=scn["method"], assets={ASSET: entries}, country=country)
    if sets["in"].count == 0:
        return  # the adapter does not compute an asset without inbound rows
    input_data = InputData(
        asset=ASSET,
        unfiltered_in_transaction_set=sets["in"],
        unfiltered_out_transaction_set=sets["out"],
        unfiltered_intra_transaction_set=sets["intra"],
    )
    try:
        expected = computed_document(compute_tax(conf, accounting_engine(scn["method"]), input_data))
    except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
        expected = error_document(exc)
    if not keep_text and not _embeds_entry_text(expected):
        # The engine reads `text` only for messages that embed it; dropping
        # it elsewhere keeps the fixture file within its size budget.
        for asset in request["assets"]:
            for entry in asset["entries"]:
                entry.pop("text", None)
    cases.add(scn["name"], "compute", request, expected)


def _embeds_entry_text(expected: dict[str, Any]) -> bool:
    message = expected.get("error", {}).get("message", "")
    return "went negative" in message or message.startswith("Entry already added")


# ---------------------------------------------------------------------------
# Constructor checks the histories do not reach


def check_cases(cases: Cases) -> None:
    """One case per constructor check and message, in RP2's check order."""
    T = "2024-01-01T00:00:00Z"
    good_in = dict(kind="in", unique_id="u", timestamp=T, exchange="A", transaction_type="BUY",
                   spot_price="100", crypto_in="1", fiat_in_no_fee="100", fiat_in_with_fee="100", fiat_fee="0")
    good_sell = dict(kind="out", unique_id="u", timestamp=T, exchange="A", transaction_type="SELL",
                     spot_price="100", crypto_out_no_fee="1", crypto_fee="0", fiat_out_no_fee="100", fiat_fee="0")
    good_fee = dict(kind="out", unique_id="u", timestamp=T, exchange="A", transaction_type="FEE",
                    spot_price="100", crypto_out_no_fee="0", crypto_fee="0.001", fiat_out_no_fee=None, fiat_fee="0.1")
    good_move = dict(kind="intra", unique_id="u", timestamp=T, from_exchange="A", to_exchange="B",
                     spot_price="100", crypto_sent="1", crypto_received="0.999")
    variants: list[tuple[str, dict[str, Any], dict[str, Any]]] = [
        ("in_ok_crypto_fee_without_fiat_fee", good_in, dict(crypto_fee="0.01", fiat_fee=None)),
        ("in_ok_zero_crypto_fee_without_fiat_fee", good_in, dict(crypto_fee="0", fiat_fee=None)),
        ("in_ok_derived_fiat", good_in, dict(fiat_in_no_fee=None, fiat_in_with_fee=None, fiat_fee="1.5")),
        ("in_ok_gift_and_donate", good_in, dict(transaction_type="gift")),
        ("in_ok_wages_lowercase", good_in, dict(transaction_type="wages")),
        ("in_ok_dust_fiat_6e14", good_in, dict(fiat_in_no_fee="6E-14", fiat_in_with_fee="6E-14")),
        ("in_ok_negative_dust_spot", good_in, dict(spot_price="-5E-14")),
        ("in_ok_unique_id_none", good_in, dict(unique_id=None, notes="")),
        ("in_unknown_asset", good_in, dict(asset="ETH")),
        ("in_no_timezone", good_in, dict(timestamp="2024-01-01T00:00:00")),
        ("in_date_only", good_in, dict(timestamp="2024-01-01")),
        ("in_unparseable_timestamp", good_in, dict(timestamp="not a date")),
        ("in_out_of_range_timestamp", good_in, dict(timestamp="2024-13-01T00:00:00Z")),
        ("in_invalid_type", good_in, dict(transaction_type="PURCHASE")),
        ("in_padded_type", good_in, dict(transaction_type=" BUY")),
        ("in_missing_type", good_in, dict(transaction_type=None)),
        ("in_missing_spot", good_in, dict(spot_price=None)),
        ("in_negative_spot", good_in, dict(spot_price="-1")),
        ("in_spot_dust_negative", good_in, dict(spot_price="-6E-14")),
        ("in_spot_huge", good_in, dict(spot_price="1E19")),
        ("in_unknown_exchange", good_in, dict(exchange="Z")),
        ("in_padded_exchange", good_in, dict(exchange=" A ")),
        ("in_missing_exchange", good_in, dict(exchange=None)),
        ("in_unknown_holder", good_in, dict(holder="Other")),
        ("in_missing_crypto_in", good_in, dict(crypto_in=None)),
        ("in_zero_crypto_in", good_in, dict(crypto_in="0")),
        ("in_dust_crypto_in", good_in, dict(crypto_in="5E-14")),
        ("in_negative_crypto_in", good_in, dict(crypto_in="-0.5")),
        ("in_negative_crypto_fee", good_in, dict(crypto_fee="-1", fiat_fee=None)),
        ("in_negative_fiat_fee", good_in, dict(fiat_fee="-1")),
        ("in_zero_spot", good_in, dict(spot_price="0")),
        ("in_dust_spot", good_in, dict(spot_price="4E-14")),
        ("in_both_fees", good_in, dict(crypto_fee="0.01", fiat_fee="1")),
        ("in_zero_fiat_in_no_fee", good_in, dict(fiat_in_no_fee="0")),
        ("in_dust_fiat_in_with_fee", good_in, dict(fiat_in_with_fee="5E-14")),
        ("in_sell_type", good_in, dict(transaction_type="SELL")),
        ("in_move_type", good_in, dict(transaction_type="move")),
        ("in_huge_fiat_mismatch", good_in, dict(crypto_in="1E18", spot_price="1E18", fiat_in_no_fee="1", fiat_in_with_fee="1")),
        ("sell_ok_derived_fiat", good_sell, dict(fiat_out_no_fee=None, fiat_fee=None, crypto_fee="0.25")),
        ("sell_ok_explicit_with_fee", good_sell, dict(crypto_out_with_fee="1.5")),
        ("sell_ok_lost_without_fiat", good_sell, dict(transaction_type="LOST", fiat_out_no_fee=None)),
        ("sell_ok_lost_with_fiat", good_sell, dict(transaction_type="lost")),
        ("sell_ok_staking_out", good_sell, dict(transaction_type="STAKING")),
        ("sell_ok_donate_and_gift", good_sell, dict(transaction_type="Gift")),
        ("sell_zero_spot", good_sell, dict(spot_price="0")),
        ("sell_zero_amount", good_sell, dict(crypto_out_no_fee="0")),
        ("sell_negative_amount", good_sell, dict(crypto_out_no_fee="-1")),
        ("sell_missing_fee", good_sell, dict(crypto_fee=None)),
        ("sell_negative_fee", good_sell, dict(crypto_fee="-0.1")),
        ("sell_zero_with_fee", good_sell, dict(crypto_out_with_fee="0")),
        ("sell_zero_fiat", good_sell, dict(fiat_out_no_fee="0")),
        ("sell_negative_fiat_fee", good_sell, dict(fiat_fee="-1")),
        ("sell_buy_type", good_sell, dict(transaction_type="BUY")),
        ("sell_income_type", good_sell, dict(transaction_type="income")),
        ("sell_unknown_holder", good_sell, dict(holder="Other")),
        ("fee_ok_zero_spot", good_fee, dict(spot_price="0", fiat_fee="0")),
        ("fee_ok_derived_fiat_fee", good_fee, dict(fiat_fee=None)),
        ("fee_nonzero_amount", good_fee, dict(crypto_out_no_fee="0.5")),
        ("fee_negative_amount", good_fee, dict(crypto_out_no_fee="-0.5")),
        ("fee_zero_fee", good_fee, dict(crypto_fee="0")),
        ("fee_missing_amount", good_fee, dict(crypto_out_no_fee=None)),
        ("move_ok_no_spot_no_fee", good_move, dict(spot_price=None, crypto_received="1")),
        ("move_ok_zero_spot_dust_fee", good_move, dict(spot_price="0", crypto_received="0.99999999999996")),
        ("move_ok_zero_received", good_move, dict(crypto_received="0")),
        ("move_ok_dust_fiat_fee", good_move, dict(spot_price="0.005", crypto_received="0.99999999999")),
        ("move_ok_same_account", good_move, dict(to_exchange="A")),
        ("move_fee_without_spot", good_move, dict(spot_price=None)),
        ("move_fee_with_zero_spot_unique_id_none", good_move, dict(spot_price="0E-3", unique_id=None)),
        ("move_zero_sent", good_move, dict(crypto_sent="0")),
        ("move_missing_sent", good_move, dict(crypto_sent=None)),
        ("move_negative_received", good_move, dict(crypto_received="-1")),
        ("move_sent_below_received", good_move, dict(crypto_received="1.5")),
        ("move_unknown_from", good_move, dict(from_exchange="Z")),
        ("move_unknown_to_holder", good_move, dict(to_holder="Other")),
        ("move_unknown_asset", good_move, dict(asset="ETH")),
        ("move_no_timezone", good_move, dict(timestamp="2024-01-01 00:00:00")),
        ("move_negative_spot", good_move, dict(spot_price="-2")),
    ]
    conf = None
    with tempfile.TemporaryDirectory(prefix="kassiber-engine-fixtures-") as scratch:
        conf = configuration(scenario("checks", "fifo", ["A", "B"], []), Path(scratch))
    names = {"assets": [ASSET], "exchanges": ["A", "B"], "holders": [HOLDER]}
    for row_number, (name, base, changes) in enumerate(variants, start=1):
        row = {**base, **changes}
        kind = row.pop("kind")
        rp2_kwargs = {"row": row_number}
        native_kwargs = {"row": row_number}
        for key, value in row.items():
            rp2_kwargs[key] = RP2Decimal(value) if key in DECIMAL_KEYS and value is not None else value
            native_kwargs[key] = Decimal(value) if key in DECIMAL_KEYS and value is not None else value
        for target in (rp2_kwargs, native_kwargs):
            target.setdefault("asset", ASSET)
            if kind == "intra":
                target.setdefault("from_holder", HOLDER)
                target.setdefault("to_holder", HOLDER)
            else:
                target.setdefault("holder", HOLDER)
        entry = NATIVE_BUILDERS[kind](**native_kwargs)
        request = NR.check_entry_request(entry, **names)
        try:
            tx = RP2_CLASSES[kind](configuration=conf, **rp2_kwargs)
        except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
            cases.add(f"check/{name}", "check_entry", request, error_document(exc))
            continue
        check_timestamp(f"check/{name}", entry["ts"], tx.timestamp)
        derived = derived_values(kind, tx)
        if NR.entry_text(entry, derived) != str(tx):
            raise SystemExit(f"check/{name}: entry_text differs from RP2:\n{NR.entry_text(entry, derived)}\n---\n{tx}")
        cases.add(f"check/{name}", "check_entry", request, {"schema_version": 1, "ok": True, "entry": derived})


# ---------------------------------------------------------------------------
# Random histories


def msat_to_btc(msat: int) -> str:
    with localcontext(CTX):
        return str(Decimal(int(msat)) / Decimal(100_000_000_000))


def random_price(rng: random.Random) -> str:
    digits = rng.choice([0, 0, 2, 2, 4, 8])
    whole = rng.choice([1, 50, 999, 16000, 43210, 98765])
    if not digits:
        return str(whole)
    return f"{whole}.{rng.randrange(10**digits):0{digits}d}"


def random_fiat(rng: random.Random, amount: str, spot: str) -> str:
    if rng.random() < 0.5:
        return kassiber_product(amount, spot)
    return str(Decimal(rng.randint(1, 10**7)) / Decimal(100))


OFFSETS = [timedelta(hours=1), timedelta(hours=2), timedelta(hours=-5), timedelta(hours=5, minutes=30)]


def stamp(rng: random.Random, instant: datetime, style_weights: tuple[int, int, int]) -> str:
    """Kassiber's canonical ``...Z`` form, or (rarely) an offset or fraction."""
    style = rng.choices(["z", "offset", "micro"], style_weights)[0]
    if style == "offset":
        return instant.astimezone(timezone(rng.choice(OFFSETS))).isoformat()
    if style == "micro":
        instant = instant + timedelta(microseconds=rng.choice([1, 250000, 999999]))
        return instant.strftime("%Y-%m-%dT%H:%M:%S.%fZ")
    return instant.strftime("%Y-%m-%dT%H:%M:%SZ")


def instant_pool(rng: random.Random, long_term_days: int) -> list[datetime]:
    """Instants for one history: shared instants (ties), holding-period
    boundaries around the first instant, and a year boundary."""
    start = datetime(rng.choice([2019, 2021, 2023]), rng.randint(1, 12), rng.randint(1, 28), rng.choice([0, 12, 23]), tzinfo=timezone.utc)
    pool = [start + timedelta(days=rng.randint(0, 900), hours=rng.choice([0, 6, 23])) for _ in range(rng.randint(3, 9))]
    pool.append(start)
    boundary = start + timedelta(days=long_term_days)
    pool += [boundary - timedelta(seconds=1), boundary, boundary + timedelta(seconds=1)]
    pool.append(datetime(start.year, 12, 31, 23, 59, 59, tzinfo=timezone.utc))
    if rng.random() < 0.04:
        pool.append(datetime(1969, 12, 31, 12, tzinfo=timezone.utc))
    return sorted(set(pool))


def random_history(index: int, seed: int = SEED) -> dict[str, Any]:
    rng = random.Random(seed * 1000 + index)
    method = METHODS[index % len(METHODS)]
    country = "at" if rng.random() < 0.12 else "generic"
    long_term_days = rng.choice([0, 1, 30, 365, 365, 366, 730])
    wallets = rng.choice([["A"], ["A", "B"], ["A", "B", "C"], ["A", "B", "C"], ["A", "A-b", "A_x", "a"]])
    profile = rng.choices(["gated", "ungated", "neartie"], [70, 18, 12])[0]
    if profile == "neartie" and method in ("fifo", "lifo"):
        profile = "gated"
    instants = instant_pool(rng, long_term_days)
    styles = (90, 6, 4) if rng.random() < 0.7 else (100, 0, 0)
    if profile == "neartie":
        rows = neartie_rows(rng, wallets, instants)
    else:
        rows = flow_rows(rng, wallets, instants, styles, gated=profile == "gated")
    return scenario(f"R{index:03d}_{method}_{profile}", method, wallets, rows, long_term_days=long_term_days, country=country)


def neartie_rows(rng: random.Random, wallets: list[str], instants: list[datetime]) -> list[dict[str, Any]]:
    """HIFO/LOFO lots whose unit costs differ by multiples of 3E-14, so the
    13-place comparison is not transitive and heap order decides."""
    rows = []
    lot_instants = instants[: max(2, len(instants) // 2)]
    total = 0
    for i in range(rng.randint(3, 9)):
        fiat = Decimal(100) + Decimal(rng.randint(0, 6)) * Decimal("3E-14")
        amount = rng.choice(["1", "0.5", "2"])
        total += int(Decimal(amount) * 10)
        rows.append(buy(f"b{i}", stamp(rng, rng.choice(lot_instants), (100, 0, 0)), wallets[0], "100", amount, str(fiat * Decimal(amount))))
    rows.sort(key=lambda r: r["timestamp"])
    later = [t for t in instants if t > max(lot_instants)] or [max(lot_instants) + timedelta(days=1)]
    left = total
    for i, instant in enumerate(later[:4]):
        if left <= 0:
            break
        amount = min(left, rng.randint(1, 15))
        rows.append(sell(f"s{i}", stamp(rng, instant, (100, 0, 0)), wallets[0], str(Decimal(amount) / Decimal(10)), "200", "100"))
        left -= amount
    return rows


def flow_rows(
    rng: random.Random,
    wallets: list[str],
    instants: list[datetime],
    styles: tuple[int, int, int],
    *,
    gated: bool,
) -> list[dict[str, Any]]:
    """Buys, earn receipts, sales (with and without fees), fee-only and
    channel-open rows, and moves. Gated histories keep every wallet and the
    lot total non-negative in emission order, as the adapter's gate does."""
    n = rng.randint(3, 12)
    raw = []
    for i in range(n):
        instant = rng.choice(instants[: max(2, len(instants) * (i + 1) // n)])
        kind = rng.choices(["in", "out", "intra"], [5, 4, 2])[0]
        raw.append((stamp(rng, instant, styles), kind, i))
    if gated or rng.random() < 0.5:
        rank = {"in": 0, "intra": 1, "out": 2}
        raw.sort(key=lambda r: (r[0], rank[r[1]], r[2]))
    else:
        rng.shuffle(raw)
    balance = {w: 0 for w in wallets}
    lots = 0
    rows = []
    for ts, kind, i in raw:
        wallet = rng.choice(wallets)
        spot = random_price(rng)
        uid = f"{kind}{i}"
        if kind == "in":
            msat = rng.choice([rng.randint(1, 10**11), rng.randint(1, 10**8) * 1000, 10**11, 5 * 10**10])
            if rng.random() < 0.01:
                msat = 0  # a priced zero-amount inbound: RP2 rejects it
            amount = msat_to_btc(msat)
            type_ = rng.choice(["BUY"] * 5 + list(EARN_TYPES))
            rows.append(buy(uid, ts, wallet, spot, amount, random_fiat(rng, amount, spot), type_, notes=rng.choice(["", "", "Exchange buy"])))
            balance[wallet] += msat
            lots += msat
        elif kind == "out":
            available = balance[wallet] if gated else rng.randint(1, 15 * 10**10)
            if available <= 0:
                continue
            shape = rng.choice(["sell", "sell", "sellfee", "feeonly", "channel"])
            if shape in ("feeonly", "channel"):
                fee = rng.randint(1, min(available, 10**8))
                needed = fee
                rows.append(fee_out(uid, ts, wallet, msat_to_btc(fee), spot))
            else:
                total = rng.randint(1, available)
                fee = rng.randint(1, min(total - 1, 10**7)) if shape == "sellfee" and total > 1 else 0
                needed = total
                amount = msat_to_btc(total - fee)
                fiat = random_fiat(rng, amount, spot) if rng.random() < 0.97 else "1E-14"
                rows.append(sell(uid, ts, wallet, amount, spot, fiat, fee=msat_to_btc(fee)))
            if gated and (balance[wallet] < needed or lots < needed):
                rows.pop()
                continue
            balance[wallet] -= needed
            lots -= needed
        else:
            others = [w for w in wallets if w != wallet] or [wallet]
            to = rng.choice(others)
            available = balance[wallet] if gated else rng.randint(1, 10**11)
            if available <= 0:
                continue
            sent = rng.randint(1, available)
            fee = rng.randint(0, min(sent, 10**7)) if rng.random() < 0.7 else 0
            if rng.random() < 0.05:
                fee = 1  # a 1-msat fee: dust fiat at small prices
            spot_text = "0" if fee == 0 and rng.random() < 0.5 else spot
            if fee > 0 and rng.random() < 0.02:
                spot_text = "0"  # RP2 rejects a fee without a price
            rows.append(move(uid, ts, wallet, to, msat_to_btc(sent), msat_to_btc(sent - fee), spot_text))
            balance[wallet] -= sent
            balance[to] += sent - fee
            lots -= fee
    if gated and rng.random() < 0.1:
        # Overdraw one wallet by a few msat while other wallets hold the
        # lots: RP2 tolerates up to 5 msat (10-place rounding), not 6.
        wallet = rng.choice(wallets)
        overdraft = rng.randint(1, 7)
        if lots >= balance[wallet] + overdraft and balance[wallet] + overdraft > 0:
            amount = msat_to_btc(balance[wallet] + overdraft)
            last = stamp(rng, max(instants) + timedelta(days=1), (100, 0, 0))
            rows.append(sell("overdraft", last, wallet, amount, "100", kassiber_product(amount, "100")))
    if not any(r["kind"] == "in" for r in rows):
        rows.insert(0, buy("seed", "2018-01-01T00:00:00Z", wallets[0], "100", "1", "100"))
    return rows


# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--histories", type=int, default=RANDOM_HISTORIES)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--max-bytes", type=int, default=MAX_BYTES, help="size budget (0 disables it)")
    args = parser.parse_args()

    context = decimal.getcontext()
    if context.prec != 32 or context.rounding != ROUND_HALF_EVEN:
        raise SystemExit(f"expected RP2's 32-digit context on the main thread, found {context}")

    cases = Cases()
    with tempfile.TemporaryDirectory(prefix="kassiber-engine-fixtures-") as scratch:
        scratch_path = Path(scratch)
        for scn in B_SCENARIOS + EXTRA_SCENARIOS:
            run_scenario(scn, cases, scratch_path, check_entries=lambda index: True)
        check_cases(cases)
        for index in range(args.histories):
            run_scenario(
                random_history(index, args.seed),
                cases,
                scratch_path,
                check_entries=lambda i, k=index: k % 12 == 0 and i < 3,
                keep_text=False,
            )

    document = {
        "schema": "kassiber-tax-engine/generic-fixtures/v1",
        "generator": "tax-engine/scripts/gen_engine_fixtures.py",
        "seed": args.seed,
        "notes": [
            "Each case's expected value is RP2's response for the request, run as the adapter drives it.",
            "Decimals use the canonical form [-]<digits>E<exponent>; exponents are significant.",
        ],
        "stats": dict(sorted(cases.stats.items())),
        "cases": cases.cases,
    }
    text = json.dumps(document, ensure_ascii=False, separators=(",", ":")) + "\n"
    if args.max_bytes and len(text.encode()) > args.max_bytes:
        raise SystemExit(f"fixtures are {len(text.encode())} bytes, over the {args.max_bytes} byte budget")
    args.output.write_text(text, encoding="utf-8")
    print(f"wrote {len(cases.cases)} cases ({len(text.encode())} bytes) to {args.output}: {document['stats']}")


if __name__ == "__main__":
    main()
