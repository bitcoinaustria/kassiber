#!/usr/bin/env python3
"""Generate RP2-oracle fixtures for the native tax engine.

Each case is an engine request built with ``kassiber/core/engines/
native_request.py`` plus the response RP2 gives for the same input, run as
Kassiber's adapter (``kassiber/core/engines/rp2.py``) drives it. Two files
are written:

``generic_fixtures.json`` (the lot methods):

* ``compute`` cases run ``rp2.tax_engine.compute_tax`` for one asset with
  fifo, lifo, hifo, or lofo under the generic or Austrian country, and
  record every ``ComputedData`` value the adapter reads, or the error.
  Austrian rows also carry ``classify_disposal``'s category or error.
* ``check_entry`` cases run one transaction constructor and record the
  values it stores and derives, or the error it raises.

``at_fixtures.json`` (moving averages and Austria): each scenario is one
adapter run over every asset of a book. RP2's ``validate_input_data``, then
``compute_tax_for_assets``, then (when that declines) ``compute_tax`` per
asset become the engine's ``validate``, ``compute_multi``, and ``compute``
steps; a failing step ends the run, as it aborts the adapter. It holds every
scenario of the area-C black-box spec, hand-written edge probes, and seeded
random histories: the generic moving average; Alt, Neu, mixed, and unmarked
routing; Wahlrecht-tagged sales; swaps (chains, same-instant legs, direct
payouts); fees and earn types; cutoff and Spekulationsfrist boundaries;
pools; lot methods on Austrian books; and injected marker, pairing, and
routing errors.

The inputs are the adapter's constructor arguments: inbound rows with
``fiat_in_no_fee == fiat_in_with_fee`` and a zero fiat fee, SELL or FEE
outbound rows with ``fiat_fee = fee * spot`` computed in Kassiber's 32-digit
context, and moves with ``spot_price`` 0 when unknown. The generic histories
cover every lot method, partial lots, fees, earn types, moves, same-instant
ties, near-tie unit costs, holding-period boundaries, offsets and
microseconds, pre-1970 rows, and error cases. Every entry's ``text`` is
checked against RP2's own ``str(transaction)``; random histories keep it
only where an expected message embeds it, to stay within the size budget.

RP2 is imported on the main thread, which sets that thread's decimal context
to 32 digits; every value here is computed on that thread. Run it from a
scratch directory (RP2 may create a log directory in the working directory):

    cd "$(mktemp -d)" && uv run --locked --project <repo> \\
        python <repo>/tax-engine/scripts/gen_engine_fixtures.py

It writes both files under tax-engine/core/tests/data/ (``--suite generic``
or ``--suite at`` writes one). The output depends only on the seed and the
pinned RP2. For a stress run, write a larger set elsewhere and point the
Rust test at it:

    ... gen_engine_fixtures.py --suite generic --seed 7 --histories 5000 \\
        --max-bytes 0 --output /tmp/engine_stress.json
    KASSIBER_ENGINE_FIXTURES=/tmp/engine_stress.json \\
        cargo test -p kassiber-tax-core --test engine_fixtures
    ... gen_engine_fixtures.py --suite at --seed 7 --at-histories 3000 \\
        --at-max-bytes 0 --at-output /tmp/at_stress.json
    KASSIBER_AT_FIXTURES=/tmp/at_stress.json \\
        cargo test -p kassiber-tax-core --test engine_fixtures
"""

from __future__ import annotations

import argparse
import decimal
import json
import random
import re
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

AT_MODULE = import_module("rp2.plugin.country.at")

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


def country_object(country: str, long_term_days: int) -> AbstractCountry:
    return AT_MODULE.AT() if country == "at" else _Country(long_term_days)


def rp2_configuration(country: AbstractCountry, assets: list[str], wallets: list[str], holder: str, scratch: Path) -> Configuration:
    path = scratch / "rp2.ini"
    path.write_text(
        INI_TEMPLATE.format(assets=", ".join(sorted(assets)), exchanges=", ".join(sorted(wallets)), holder=holder),
        encoding="utf-8",
    )
    try:
        return Configuration(str(path), country)
    finally:
        path.unlink()


def configuration(scn: dict[str, Any], scratch: Path) -> Configuration:
    country = country_object(scn["country"], scn["long_term_days"])
    return rp2_configuration(country, [ASSET], scn["wallets"], HOLDER, scratch)


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
        "ValueError": "ValueError",
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


def classification(gain_loss: Any) -> dict[str, Any]:
    """``classify_disposal`` as the adapter calls it for every row of an
    Austrian book: the category, or the error it raises."""
    try:
        return {"at_category": AT_MODULE.classify_disposal(gain_loss).value}
    except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
        return {"at_category_error": error_document(exc)["error"]}


def asset_document(computed: Any, classify: bool = False) -> dict[str, Any]:
    """Every ``ComputedData`` value the adapter reads, in the engine's form,
    with the Austrian classification of each row when ``classify``."""
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
            **(classification(gl) if classify else {}),
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
    return {
        "asset": computed.asset,
        "in_transactions": [tx.row for tx in lots],
        "in_fiat_in_with_fee": [enc(computed.get_in_transaction_fiat_in_with_fee(tx)) for tx in lots],
        "gain_losses": gain_losses,
        "yearly": yearly,
        "open_positions": open_positions,
        "balances": balances,
    }


def computed_document(computed: Any, classify: bool = False) -> dict[str, Any]:
    """A successful ``compute`` response for one asset."""
    return {"schema_version": 1, "ok": True, "assets": [asset_document(computed, classify)]}


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
        computed = compute_tax(conf, accounting_engine(scn["method"]), input_data)
        expected = computed_document(computed, classify=scn["country"] == "at")
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
# Austrian and moving-average fixtures (at_fixtures.json)
#
# Each scenario is one adapter run over every asset of a book: RP2's
# `validate_input_data`, then `compute_tax_for_assets`, then (when that
# declines) `compute_tax` per asset, recorded as the engine's `validate`,
# `compute_multi`, and `compute` steps. Austrian rows also carry the
# `classify_disposal` result the adapter computes for each of them.

AT_OUTPUT = REPO / "tax-engine" / "core" / "tests" / "data" / "at_fixtures.json"
AT_RANDOM_HISTORIES = 420
AT_MAX_BYTES = 3_100_000
AT_HOLDER = "H"
VIENNA = ZoneInfo("Europe/Vienna")
NEU_CUTOFF = datetime(2021, 3, 1, tzinfo=VIENNA)


def at_in(uid, asset, ts, amount, spot, fiat, notes="", type_="BUY", wallet="W1"):
    """An inbound row: a lot, taxable at receipt when earn-typed."""
    return {"kind": "in", "id": uid, "asset": asset, "ts": ts, "amount": amount, "spot": spot, "fiat": fiat,
            "notes": notes, "type": type_, "wallet": wallet}


def at_out(uid, asset, ts, amount, fee, spot, fiat, notes="", wallet="W1"):
    """An outbound row: a SELL, or a FEE row when the amount is zero."""
    return {"kind": "out", "id": uid, "asset": asset, "ts": ts, "amount": amount, "fee": fee, "spot": spot,
            "fiat": fiat, "notes": notes, "wallet": wallet}


def at_move(uid, asset, ts, sent, received, spot, src, dst, notes=""):
    """A transfer between two of the profile's wallets."""
    return {"kind": "move", "id": uid, "asset": asset, "ts": ts, "sent": sent, "received": received, "spot": spot,
            "from": src, "to": dst, "notes": notes}


def at_scenario(name, method, txs, *, country="at", long_term_days=365):
    return {"name": name, "method": method, "country": country, "long_term_days": long_term_days, "txs": txs}


# Every scenario of the black-box spec for moving averages and Austria
# (area C), in emission order: rows are numbered per asset from 1.
C_SCENARIOS = [
    at_scenario('S01_alt_only_fifo_spekulationsfrist', 'moving_average_at', [
        at_in('a1', 'BTC', '2019-05-10T08:00:00Z', '0.4', '5000', '2000', 'at_regime=alt at_pool=default'),
        at_in('a2', 'BTC', '2020-11-20T08:00:00Z', '0.6', '15000', '9000.30', 'at_regime=alt at_pool=otherpool'),
        at_out('d1', 'BTC', '2021-06-01T08:00:00Z', '0.5', '0.0002', '30000', '15000', 'at_regime=alt at_pool=default'),
        at_out('d2', 'BTC', '2021-11-20T22:59:59Z', '0.1', '0', '50000', '5000', 'at_regime=alt at_pool=default'),
        at_out('d3', 'BTC', '2021-11-20T23:00:00Z', '0.1', '0', '50000', '5000', 'at_regime=alt at_pool=default'),
    ]),
    at_scenario('S02_neu_only_moving_average_fee_gain_loss', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T10:00:00Z', '0.3', '40000', '12000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-02-01T10:00:00Z', '0.7', '35000', '24500.01', 'at_regime=neu at_pool=default'),
        at_in('b3', 'BTC', '2022-02-02T10:00:00Z', '0.11', '33333.33', '3666.67', 'at_regime=neu at_pool=default'),
        at_out('s1', 'BTC', '2022-03-01T10:00:00Z', '0.5', '0.0001', '45000', '22500', 'at_regime=neu at_pool=default'),
        at_out('s2', 'BTC', '2022-04-01T10:00:00Z', '0.2', '0', '30000', '6000', 'at_regime=neu at_pool=default'),
        at_in('b4', 'BTC', '2022-05-01T10:00:00Z', '0.07', '29000', '2030.03', 'at_regime=neu at_pool=default'),
        at_out('s3', 'BTC', '2022-06-01T10:00:00Z', '0.33', '0.00005', '20000', '6600', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S03_mixed_explicit_wahlrecht', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-10T08:00:00Z', '0.5', '8000', '4000', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2021-04-01T08:00:00Z', '0.5', '50000', '25000', 'at_regime=neu at_pool=default'),
        at_in('n2', 'BTC', '2021-05-01T08:00:00Z', '0.25', '40000', '10000', 'at_regime=neu at_pool=default'),
        at_out('dn', 'BTC', '2021-06-01T08:00:00Z', '0.6', '0', '30000', '18000', 'at_regime=neu at_regime_basis=wahlrecht at_pool=default Kassiber Neu-first designation'),
        at_out('da', 'BTC', '2021-07-01T08:00:00Z', '0.2', '0', '32000', '6400', 'at_regime=alt at_pool=default'),
        at_out('da2', 'BTC', '2021-07-02T08:00:00Z', '0.1', '0', '32000', '3200', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S04_mixed_untagged_disposal_is_ambiguous', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-10T08:00:00Z', '0.5', '8000', '4000', ''),
        at_in('n1', 'BTC', '2021-04-01T08:00:00Z', '0.5', '50000', '25000', ''),
        at_out('d1', 'BTC', '2021-06-01T08:00:00Z', '0.1', '0', '30000', '3000', ''),
    ]),
    at_scenario('S05_untagged_routing_by_availability', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-10T08:00:00Z', '0.3', '8000', '2400', ''),
        at_out('d1', 'BTC', '2020-06-01T08:00:00Z', '0.1', '0', '9000', '900', ''),
        at_out('d2', 'BTC', '2021-03-05T08:00:00Z', '0.2', '0', '45000', '9000', ''),
        at_in('n1', 'BTC', '2021-04-01T08:00:00Z', '0.4', '50000', '20000', ''),
        at_out('d3', 'BTC', '2021-05-01T08:00:00Z', '0.1', '0', '55000', '5500', ''),
    ]),
    at_scenario('S06_cutoff_boundary_untagged_lots', 'moving_average_at', [
        at_in('last_alt', 'BTC', '2021-02-28T22:59:59.999999Z', '0.2', '40000', '8000', ''),
        at_in('first_neu', 'BTC', '2021-02-28T23:00:00Z', '0.3', '40000', '12000', ''),
        at_in('neu_offset', 'BTC', '2021-03-01T00:00:00+01:00', '0.1', '40000', '4000', ''),
        at_out('dn', 'BTC', '2021-03-02T08:00:00Z', '0.35', '0', '41000', '14350', 'at_regime=neu'),
        at_out('da', 'BTC', '2021-03-02T09:00:00Z', '0.2', '0', '41000', '8200', 'at_regime=alt'),
    ]),
    at_scenario('S07_neu_pools_separate_averages', 'moving_average_at', [
        at_in('pa1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=A'),
        at_in('pb1', 'BTC', '2022-01-02T00:00:00Z', '1', '40000', '40000', 'at_regime=neu at_pool=B'),
        at_in('pa2', 'BTC', '2022-01-03T00:00:00Z', '2', '33000', '66000', 'at_regime=neu at_pool=A'),
        at_in('pd1', 'BTC', '2022-01-03T12:00:00Z', '0.5', '35000', '17500', 'at_regime=neu'),
        at_out('sa', 'BTC', '2022-02-01T00:00:00Z', '1.5', '0', '35000', '52500', 'at_regime=neu at_pool=A'),
        at_out('sb', 'BTC', '2022-02-02T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=B'),
        at_out('sd', 'BTC', '2022-02-03T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu'),
    ]),
    at_scenario('S07b_neu_pool_mismatch_error', 'moving_average_at', [
        at_in('pa1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=A'),
        at_out('sb', 'BTC', '2022-02-02T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=B'),
    ]),
    at_scenario('S08_earn_types_neu', 'moving_average_at', [
        at_in('buy', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('stk', 'BTC', '2022-01-05T00:00:00Z', '0.003', '31234.567', '93.703701', 'at_regime=neu at_pool=default', type_='STAKING'),
        at_in('int', 'BTC', '2022-01-06T00:00:00Z', '0.0007', '31000', '21.7', 'at_regime=neu at_pool=default', type_='INTEREST'),
        at_in('min', 'BTC', '2022-01-07T00:00:00Z', '0.01', '32000', '320', 'at_regime=neu at_pool=default', type_='MINING'),
        at_in('inc', 'BTC', '2022-01-08T00:00:00Z', '0.0003', '33333.3333', '9.99999999', 'at_regime=neu at_pool=default', type_='INCOME'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '1.01', '0', '35000', '35350', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S08b_earn_alt_regime', 'moving_average_at', [
        at_in('buy', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', 'at_regime=alt at_pool=default'),
        at_in('stk', 'BTC', '2020-03-05T00:00:00Z', '0.01', '8000', '80', 'at_regime=alt at_pool=default', type_='STAKING'),
        at_out('s1', 'BTC', '2021-01-01T00:00:00Z', '1.005', '0', '25000', '25125', 'at_regime=alt at_pool=default'),
    ]),
    at_scenario('S09_transfer_fee_and_fee_type_out', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.5', '36000', '18000', 'at_regime=neu at_pool=default'),
        at_move('m1', 'BTC', '2022-01-10T00:00:00Z', '0.4', '0.39985', '40000', 'W1', 'W2', 'at_regime=neu at_pool=default self-transfer proven by address ownership'),
        at_out('f1', 'BTC', '2022-01-11T00:00:00Z', '0', '0.0001', '41000', None, 'at_regime=neu at_pool=default'),
        at_move('m0', 'BTC', '2022-01-12T00:00:00Z', '0.1', '0.1', '0', 'W2', 'W1', 'at_regime=neu at_pool=default self-transfer proven by address ownership'),
        at_out('s1', 'BTC', '2022-01-20T00:00:00Z', '0.25', '0', '42000', '10500', 'at_regime=neu at_pool=default', wallet='W2'),
    ]),
    at_scenario('S09b_transfer_fee_untagged_mixed_is_ambiguous', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', ''),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', ''),
        at_move('m1', 'BTC', '2022-01-10T00:00:00Z', '0.4', '0.39985', '40000', 'W1', 'W2', 'self-transfer proven by address ownership'),
    ]),
    at_scenario('S10_swap_neu_carry_with_fee', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.4', '30000', '12000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.6', '33333.33', '19999.998', 'at_regime=neu at_pool=default'),
        at_out('sw_out', 'BTC', '2022-03-01T00:00:00Z', '0.5', '0.00003', '40000', '20000', 'at_regime=neu at_pool=default at_swap_link=p1 Swap to LBTC'),
        at_in('l0', 'LBTC', '2022-02-01T00:00:00Z', '0.1', '38000', '3800', 'at_regime=neu at_pool=default'),
        at_in('sw_in', 'LBTC', '2022-03-01T00:00:10Z', '0.49990', '40000', '19996', 'at_regime=neu at_pool=default at_swap_link=p1'),
        at_out('ls1', 'LBTC', '2022-04-01T00:00:00Z', '0.3', '0', '45000', '13500', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S11_swap_chain_three_assets_with_sale_between', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '1', '31000.07', '31000.07', 'at_regime=neu at_pool=default'),
        at_out('x1_out', 'BTC', '2022-02-01T00:00:00Z', '0.7', '0', '35000', '24500', 'at_regime=neu at_pool=default at_swap_link=x1'),
        at_in('l0', 'LBTC', '2022-01-15T00:00:00Z', '0.2', '33000', '6600', 'at_regime=neu at_pool=default'),
        at_out('ls_before', 'LBTC', '2022-01-20T00:00:00Z', '0.05', '0', '34000', '1700', 'at_regime=neu at_pool=default'),
        at_in('x1_in', 'LBTC', '2022-02-01T01:00:00Z', '0.7', '35000', '24500', 'at_regime=neu at_pool=default at_swap_link=x1'),
        at_out('ls_mid', 'LBTC', '2022-02-05T00:00:00Z', '0.1', '0', '36000', '3600', 'at_regime=neu at_pool=default'),
        at_out('x2_out', 'LBTC', '2022-03-01T00:00:00Z', '0.5', '0', '37000', '18500', 'at_regime=neu at_pool=default at_swap_link=x2'),
        at_in('u0', 'USDT', '2022-01-01T00:00:00Z', '100', '0.9', '90', 'at_regime=neu at_pool=default'),
        at_in('x2_in', 'USDT', '2022-03-01T00:00:00Z', '18500', '1', '18500', 'at_regime=neu at_pool=default at_swap_link=x2'),
        at_out('us1', 'USDT', '2022-04-01T00:00:00Z', '10000', '0', '0.95', '9500', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S12_swap_same_timestamp_chain', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('y1_out', 'BTC', '2022-02-01T00:00:00Z', '0.4', '0', '35000', '14000', 'at_regime=neu at_pool=default at_swap_link=y1'),
        at_in('y1_in', 'LBTC', '2022-02-01T00:00:00Z', '0.4', '35000', '14000', 'at_regime=neu at_pool=default at_swap_link=y1'),
        at_out('y2_out', 'LBTC', '2022-02-01T00:00:00Z', '0.4', '0', '35000', '14000', 'at_regime=neu at_pool=default at_swap_link=y2'),
        at_in('y2_in', 'USDT', '2022-02-01T00:00:00Z', '14000', '1', '14000', 'at_regime=neu at_pool=default at_swap_link=y2'),
    ]),
    at_scenario('S13_swap_alt_tagged_both_legs_realizes', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', 'at_regime=alt at_pool=default'),
        at_out('z_out', 'BTC', '2020-06-01T00:00:00Z', '0.5', '0', '9000', '4500', 'at_regime=alt at_pool=default at_swap_link=z'),
        at_in('z_in', 'ETH', '2020-06-01T00:00:00Z', '20', '225', '4500', 'at_regime=alt at_pool=default at_swap_link=z'),
    ]),
    at_scenario('S13b_swap_unmarked_regime_routed_to_alt_no_carry', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', ''),
        at_out('z_out', 'BTC', '2021-06-01T00:00:00Z', '0.5', '0', '40000', '20000', 'at_swap_link=z'),
        at_in('z_in', 'ETH', '2021-06-01T00:00:00Z', '8', '2500', '20000', 'at_swap_link=z'),
        at_out('e1', 'ETH', '2021-07-01T00:00:00Z', '1', '0', '2000', '2000', ''),
    ]),
    at_scenario('S14a_swap_unpaired_out', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=orphan'),
    ]),
    at_scenario('S14b_swap_same_asset', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=same'),
        at_in('i1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=same'),
    ]),
    at_scenario('S14c_swap_incoming_earlier', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=early'),
        at_in('i1', 'LBTC', '2022-01-31T23:59:59Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=early'),
    ]),
    at_scenario('S14d_swap_empty_id', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_swap_link='),
    ]),
    at_scenario('S14e_swap_cycle_same_timestamp', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('l1', 'LBTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('c1_out', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=c1'),
        at_in('c1_in', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=c1'),
        at_out('c2_out', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=c2'),
        at_in('c2_in', 'BTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=c2'),
    ]),
    at_scenario('S14f_swap_marker_on_fee_type_out_neu', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('f1', 'BTC', '2022-02-01T00:00:00Z', '0', '0.001', '35000', None, 'at_regime=neu at_pool=default at_swap_link=fee'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.001', '35000', '35', 'at_regime=neu at_pool=default at_swap_link=fee'),
    ]),
    at_scenario('S14g_swap_with_fifo_method', 'fifo', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
    ]),
    at_scenario('S14h_swap_with_plain_moving_average', 'moving_average', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
    ]),
    at_scenario('S14i_swap_cycle_different_times', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('l1', 'LBTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('k1_out', 'LBTC', '2022-02-01T00:00:00Z', '0.2', '0', '35000', '7000', 'at_regime=neu at_pool=default at_swap_link=k1'),
        at_in('k1_in', 'BTC', '2022-02-01T00:00:00Z', '0.2', '35000', '7000', 'at_regime=neu at_pool=default at_swap_link=k1'),
        at_out('k2_out', 'BTC', '2022-03-01T00:00:00Z', '0.5', '0', '36000', '18000', 'at_regime=neu at_pool=default at_swap_link=k2'),
        at_in('k2_in', 'LBTC', '2022-03-01T00:00:00Z', '0.5', '36000', '18000', 'at_regime=neu at_pool=default at_swap_link=k2'),
    ]),
    at_scenario('S15_generic_moving_average', 'moving_average', [
        at_in('g1', 'BTC', '2021-01-01T00:00:00Z', '0.3', '29000', '8700', 'Exchange buy'),
        at_in('g2', 'BTC', '2021-02-01T00:00:00Z', '0.9', '33000.01', '29700.009', 'at_regime=neu at_pool=default'),
        at_in('g3', 'BTC', '2021-02-15T00:00:00Z', '0.01', '48000', '480', 'at_regime=neu at_pool=default', type_='STAKING'),
        at_out('gs1', 'BTC', '2021-06-01T00:00:00Z', '0.6', '0.0004', '36000', '21600', 'at_regime=neu at_pool=default'),
        at_move('gm1', 'BTC', '2021-07-01T00:00:00Z', '0.2', '0.19993', '34000', 'W1', 'W2', 'self-transfer'),
        at_out('gs2', 'BTC', '2022-03-01T00:00:00Z', '0.4096', '0', '44000', '18022.4', 'at_regime=neu at_pool=default'),
        at_out('gs3', 'BTC', '2022-03-02T00:00:00Z', '0.19993', '0', '44000', '8796.92', 'at_regime=neu at_pool=default', wallet='W2'),
        at_in('g4', 'BTC', '2022-04-01T00:00:00Z', '0.5', '45000', '22500', 'at_regime=neu at_pool=default'),
        at_out('gs4', 'BTC', '2023-04-02T00:00:00Z', '0.25', '0', '28000', '7000', 'at_regime=neu at_pool=default'),
    ], country='generic'),
    at_scenario('S16_neu_pool_full_depletion_residual', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '10000', '3000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.7', '10000.01', '7000.007', 'at_regime=neu at_pool=default'),
        at_in('b3', 'BTC', '2022-01-03T00:00:00Z', '0.33', '10000', '3300.01', 'at_regime=neu at_pool=default'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.11', '0', '12000', '1320', 'at_regime=neu at_pool=default'),
        at_out('s2', 'BTC', '2022-02-02T00:00:00Z', '1.22', '0', '12000', '14640', 'at_regime=neu at_pool=default'),
        at_in('b4', 'BTC', '2022-03-01T00:00:00Z', '0.2', '20000', '4000', 'at_regime=neu at_pool=default'),
        at_out('s3', 'BTC', '2022-04-01T00:00:00Z', '0.2', '0', '20000', '4000', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S17_same_timestamp_ordering', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-02-01T00:00:00Z', '1', '40000', '40000', 'at_regime=neu at_pool=default'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '40000', '20000', 'at_regime=neu at_pool=default'),
        at_move('m1', 'BTC', '2022-02-01T00:00:00Z', '0.3', '0.2999', '40000', 'W1', 'W2', 'at_regime=neu at_pool=default self-transfer proven by address ownership'),
        at_in('e1', 'BTC', '2022-02-01T00:00:00Z', '0.01', '40000', '400', 'at_regime=neu at_pool=default', type_='STAKING'),
    ]),
    at_scenario('S19_explicit_neu_exceeds_neu_with_alt_available', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '0.2', '30000', '6000', 'at_regime=neu at_pool=default'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.3', '0', '35000', '10500', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S20_neu_zero_and_tiny_gain_sign', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '10000', '1000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.3', '10000', '2000', 'at_regime=neu at_pool=default'),
        at_out('s_even', 'BTC', '2022-02-01T00:00:00Z', '0.3', '0', '5000', '1500', 'at_regime=neu at_pool=default'),
        at_out('s_third', 'BTC', '2022-02-02T00:00:00Z', '0.1', '0', '5000', '500', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S21_open_positions_mixed_regimes', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.4', '7000', '2800', 'at_regime=alt at_pool=default'),
        at_in('a2', 'BTC', '2020-02-01T00:00:00Z', '0.4', '8000', '3200', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '30000', '9000', 'at_regime=neu at_pool=default'),
        at_in('n2', 'BTC', '2022-01-02T00:00:00Z', '0.6', '33000', '19800.01', 'at_regime=neu at_pool=default'),
        at_out('da', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=alt at_pool=default'),
        at_out('dn', 'BTC', '2022-02-02T00:00:00Z', '0.4', '0', '35000', '14000', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S22_plain_moving_average_on_at_book', 'moving_average', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.4', '7000', '2800', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '30000', '9000', 'at_regime=neu at_pool=A'),
        at_in('n2', 'BTC', '2022-01-02T00:00:00Z', '0.6', '33000', '19800.01', 'at_regime=neu at_pool=B'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=alt at_pool=default'),
        at_out('d2', 'BTC', '2022-02-02T00:00:00Z', '0.6', '0', '35000', '21000', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S23_fifo_on_at_book_no_swaps', 'fifo', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.4', '7000', '2800', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '30000', '9000', 'at_regime=neu at_pool=default'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default'),
    ]),
    at_scenario('S24_swap_pool_to_other_pool_and_carry_precision', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '10000', '1000', 'at_regime=neu at_pool=P'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.7', '10000', '2000.01', 'at_regime=neu at_pool=P'),
        at_out('w_out', 'BTC', '2022-02-01T00:00:00Z', '0.33333', '0.00007', '30000', '9999.9', 'at_regime=neu at_pool=P at_swap_link=w'),
        at_in('w_in', 'LBTC', '2022-02-01T00:00:00Z', '0.33326', '30000', '9997.8', 'at_regime=neu at_pool=Q at_swap_link=w'),
        at_in('l2', 'LBTC', '2022-02-02T00:00:00Z', '0.1', '31000', '3100', 'at_regime=neu at_pool=Q'),
        at_out('lq', 'LBTC', '2022-03-01T00:00:00Z', '0.2', '0', '32000', '6400', 'at_regime=neu at_pool=Q'),
    ]),
    at_scenario('S25_alt_disposal_exceeds_alt_no_fallback', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.2', '7000', '1400', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.3', '0', '35000', '10500', 'at_regime=alt at_pool=default'),
    ]),
    at_scenario('S26_description_injects_markers', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.3', '0', '35000', '10500', 'at_regime=neu at_pool=default user note at_pool=savings'),
    ]),
    at_scenario('X01_swap_multi_lot_drift_carry', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.3', '10000', '3000.01', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '0.7', '10000', '7000.02', 'at_regime=neu at_pool=default'),
        at_in('b3', 'BTC', '2022-01-03T00:00:00Z', '0.11', '10000', '1100.07', 'at_regime=neu at_pool=default'),
        at_out('s0', 'BTC', '2022-01-04T00:00:00Z', '0.13', '0', '11000', '1430', 'at_regime=neu at_pool=default'),
        at_out('v_out', 'BTC', '2022-02-01T00:00:00Z', '0.95', '0.00011', '12000', '11400', 'at_regime=neu at_pool=default at_swap_link=v'),
        at_in('v_in', 'LBTC', '2022-02-01T00:00:00Z', '0.94989', '12000', '11398.68', 'at_regime=neu at_pool=default at_swap_link=v'),
    ]),
    at_scenario('X02_same_ts_destination_event_waits', 'moving_average_at', [
        at_in('l_in', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '40000', '20000', 'at_regime=neu at_pool=default at_swap_link=w'),
        at_out('l_sale', 'LBTC', '2022-02-01T00:00:00Z', '0.2', '0', '40000', '8000', 'at_regime=neu at_pool=default'),
        at_in('b1', 'BTC', '2021-12-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2021-12-02T00:00:00Z', '1', '31000', '31000', 'at_regime=neu at_pool=default'),
        at_out('b_out', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '40000', '20000', 'at_regime=neu at_pool=default at_swap_link=w'),
    ]),
    at_scenario('X03_same_ts_destination_alt_event_not_blocked', 'moving_average_at', [
        at_in('l_alt', 'LBTC', '2020-01-01T00:00:00Z', '1', '8000', '8000', 'at_regime=alt at_pool=default'),
        at_in('l_in', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '40000', '20000', 'at_regime=neu at_pool=default at_swap_link=w'),
        at_out('l_sale', 'LBTC', '2022-02-01T00:00:00Z', '0.2', '0', '40000', '8000', 'at_regime=alt at_pool=default'),
        at_in('b1', 'BTC', '2021-12-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2021-12-02T00:00:00Z', '1', '31000', '31000', 'at_regime=neu at_pool=default'),
        at_out('b_out', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '40000', '20000', 'at_regime=neu at_pool=default at_swap_link=w'),
    ]),
    at_scenario('X04_two_pool_runtime_deadlock', 'moving_average_at', [
        at_in('a_x', 'AAA', '2022-01-01T00:00:00Z', '1', '100', '100', 'at_regime=neu at_pool=X'),
        at_in('a_y', 'AAA', '2022-01-01T00:00:00Z', '1', '100', '100', 'at_regime=neu at_pool=Y'),
        at_in('p_in', 'AAA', '2022-02-01T00:00:00Z', '1', '100', '100', 'at_regime=neu at_pool=X at_swap_link=P'),
        at_out('e_a', 'AAA', '2022-02-01T00:00:00Z', '0.5', '0', '100', '50', 'at_regime=neu at_pool=X'),
        at_out('s_out', 'AAA', '2022-02-01T00:00:00Z', '0.5', '0', '100', '50', 'at_regime=neu at_pool=Y at_swap_link=S'),
        at_in('b0', 'BBB', '2022-01-01T00:00:00Z', '1', '100', '100', 'at_regime=neu at_pool=Z'),
        at_in('s_in', 'BBB', '2022-02-01T00:00:00Z', '0.5', '100', '50', 'at_regime=neu at_pool=Z at_swap_link=S'),
        at_out('p_out', 'BBB', '2022-02-01T00:00:00Z', '1', '0', '100', '100', 'at_regime=neu at_pool=Z at_swap_link=P'),
    ]),
    at_scenario('X05_forced_neu_out_leg_alt_in_leg_unpaired', 'moving_average_at', [
        at_in('b1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', 'at_regime=neu at_pool=default'),
        at_out('f_out', 'BTC', '2020-06-01T00:00:00Z', '0.5', '0', '9000', '4500', 'at_regime=neu at_pool=default at_swap_link=f'),
        at_in('f_in', 'ETH', '2020-06-01T00:00:00Z', '20', '225', '4500', 'at_regime=alt at_pool=default at_swap_link=f'),
    ]),
    at_scenario('X06_carry_onto_alt_by_date_in_leg', 'moving_average_at', [
        at_in('b1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000.03', '7000.03', 'at_regime=neu at_pool=default'),
        at_out('g_out', 'BTC', '2020-06-01T00:00:00Z', '0.3', '0', '9000', '2700', 'at_regime=neu at_pool=default at_swap_link=g'),
        at_in('g_in', 'ETH', '2020-06-01T01:00:00Z', '12', '225', '2700', 'at_pool=default at_swap_link=g'),
        at_out('e1', 'ETH', '2021-01-01T00:00:00Z', '5', '0', '700', '3500', 'at_regime=alt'),
    ]),
    at_scenario('X07_direct_payout_shape', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.6', '30000', '18000', 'at_regime=neu at_pool=default'),
        at_in('b2', 'BTC', '2022-01-05T00:00:00Z', '0.4', '33000', '13200', 'at_regime=neu at_pool=default'),
        at_out('src', 'BTC', '2022-03-01T00:00:00Z', '0.25', '0.00002', '40000', '10000', 'at_regime=neu at_pool=default at_swap_link=direct-payout:abc Payout swap'),
        at_in('dp_in', 'USDT', '2022-03-01T00:00:00Z', '9990', '1', '9990', 'at_regime=neu at_pool=default at_swap_link=direct-payout:abc'),
        at_out('dp_out', 'USDT', '2022-03-01T00:00:00Z', '9990', '0', '1', '9990', 'at_regime=neu at_pool=default payout to recipient'),
    ]),
    at_scenario('X08_untagged_swap_source_routes_neu', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', ''),
        at_out('u_out', 'BTC', '2022-02-01T00:00:00Z', '0.4', '0', '35000', '14000', 'at_swap_link=u'),
        at_in('u_in', 'LBTC', '2022-02-01T00:00:00Z', '0.4', '35000', '14000', 'at_swap_link=u'),
    ]),
    at_scenario('X09_generic_ma_same_ts_and_fee_out', 'moving_average', [
        at_in('g1', 'BTC', '2022-01-01T00:00:00Z', '0.4', '30000', '12000.01', 'at_regime=neu at_pool=default'),
        at_in('g2', 'BTC', '2022-01-02T00:00:00Z', '0.6', '31000', '18600.02', 'at_regime=neu at_pool=default'),
        at_out('gs', 'BTC', '2022-01-02T00:00:00Z', '0.5', '0', '31000', '15500', 'at_regime=neu at_pool=default'),
        at_out('gf', 'BTC', '2022-01-03T00:00:00Z', '0', '0.00013', '32000', None, 'at_regime=neu at_pool=default'),
        at_out('gs2', 'BTC', '2022-01-04T00:00:00Z', '0.49987', '0', '33000', '16495.71', 'at_regime=neu at_pool=default'),
        at_in('g3', 'BTC', '2022-01-05T00:00:00Z', '0.3', '34000', '10200', 'at_regime=neu at_pool=default'),
        at_out('gs3', 'BTC', '2022-01-06T00:00:00Z', '0.1', '0', '35000', '3500', 'at_regime=neu at_pool=default'),
    ], country='generic'),
    at_scenario('X10_untagged_pool_mismatch_with_alt_present', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.2', '7000', '1400', 'at_regime=alt'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=A'),
        at_out('d1', 'BTC', '2022-02-01T00:00:00Z', '0.1', '0', '35000', '3500', ''),
        at_out('d2', 'BTC', '2022-02-02T00:00:00Z', '0.2', '0', '35000', '7000', ''),
    ]),
    at_scenario('M01_fifo_at_book_swap_consumes_only_alt', 'fifo', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '7000', '7000', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
    ]),
    at_scenario('M02_plain_ma_at_book_swap_consumes_alt_and_neu', 'moving_average', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.3', '7000', '2100', 'at_regime=alt at_pool=default'),
        at_in('n1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_pool=default'),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '35000', '17500', 'at_regime=neu at_pool=default at_swap_link=q'),
    ]),
    at_scenario('M03_three_swap_cycle', 'moving_average_at', [
        at_in('a0', 'AAA', '2022-01-01T00:00:00Z', '1', '1', '1', 'at_regime=neu'),
        at_in('b0', 'BBB', '2022-01-01T00:00:00Z', '1', '1', '1', 'at_regime=neu'),
        at_in('c0', 'CCC', '2022-01-01T00:00:00Z', '1', '1', '1', 'at_regime=neu'),
        at_out('ab_out', 'AAA', '2022-02-01T00:00:00Z', '0.5', '0', '1', '0.5', 'at_regime=neu at_swap_link=zz'),
        at_in('ab_in', 'BBB', '2022-02-01T00:00:00Z', '0.5', '1', '0.5', 'at_regime=neu at_swap_link=zz'),
        at_out('bc_out', 'BBB', '2022-02-01T00:00:00Z', '0.5', '0', '1', '0.5', 'at_regime=neu at_swap_link=mm'),
        at_in('bc_in', 'CCC', '2022-02-01T00:00:00Z', '0.5', '1', '0.5', 'at_regime=neu at_swap_link=mm'),
        at_out('ca_out', 'CCC', '2022-02-01T00:00:00Z', '0.5', '0', '1', '0.5', 'at_regime=neu at_swap_link=aa'),
        at_in('ca_in', 'AAA', '2022-02-01T00:00:00Z', '0.5', '1', '0.5', 'at_regime=neu at_swap_link=aa'),
    ]),
    at_scenario('M04_conflicting_regime_on_lot_only_hit_at_use', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_regime=alt'),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '1', '30000', '30000', 'at_regime=neu'),
    ]),
    at_scenario('M05_conflicting_regime_on_lot_hit_by_disposal', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', 'at_regime=neu at_regime=alt'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.1', '0', '30000', '3000', 'at_regime=neu'),
    ]),
    at_scenario('L02_fiat_times_q_over_q_rounds', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '0.15186497580', '30000', '740283.06814601115230685398217784', 'at_regime=neu at_pool=default'),
        at_in('st', 'BTC', '2022-01-02T00:00:00Z', '0.64095692680', '31000', '85.529328179178118125898318655614', 'at_regime=neu at_pool=default', type_='STAKING'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.1', '0', '33333.333', '3333.3333', 'at_regime=neu at_pool=default'),
    ]),
]


N = "at_regime=neu at_pool=default"
A = "at_regime=alt at_pool=default"

# Hand-written cases for behavior the C-scenarios do not pin.
AT_EXTRA_SCENARIOS = [
    at_scenario('Y01_sale_before_any_lot_with_duplicate_pool_marker', 'moving_average_at', [
        at_out('s1', 'BTC', '2022-01-01T00:00:00Z', '0.1', '0', '100', '10', N + ' at_pool=x'),
        at_in('b1', 'BTC', '2022-02-01T00:00:00Z', '1', '100', '100', N),
    ]),
    at_scenario('Y02_sale_before_any_lot_generic_moving_average', 'moving_average', [
        at_out('s1', 'BTC', '2022-01-01T00:00:00Z', '0.1', '0', '100', '10'),
        at_in('b1', 'BTC', '2022-02-01T00:00:00Z', '1', '100', '100'),
    ], country='generic'),
    at_scenario('Y03_pre_1970_marked_rows_are_invisible_to_validation', 'moving_average_at', [
        at_in('old', 'BTC', '1969-12-31T00:00:00Z', '1', '1', '1', N + ' at_swap_link=ghost'),
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100', N),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '120', '60', N),
    ]),
    at_scenario('Y04_swap_with_an_asset_whose_lots_are_all_pre_1970', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100', N),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '120', '60', N + ' at_swap_link=x'),
        at_in('e0', 'ETH', '1969-06-01T00:00:00Z', '1', '1', '1', N),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.5', '120', '60', N + ' at_swap_link=x'),
    ]),
    at_scenario('Y05_untagged_event_before_lots_of_both_regimes', 'moving_average_at', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '100', '100', ''),
        at_out('s0', 'BTC', '2020-06-01T00:00:00Z', '0.25', '0', '100', '25', ''),
        at_in('n1', 'BTC', '2021-06-01T00:00:00Z', '1', '100', '100', ''),
        at_out('s1', 'BTC', '2021-07-01T00:00:00Z', '0.5', '0', '100', '50', 'at_regime=alt'),
        at_out('s2', 'BTC', '2021-08-01T00:00:00Z', '0.9', '0', '100', '90', ''),
    ]),
    at_scenario('Y06_empty_swap_marker_on_moves_classified_under_fifo', 'fifo', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '0.0001', '100', '0.01', A),
        at_in('n1', 'BTC', '2021-06-01T00:00:00Z', '1', '100', '100', N),
        at_move('m1', 'BTC', '2021-07-01T00:00:00Z', '0.5', '0.4999', '200', 'W1', 'W2', N + ' at_swap_link= self-transfer'),
        at_move('m2', 'BTC', '2021-07-02T00:00:00Z', '0.5', '0.4999', '200', 'W1', 'W2', N + ' at_swap_link= self-transfer'),
        at_out('s1', 'BTC', '2021-08-01T00:00:00Z', '0.5', '0', '300', '150', N, wallet='W2'),
    ]),
    at_scenario('Y07_carried_lot_sold_at_the_swap_instant_by_untagged_event', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '30000', '30000', N),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.4', '0.0001', '40000', '16000', N + ' at_swap_link=c1'),
        at_out('u1', 'LBTC', '2022-02-01T00:00:00Z', '0.1', '0', '40000', '4000', 'at_pool=default'),
        at_in('i1', 'LBTC', '2022-02-01T00:00:00Z', '0.4', '40000', '15990', N + ' at_swap_link=c1'),
        at_out('u2', 'LBTC', '2022-03-01T00:00:00Z', '0.3', '0', '20000', '6000', ''),
    ]),
    at_scenario('Y08_swap_chain_with_hifo_on_alt_only_source', 'hifo', [
        at_in('a1', 'BTC', '2020-01-01T00:00:00Z', '1', '100', '100', A),
        at_in('a2', 'BTC', '2020-02-01T00:00:00Z', '1', '300', '300', A),
        at_out('o1', 'BTC', '2021-06-01T00:00:00Z', '0.5', '0', '400', '200', 'at_pool=default at_swap_link=h1'),
        at_in('i1', 'ETH', '2021-06-01T00:00:00Z', '2', '100', '200', N + ' at_swap_link=h1'),
        at_out('s1', 'ETH', '2021-07-01T00:00:00Z', '1', '0', '150', '150', N),
    ]),
    at_scenario('Y09_duplicate_regime_marker_on_an_in_leg', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100', N),
        at_out('o1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '120', '60', N + ' at_swap_link=d'),
        at_in('i1', 'ETH', '2022-02-01T00:00:00Z', '0.5', '120', '60', 'at_regime=neu at_regime=neu at_pool=default at_swap_link=d'),
    ]),
    # Gains of -4E-14 and -5E-14 are gains at 13 places; -6E-14 is a loss.
    at_scenario('Y11_neu_gain_sign_at_13_places', 'moving_average_at', [
        at_in(f'b{i}', 'BTC', '2022-01-01T00:00:00Z', '1', '100', basis, f'at_regime=neu at_pool=P{i}')
        for i, basis in enumerate(['100.00000000000004', '100.00000000000005', '100.00000000000006',
                                   '100.00000000000015', '99.99999999999995'])
    ] + [
        at_out(f's{i}', 'BTC', '2022-02-01T00:00:00Z', '1', '0', '100', '100', f'at_regime=neu at_pool=P{i}')
        for i in range(5)
    ]),
    at_scenario('Y12_lot_gain_sign_and_alt_tagged_swap_marker_under_fifo', 'fifo', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100.00000000000005', N),
        at_in('b2', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100.00000000000006', N),
        at_in('b3', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '90', N),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '1', '0', '100', '100', N),
        at_out('s2', 'BTC', '2022-02-01T00:00:00Z', '1', '0', '100', '100', N),
        at_out('s3', 'BTC', '2022-02-02T00:00:00Z', '0.5', '0', '100', '50', A + ' at_swap_link=x'),
    ]),
    at_scenario('Y10_pool_marker_quoting_in_messages', 'moving_average_at', [
        at_in('b1', 'BTC', '2022-01-01T00:00:00Z', '1', '100', '100', "at_regime=neu at_pool=it's"),
        at_in('b2', 'BTC', '2022-01-02T00:00:00Z', '1', '100', '100', 'at_regime=neu at_pool=a\\b\x0b\xa0'),
        at_out('s1', 'BTC', '2022-02-01T00:00:00Z', '0.5', '0', '120', '60', 'at_regime=neu at_pool=q"x'),
    ]),
]


def at_constructor_arguments(tx: dict[str, Any], row: int, decimal_type: type) -> tuple[str, dict[str, Any]]:
    """The adapter's constructor arguments for one row, with decimals of
    ``decimal_type``: inbound fiat with no fee, SELL or FEE outbound rows
    with ``fiat_fee = fee * spot``, and moves."""
    D = decimal_type
    common = {"timestamp": tx["ts"], "asset": tx["asset"], "row": row, "unique_id": tx["id"], "notes": tx["notes"]}
    if tx["kind"] == "in":
        return "in", {
            **common, "exchange": tx["wallet"], "holder": AT_HOLDER, "transaction_type": tx["type"],
            "spot_price": D(tx["spot"]), "crypto_in": D(tx["amount"]), "fiat_in_no_fee": D(tx["fiat"]),
            "fiat_in_with_fee": D(tx["fiat"]), "fiat_fee": D("0"),
        }
    if tx["kind"] == "out":
        is_sell = Decimal(tx["amount"]) > 0
        return "out", {
            **common, "exchange": tx["wallet"], "holder": AT_HOLDER, "transaction_type": "SELL" if is_sell else "FEE",
            "spot_price": D(tx["spot"]), "crypto_out_no_fee": D(tx["amount"]), "crypto_fee": D(tx["fee"]),
            "fiat_out_no_fee": D(tx["fiat"]) if is_sell else None,
            "fiat_fee": D(kassiber_product(tx["fee"], tx["spot"])),
        }
    return "intra", {
        **common, "from_exchange": tx["from"], "from_holder": AT_HOLDER, "to_exchange": tx["to"],
        "to_holder": AT_HOLDER, "spot_price": D(tx["spot"]), "crypto_sent": D(tx["sent"]),
        "crypto_received": D(tx["received"]),
    }


def _ok(**fields: Any) -> dict[str, Any]:
    return {"schema_version": 1, "ok": True, **fields}


def at_steps(country: AbstractCountry, conf: Configuration, method: str, input_data: dict[str, InputData], classify: bool) -> list[dict[str, Any]]:
    """The adapter's calls in order, each with RP2's result; a failing call
    ends the run."""
    try:
        country.validate_input_data(list(input_data.values()))
    except Exception as exc:  # noqa: BLE001 - every RP2 failure is a fixture
        return [{"operation": "validate", "expected": error_document(exc)}]
    steps = [{"operation": "validate", "expected": _ok()}]
    try:
        computed = country.compute_tax_for_assets(conf, accounting_engine(method), input_data)
    except Exception as exc:  # noqa: BLE001
        return steps + [{"operation": "compute_multi", "expected": error_document(exc)}]
    if computed is not None:
        assets = [asset_document(computed[asset], classify) for asset in input_data]
        return steps + [{"operation": "compute_multi", "expected": _ok(handled=True, assets=assets)}]
    steps.append({"operation": "compute_multi", "expected": _ok(handled=False)})
    for asset, data in input_data.items():
        try:
            expected = computed_document(compute_tax(conf, accounting_engine(method), data), classify)
        except Exception as exc:  # noqa: BLE001
            expected = error_document(exc)
        steps.append({"operation": "compute", "asset": asset, "expected": expected})
    return steps


def _messages(steps: list[dict[str, Any]]):
    for step in steps:
        expected = step["expected"]
        if not expected["ok"]:
            yield expected["error"]["message"]
        for asset in expected.get("assets", []):
            for gain_loss in asset["gain_losses"]:
                if "at_category_error" in gain_loss:
                    yield gain_loss["at_category_error"]["message"]


def run_at_scenario(scn: dict[str, Any], scratch: Path) -> Optional[dict[str, Any]]:
    """Runs ``scn`` through RP2 as the adapter does; ``None`` when no asset
    has an inbound row (the adapter computes nothing)."""
    assets: list[str] = []
    for tx in scn["txs"]:
        if tx["asset"] not in assets:
            assets.append(tx["asset"])
    wallets = sorted({tx[key] for tx in scn["txs"] for key in ("wallet", "from", "to") if key in tx})
    country = country_object(scn["country"], scn["long_term_days"])
    conf = rp2_configuration(country, assets, wallets, AT_HOLDER, scratch)
    sets = {asset: {kind: TransactionSet(conf, kind.upper(), asset) for kind in RP2_CLASSES} for asset in assets}
    entries: dict[str, list[dict[str, Any]]] = {asset: [] for asset in assets}
    rows = {asset: 0 for asset in assets}
    for tx in scn["txs"]:
        asset = tx["asset"]
        rows[asset] += 1
        kind, rp2_kwargs = at_constructor_arguments(tx, rows[asset], RP2Decimal)
        _, native_kwargs = at_constructor_arguments(tx, rows[asset], Decimal)
        entry = NATIVE_BUILDERS[kind](**native_kwargs)
        where = f"{scn['name']} {asset} row {rows[asset]} ({tx['id']})"
        try:
            transaction = RP2_CLASSES[kind](configuration=conf, **rp2_kwargs)
        except Exception as exc:  # noqa: BLE001 - the histories are built to be valid
            raise SystemExit(f"{where}: RP2 rejects the row: {exc}") from exc
        check_timestamp(where, entry["ts"], transaction.timestamp)
        text = str(transaction)
        if NR.entry_text(entry, derived_values(kind, transaction)) != text:
            raise SystemExit(f"{where}: entry_text differs from RP2")
        entries[asset].append(NR.with_text(entry, text))
        sets[asset][kind].add_entry(transaction)
    input_data = {
        asset: InputData(
            asset=asset,
            unfiltered_in_transaction_set=sets[asset]["in"],
            unfiltered_out_transaction_set=sets[asset]["out"],
            unfiltered_intra_transaction_set=sets[asset]["intra"],
        )
        for asset in assets
        if sets[asset]["in"].count > 0
    }
    if not input_data:
        return None
    steps = at_steps(country, conf, scn["method"], input_data, classify=scn["country"] == "at")
    # The engine reads `text` only for messages that embed it; null fields
    # are the request defaults. Both keep the file within its size budget.
    messages = list(_messages(steps))
    compact = []
    for asset in input_data:
        kept = []
        for entry in entries[asset]:
            fields = {key: value for key, value in entry.items() if value is not None}
            if not any(entry["text"] in message for message in messages):
                del fields["text"]
            kept.append(fields)
        compact.append({"asset": asset, "entries": kept})
    return {
        "name": scn["name"],
        "country": NR.country_spec(scn["country"], scn["long_term_days"] if scn["country"] == "generic" else None),
        "method": scn["method"],
        "assets": compact,
        "steps": steps,
    }


def at_stats(scenarios: list[dict[str, Any]]) -> dict[str, int]:
    """Counts per step outcome, Austrian category, and classification error."""
    stats: dict[str, int] = {}

    def bump(key: str) -> None:
        stats[key] = stats.get(key, 0) + 1

    for scn in scenarios:
        bump(f"method:{scn['method']}")
        for step in scn["steps"]:
            expected = step["expected"]
            if not expected["ok"]:
                bump(f"{step['operation']}:error")
                continue
            if step["operation"] == "compute_multi":
                bump("compute_multi:handled" if expected["handled"] else "compute_multi:declined")
            else:
                bump(f"{step['operation']}:ok")
            for asset in expected.get("assets", []):
                for gain_loss in asset["gain_losses"]:
                    if "at_category" in gain_loss:
                        bump(f"category:{gain_loss['at_category']}")
                    elif "at_category_error" in gain_loss:
                        bump("category:error")
    return dict(sorted(stats.items()))


# ---------------------------------------------------------------------------
# Random Austrian and moving-average histories

AT_ASSETS = ("BTC", "LBTC", "USDT")
AT_EARN_TYPES = ("STAKING", "INTEREST", "MINING", "INCOME", "STAKING", "INTEREST", "AIRDROP", "HARDFORK", "WAGES")
MOVE_NOTE = "self-transfer proven by address ownership"

# Vienna dates that move the Spekulationsfrist: a leap day, the day before
# and after DST changes, and UTC instants that are already the next day in
# Vienna.
FRIST_LOTS = (
    datetime(2020, 2, 29, 10, tzinfo=timezone.utc),
    datetime(2020, 2, 28, 23, 30, tzinfo=timezone.utc),
    datetime(2020, 6, 30, 22, tzinfo=timezone.utc),
    datetime(2020, 1, 15, 23, 30, tzinfo=timezone.utc),
    datetime(2020, 10, 24, 22, 30, tzinfo=timezone.utc),
    datetime(2020, 3, 28, 23, 30, tzinfo=timezone.utc),
    datetime(2019, 12, 31, 23, 30, tzinfo=timezone.utc),
    datetime(2019, 7, 1, 12, tzinfo=timezone.utc),
)

# The cutoff instant (2021-02-28T23:00:00Z) in the forms RP2 compares as
# instants, with the regime each one gets by date.
CUTOFF_STAMPS = (
    ("2021-02-28T22:59:59Z", "alt"),
    ("2021-02-28T22:59:59.999999Z", "alt"),
    ("2021-02-28T23:00:00Z", "neu"),
    ("2021-02-28T23:00:01Z", "neu"),
    ("2021-03-01T00:00:00+01:00", "neu"),
    ("2021-02-28T23:59:59+01:00", "alt"),
    ("2021-03-01T00:30:00+02:00", "alt"),
    ("2021-02-28T23:30:00-01:00", "neu"),
)


def z(instant: datetime) -> str:
    """Kassiber's canonical UTC timestamp."""
    instant = instant.astimezone(timezone.utc)
    return instant.strftime("%Y-%m-%dT%H:%M:%S.%fZ" if instant.microsecond else "%Y-%m-%dT%H:%M:%SZ")


def instant_of(stamp_text: str) -> datetime:
    return NR.parse_timestamp(stamp_text)


class AtFlow:
    """One random history as Kassiber emits it, tracking what each regime,
    pool, and wallet holds (in msat) so most histories stay computable."""

    def __init__(self, rng: random.Random, *, country: str, assets: list[str], pools: list[str], start: datetime) -> None:
        self.rng = rng
        self.at = country == "at"
        self.assets = assets
        self.pools = pools
        self.t = start
        self.txs: list[dict[str, Any]] = []
        self.count = 0
        self.wallets = ["W1", "W2"]
        self.balance = {(asset, wallet): 0 for asset in assets for wallet in self.wallets}
        self.held: dict[tuple[str, str, Optional[str]], int] = {}
        self.swaps = 0

    def uid(self, prefix: str) -> str:
        self.count += 1
        return f"{prefix}{self.count}"

    def step(self) -> None:
        if self.rng.random() < 0.15:
            return  # a tie with the previous row
        seconds = self.rng.choice([1, 600, 3600, 86400, 86400 * 9, 86400 * 40, 86400 * 120])
        self.t += timedelta(seconds=seconds)

    @staticmethod
    def date_regime(instant: datetime) -> str:
        return "alt" if instant < NEU_CUTOFF else "neu"

    def key(self, asset: str, regime: str, pool: str) -> tuple[str, str, Optional[str]]:
        if not self.at:
            return (asset, "pool", None)
        # Alt is FIFO across all pools.
        return (asset, "alt", None) if regime == "alt" else (asset, "neu", pool)

    def holding(self, asset: str, regime: str, pool: str) -> int:
        return self.held.get(self.key(asset, regime, pool), 0)

    def add(self, asset: str, regime: str, pool: str, msat: int) -> None:
        key = self.key(asset, regime, pool)
        self.held[key] = self.held.get(key, 0) + msat

    def compose(self, regime, pool, *, wahlrecht=False, swap=None, description="") -> str:
        """``_compose_event_notes``: regime, basis, pool, swap id, then the
        description; generic books carry the description only."""
        if not self.at:
            return description
        parts = []
        if regime:
            parts.append(f"at_regime={regime}")
        if wahlrecht:
            parts.append("at_regime_basis=wahlrecht")
        if pool is not None:
            parts.append(f"at_pool={pool}")
        if swap is not None:
            parts.append(f"at_swap_link={swap}")
        if description:
            parts.append(description)
        return " ".join(parts)

    def msat(self) -> int:
        return self.rng.choice([
            self.rng.randint(10**6, 2 * 10**11),
            self.rng.randint(1, 50) * 10**10,
            self.rng.randint(1, 10**8) * 1000,
        ])

    def fiat(self, msat: int, spot: str) -> str:
        with localcontext(CTX):
            exact = Decimal(msat) / Decimal(10**11) * Decimal(spot)
            roll = self.rng.random()
            if roll < 0.5:
                value = exact.quantize(Decimal("0.01"))
            elif roll < 0.8:
                value = exact
            else:
                value = Decimal(self.rng.randint(1, 10**7)) / Decimal(100)
        return str(value) if value > Decimal("0.01") else "0.01"

    def wallet_with(self, asset: str, minimum: int = 1) -> Optional[str]:
        wallets = [w for w in self.wallets if self.balance[(asset, w)] >= minimum]
        return self.rng.choice(wallets) if wallets else None

    def sources(self, asset: Optional[str] = None) -> list[tuple[str, str, Optional[str]]]:
        """(asset, regime, pool) holdings with something to dispose of."""
        found = []
        for (held_asset, regime, pool), amount in self.held.items():
            if amount > 1 and (asset is None or held_asset == asset):
                if self.wallet_with(held_asset, 2):
                    found.append((held_asset, regime, pool))
        return found

    # -- rows ----------------------------------------------------------------

    def buy(self, asset=None, *, wallet=None, regime=None, pool=None, type_=None, tagged=True,
            swap=None, stamp=None, description="", msat=None) -> dict[str, Any]:
        asset = asset or self.rng.choice(self.assets)
        wallet = wallet or self.rng.choice(self.wallets)
        stamp = stamp or z(self.t)
        instant = instant_of(stamp)
        pool = pool or self.rng.choice(self.pools)
        actual = regime or self.date_regime(instant)
        if type_ is None:
            type_ = "BUY" if self.rng.random() < 0.75 else self.rng.choice(AT_EARN_TYPES)
        msat = msat or self.msat()
        spot = random_price(self.rng)
        marked_pool = pool if tagged or self.rng.random() < 0.5 else None
        notes = self.compose(actual if tagged else None, marked_pool, swap=swap, description=description)
        if self.at and not tagged:
            actual, pool = self.date_regime(instant), marked_pool or "default"
        tx = at_in(self.uid("b"), asset, stamp, msat_to_btc(msat), spot, self.fiat(msat, spot), notes, type_, wallet)
        self.txs.append(tx)
        self.add(asset, actual, pool, msat)
        self.balance[(asset, wallet)] += msat
        return tx

    def pick_source(self, asset=None, regime=None):
        choices = [s for s in self.sources(asset) if regime is None or s[1] == regime]
        return self.rng.choice(choices) if choices else None

    def unambiguous(self, asset: str, regime: str, pool: str) -> bool:
        """Whether an untagged disposal routes to ``regime`` without RP2's
        ambiguity error."""
        if regime == "alt":
            return self.holding(asset, "neu", pool) == 0
        return self.holding(asset, "alt", pool) == 0

    def sell(self, source=None, *, tagged=None, wahlrecht=None, swap=None, fee=None, amount=None,
             description="", stamp=None, wallet=None, cap=True) -> Optional[dict[str, Any]]:
        source = source or self.pick_source()
        if source is None:
            return None
        asset, regime, pool = source
        pool = pool or self.rng.choice(self.pools)
        wallet = wallet or self.wallet_with(asset, 2)
        if wallet is None:
            return None
        limit = min(self.holding(asset, regime, pool), self.balance[(asset, wallet)])
        if limit < 2:
            return None
        if amount is None:
            roll = self.rng.random()
            amount = limit if roll < 0.15 else (limit // 2 or 1) if roll < 0.3 else self.rng.randint(1, limit)
        elif cap:
            amount = min(amount, limit)
        fee_msat = 0
        if (fee if fee is not None else self.rng.random() < 0.35) and amount > 1:
            fee_msat = self.rng.randint(1, min(amount - 1, 10**7))
        if tagged is None:
            tagged = not (self.at and self.rng.random() < 0.15 and self.unambiguous(asset, regime, pool))
        if wahlrecht is None:
            wahlrecht = regime == "neu" and self.rng.random() < 0.3
        spot = random_price(self.rng)
        net = amount - fee_msat
        notes = self.compose(regime if tagged else None, pool, wahlrecht=wahlrecht, swap=swap, description=description)
        tx = at_out(self.uid("s"), asset, stamp or z(self.t), msat_to_btc(net), msat_to_btc(fee_msat), spot,
                    self.fiat(net, spot), notes, wallet)
        self.txs.append(tx)
        self.add(asset, regime, pool, -amount)
        self.balance[(asset, wallet)] -= amount
        return tx

    def fee_row(self) -> Optional[dict[str, Any]]:
        source = self.pick_source()
        if source is None:
            return None
        asset, regime, pool = source
        pool = pool or "default"
        wallet = self.wallet_with(asset, 2)
        limit = min(self.holding(asset, regime, pool), self.balance[(asset, wallet)], 10**8)
        fee_msat = self.rng.randint(1, limit)
        notes = self.compose(regime, pool)
        self.txs.append(at_out(self.uid("f"), asset, z(self.t), "0", msat_to_btc(fee_msat), random_price(self.rng), None, notes, wallet))
        self.add(asset, regime, pool, -fee_msat)
        self.balance[(asset, wallet)] -= fee_msat
        return self.txs[-1]

    def move(self, *, tagged=True) -> Optional[dict[str, Any]]:
        source = self.pick_source()
        if source is None:
            return None
        asset, regime, pool = source
        pool = pool or "default"
        src = self.wallet_with(asset, 2)
        dst = "W2" if src == "W1" else "W1"
        sent = self.rng.randint(2, self.balance[(asset, src)])
        fee_msat = 0
        if self.rng.random() < 0.6:
            fee_msat = self.rng.randint(1, min(sent - 1, 10**7, self.holding(asset, regime, pool)))
        spot = "0" if fee_msat == 0 and self.rng.random() < 0.5 else random_price(self.rng)
        notes = self.compose(regime if tagged else None, pool, description=MOVE_NOTE)
        self.txs.append(at_move(self.uid("m"), asset, z(self.t), msat_to_btc(sent), msat_to_btc(sent - fee_msat), spot, src, dst, notes))
        self.balance[(asset, src)] -= sent
        self.balance[(asset, dst)] += sent - fee_msat
        self.add(asset, regime, pool, -fee_msat)
        return self.txs[-1]

    def swap(self, *, direct=False, unmarked=False, alt=False) -> bool:
        """A Neu crypto-to-crypto swap: a marked SELL and a marked BUY on
        another asset at the same or a later instant."""
        if len(self.assets) < 2:
            return False
        source = self.pick_source(regime="alt" if alt else "neu")
        if source is None:
            return False
        asset, regime, pool = source
        pool = pool or "default"
        self.swaps += 1
        sid = f"direct-payout:{self.swaps:x}{self.count}" if direct else self.rng.choice(
            [f"swap-{self.swaps}", f"s{self.swaps}", f"a->b:{self.swaps}"])
        out = self.sell(source, tagged=not unmarked or not self.unambiguous(asset, regime, pool), swap=sid, wahlrecht=False)
        if out is None:
            return False
        delay = 0 if direct else self.rng.choice([0, 0, 1, 600])
        in_instant = self.t + timedelta(seconds=delay)
        destination = self.rng.choice([a for a in self.assets if a != asset])
        in_pool = pool if self.rng.random() < 0.85 else self.rng.choice(self.pools)
        in_regime = "alt" if alt else self.date_regime(in_instant)
        lot = self.buy(destination, wallet=out["wallet"], regime=in_regime, pool=in_pool, type_="BUY", swap=sid,
                       stamp=z(in_instant))
        self.t = in_instant
        if direct:
            # The synthetic payout sale of the target asset, at the same instant.
            msat = int(Decimal(lot["amount"]) * 10**11)
            self.sell((destination, in_regime, in_pool if in_regime == "neu" else None), amount=msat, fee=False,
                      tagged=True, wahlrecht=False, description="payout to recipient", wallet=out["wallet"])
        return True

    def emitted(self) -> list[dict[str, Any]]:
        """Rows in Kassiber's emission order: by instant, ties as built (or
        shuffled, which RP2's own ordering must absorb)."""
        rows = list(self.txs)
        if self.rng.random() < 0.3:
            self.rng.shuffle(rows)
        return sorted(rows, key=lambda tx: instant_of(tx["ts"]))


def flow_ops(flow: AtFlow, count: int, weights: dict[str, int]) -> None:
    names = list(weights)
    for _ in range(count):
        flow.step()
        op = flow.rng.choices(names, [weights[n] for n in names])[0]
        done = None
        if op == "sell":
            done = flow.sell()
        elif op == "fee":
            done = flow.fee_row()
        elif op == "move":
            done = flow.move(tagged=flow.rng.random() < 0.92)
        elif op == "swap":
            done = flow.swap(unmarked=flow.rng.random() < 0.1)
        elif op == "direct":
            done = flow.swap(direct=True)
        elif op == "altswap":
            done = flow.swap(alt=True)
        if not done:
            flow.buy()


ERROR_KINDS = (
    "dup_pool", "conflict_regime", "dup_regime", "empty_swap", "unpair", "alt_in_leg", "same_asset",
    "earlier_in", "swap_on_fee", "swap_on_move", "overspend", "pool_mismatch", "ambiguous",
    "ambiguous_fee", "dup_swap", "conflict_lot", "empty_swap_move",
)


def inject_error(flow: AtFlow, rng: random.Random, kinds: tuple[str, ...] = ERROR_KINDS) -> str:
    """Breaks one rule RP2 enforces; returns the kind of break. Lot methods
    ignore markers while computing, so their breaks surface when the
    adapter classifies the rows."""
    kind = rng.choice(kinds)
    txs = flow.txs
    marked = [tx for tx in txs if "at_swap_link=" in tx["notes"]]
    lots = [tx for tx in txs if tx["kind"] == "in"]
    if kind == "conflict_lot" and lots:
        tx = rng.choice(lots)
        tx["notes"] += " at_regime=alt" if "at_regime=neu" in tx["notes"] else " at_regime=neu"
    elif kind == "empty_swap_move":
        move = flow.move()
        if move:
            move["notes"] += " at_swap_link="
    elif kind == "dup_pool" and txs:
        rng.choice(txs)["notes"] += " user note at_pool=savings"
    elif kind == "conflict_regime" and txs:
        tx = rng.choice(txs)
        tx["notes"] = "at_regime=alt " + tx["notes"] if "at_regime=neu" in tx["notes"] else tx["notes"] + " at_regime=neu"
    elif kind == "dup_regime" and txs:
        tx = rng.choice(txs)
        tx["notes"] = tx["notes"] + (" at_regime=alt" if "at_regime=alt" in tx["notes"] else " at_regime=neu")
    elif kind == "empty_swap" and marked:
        tx = rng.choice(marked)
        tx["notes"] = re.sub(r"at_swap_link=\S+", "at_swap_link=", tx["notes"])
    elif kind == "unpair" and marked:
        tx = rng.choice(marked)
        tx["notes"] = re.sub(r" ?at_swap_link=\S+", "", tx["notes"])
    elif kind == "alt_in_leg" and marked:
        legs = [tx for tx in marked if tx["kind"] == "in"]
        if legs:
            tx = rng.choice(legs)
            tx["notes"] = tx["notes"].replace("at_regime=neu", "at_regime=alt")
    elif kind == "dup_swap" and marked:
        rng.choice(marked)["notes"] += " at_swap_link=again"
    elif kind == "same_asset":
        source = flow.pick_source(regime="neu")
        if source:
            out = flow.sell(source, swap="same", wahlrecht=False)
            if out:
                flow.buy(out["asset"], swap="same", type_="BUY")
    elif kind == "earlier_in" and len(flow.assets) > 1:
        source = flow.pick_source(regime="neu")
        if source:
            out = flow.sell(source, swap="early", wahlrecht=False)
            if out:
                other = [a for a in flow.assets if a != out["asset"]][0]
                flow.buy(other, swap="early", type_="BUY", stamp=z(flow.t - timedelta(hours=1)))
    elif kind == "swap_on_fee" and len(flow.assets) > 1:
        fee = flow.fee_row()
        if fee:
            fee["notes"] += " at_swap_link=feeswap"
            other = [a for a in flow.assets if a != fee["asset"]][0]
            flow.buy(other, swap="feeswap", type_="BUY")
    elif kind == "swap_on_move":
        move = flow.move()
        if move:
            move["notes"] += " at_swap_link=moveswap"
    elif kind == "overspend":
        source = flow.pick_source()
        if source:
            asset, regime, pool = source
            pool = pool or "default"
            flow.sell(source, amount=flow.holding(asset, regime, pool) + 10**9, tagged=True, cap=False)
    elif kind == "pool_mismatch":
        source = flow.pick_source(regime="neu")
        if source:
            asset, _, pool = source
            flow.txs.append(at_out(flow.uid("s"), asset, z(flow.t), "0.001", "0", "100", "0.10",
                                   flow.compose("neu", "P9" if pool != "P9" else "P8"), "W1"))
    elif kind in ("ambiguous", "ambiguous_fee"):
        alt = flow.pick_source(regime="alt")
        if alt:
            asset = alt[0]
            if kind == "ambiguous":
                flow.buy(asset, regime="neu", pool="default")
                flow.txs.append(at_out(flow.uid("s"), asset, z(flow.t), "0.0001", "0", "100", "0.01", "at_pool=default", "W1"))
            else:
                flow.buy(asset, regime="neu", pool="default", wallet="W1")
                flow.txs.append(at_move(flow.uid("m"), asset, z(flow.t), "0.0001", "0.00009", "100", "W1", "W2",
                                        "at_pool=default " + MOVE_NOTE))
    return kind


def random_at_history(index: int, seed: int = SEED) -> dict[str, Any]:
    rng = random.Random(seed * 7919 + index)
    profile = rng.choices(
        ["generic_ma", "neu", "mixed", "swaps", "frist", "cutoff", "methods"],
        [55, 55, 75, 115, 40, 30, 70],
    )[0]
    country = "generic" if profile == "generic_ma" else "at"
    method = "moving_average" if country == "generic" else "moving_average_at"
    long_term_days = 365
    pools = ["default"]
    if profile in ("neu", "mixed", "swaps") and rng.random() < 0.15:
        pools = ["default", "default", "P2"]
    if profile == "generic_ma":
        assets = ["BTC"] if rng.random() < 0.8 else ["BTC", "LBTC"]
        long_term_days = rng.choice([0, 1, 365, 365, 366, 730])
        start = datetime(rng.choice([2018, 2020, 2022, 2024]), rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    elif profile in ("neu", "swaps"):
        assets = ["BTC"] if profile == "neu" else list(AT_ASSETS[: rng.choice([2, 2, 3])])
        start = datetime(rng.choice([2021, 2022, 2023, 2024]), rng.randint(3, 12), rng.randint(1, 28), rng.choice([0, 10, 23]), tzinfo=timezone.utc)
        if profile == "swaps" and rng.random() < 0.15:
            start = datetime(2020, 11, rng.randint(1, 28), tzinfo=timezone.utc)
    elif profile == "methods":
        assets = list(AT_ASSETS[: rng.choice([1, 2, 2])])
        method = rng.choice(["fifo", "lifo", "hifo", "lofo", "moving_average", "moving_average"])
        start = datetime(rng.choice([2019, 2020, 2021]), rng.randint(1, 12), rng.randint(1, 28), tzinfo=timezone.utc)
    else:
        assets = ["BTC"] if rng.random() < 0.75 else ["BTC", "LBTC"]
        month = rng.randint(5, 12) if rng.random() < 0.7 else rng.randint(1, 12)
        start = datetime(rng.choice([2019, 2020, 2020]), month, rng.randint(1, 28), rng.choice([0, 8, 23]), tzinfo=timezone.utc)
    flow = AtFlow(rng, country=country, assets=assets, pools=pools, start=start)

    if profile == "frist":
        lots = sorted(rng.sample(FRIST_LOTS, rng.randint(1, 3)))
        for instant in lots:
            flow.t = instant
            flow.buy("BTC", regime="alt", pool="default", type_="BUY", tagged=rng.random() < 0.7)
        anniversary = lots[0].astimezone(VIENNA).date()
        try:
            anniversary = anniversary.replace(year=anniversary.year + 1)
        except ValueError:
            anniversary = anniversary.replace(year=anniversary.year + 1, day=28)
        last = datetime.combine(anniversary, datetime.min.time(), VIENNA) + timedelta(hours=23, minutes=59, seconds=59)
        moments = sorted(rng.sample([last - timedelta(hours=11), last, last + timedelta(seconds=1),
                                     last + timedelta(days=40), last - timedelta(days=200)], rng.randint(2, 4)))
        for moment in moments:
            flow.t = max(flow.t, moment.astimezone(timezone.utc))
            source = flow.pick_source(regime="alt")
            if source:
                flow.sell(source, amount=max(2, flow.holding("BTC", "alt", None) // rng.choice([2, 3, 5])))
        if rng.random() < 0.4:
            flow.t = max(flow.t, datetime(2021, 4, 1, tzinfo=timezone.utc))
            flow_ops(flow, rng.randint(2, 5), {"buy": 3, "sell": 3, "fee": 1})
    elif profile == "cutoff":
        for stamp_text, _ in rng.sample(CUTOFF_STAMPS, rng.randint(2, 4)):
            flow.buy("BTC", pool="default", type_="BUY", tagged=rng.random() < 0.4, stamp=stamp_text)
        flow.t = datetime(2021, 3, 2, tzinfo=timezone.utc)
        flow_ops(flow, rng.randint(2, 6), {"buy": 1, "sell": 5, "fee": 1, "move": 1})
    else:
        weights = {
            "generic_ma": {"buy": 40, "sell": 35, "fee": 8, "move": 12},
            "neu": {"buy": 40, "sell": 38, "fee": 8, "move": 10},
            "mixed": {"buy": 40, "sell": 38, "fee": 8, "move": 10},
            "swaps": {"buy": 25, "sell": 18, "swap": 32, "direct": 8, "altswap": 2, "fee": 6, "move": 8},
            "methods": {"buy": 35, "sell": 30, "swap": 20, "fee": 6, "move": 8},
        }[profile]
        flow.buy(assets[0], type_="BUY")
        flow_ops(flow, rng.randint(3, 9), weights)
    label = profile
    if profile == "methods" and rng.random() < 0.45:
        label += "_" + inject_error(flow, rng, ("conflict_lot", "conflict_lot", "empty_swap_move", "dup_regime", "empty_swap"))
    elif country == "at" and rng.random() < 0.2:
        label += "_" + inject_error(flow, rng)
    return at_scenario(f"R{index:03d}_{label}_{method}", method, flow.emitted(), country=country, long_term_days=long_term_days)


def at_main(args: argparse.Namespace) -> None:
    scenarios = []
    with tempfile.TemporaryDirectory(prefix="kassiber-engine-fixtures-") as scratch:
        scratch_path = Path(scratch)
        for scn in C_SCENARIOS + AT_EXTRA_SCENARIOS:
            result = run_at_scenario(scn, scratch_path)
            if result is None:
                raise SystemExit(f"{scn['name']}: no asset has an inbound row")
            scenarios.append(result)
        for index in range(args.at_histories):
            result = run_at_scenario(random_at_history(index, args.seed), scratch_path)
            if result is not None:
                scenarios.append(result)
    document = {
        "schema": "kassiber-tax-engine/at-fixtures/v1",
        "generator": "tax-engine/scripts/gen_engine_fixtures.py",
        "seed": args.seed,
        "notes": [
            "Each scenario is one adapter run: validate, then compute_multi, then compute per asset when compute_multi declines.",
            "Requests are built from the scenario's country, method, and assets; compute steps send only their asset.",
            "Decimals use the canonical form [-]<digits>E<exponent>; exponents are significant.",
        ],
        "stats": at_stats(scenarios),
        "scenarios": scenarios,
    }
    text = json.dumps(document, ensure_ascii=False, separators=(",", ":")) + "\n"
    size = len(text.encode())
    if args.at_max_bytes and size > args.at_max_bytes:
        raise SystemExit(f"Austrian fixtures are {size} bytes, over the {args.at_max_bytes} byte budget")
    args.at_output.write_text(text, encoding="utf-8")
    print(f"wrote {len(scenarios)} scenarios ({size} bytes) to {args.at_output}: {document['stats']}")

# ---------------------------------------------------------------------------


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--suite", choices=("all", "generic", "at"), default="all", help="which fixture file to write")
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--histories", type=int, default=RANDOM_HISTORIES)
    parser.add_argument("--seed", type=int, default=SEED)
    parser.add_argument("--max-bytes", type=int, default=MAX_BYTES, help="size budget (0 disables it)")
    parser.add_argument("--at-output", type=Path, default=AT_OUTPUT)
    parser.add_argument("--at-histories", type=int, default=AT_RANDOM_HISTORIES)
    parser.add_argument("--at-max-bytes", type=int, default=AT_MAX_BYTES, help="Austrian size budget (0 disables it)")
    args = parser.parse_args()

    context = decimal.getcontext()
    if context.prec != 32 or context.rounding != ROUND_HALF_EVEN:
        raise SystemExit(f"expected RP2's 32-digit context on the main thread, found {context}")
    if args.suite in ("all", "generic"):
        generic_main(args)
    if args.suite in ("all", "at"):
        at_main(args)


def generic_main(args: argparse.Namespace) -> None:
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
