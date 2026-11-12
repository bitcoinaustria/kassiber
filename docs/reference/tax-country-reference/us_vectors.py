"""Inputs for the United States vectors; expected values come from us.py.

`hand_check` entries are figures worked by hand from the cited publication or
rule; build.py fails if the model disagrees with them.
"""

WALLET = {"method": "fifo", "pool_scope": "wallet", "rail_policy": None}
GLOBAL = {"method": "fifo", "pool_scope": "global", "rail_policy": None}


def policy(method, scope="wallet"):
    return {"method": method, "pool_scope": scope, "rail_policy": None}


def buy(i, t, q, usd, w="A"):
    return {"id": i, "time": t, "wallet": w, "type": "buy", "asset": "BTC", "quantity": q, "fiat": usd}


def sell(i, t, q, usd, w="A", fee_q=None, kind="sell"):
    e = {"id": i, "time": t, "wallet": w, "type": kind, "asset": "BTC", "quantity": q, "fiat": usd}
    if fee_q:
        e["fee_quantity"] = fee_q
    return e


def move(i, t, q, src, dst, fee_q=None, fee_v=None):
    e = {"id": i, "time": t, "wallet": src, "to_wallet": dst, "type": "transfer", "asset": "BTC", "quantity": q}
    if fee_q:
        e["fee_quantity"], e["fee_fiat_value"] = fee_q, fee_v
    return e


def inc(i, t, q, usd, kind, w="A", type_="income"):
    return {"id": i, "time": t, "wallet": w, "type": type_, "subkind": kind, "asset": "BTC", "quantity": q, "fiat": usd}


VECTORS = [
    {
        "id": "us-01-fifo-short-term",
        "title": "A sale within the year is short-term; FIFO takes the earliest lot first",
        "rules": ["US-EVENT-SELL", "US-METHOD-FIFO", "US-HOLD-1Y", "US-8949"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-01-10T15:00:00+00:00", "0.5", "20000.00"),
            buy("b2", "2024-03-01T15:00:00+00:00", "0.5", "30000.00"),
            sell("s1", "2024-06-15T15:00:00+00:00", "0.6", "42000.00"),
        ],
        # b1: 0.5 for 20000; b2: 0.1 of 0.5 for 6000. Proceeds 35000 + 7000.
        "hand_check": {
            "disposals.0.fragments.0.gain": "15000.00",
            "disposals.0.fragments.1.cost": "6000.00",
            "disposals.0.fragments.1.gain": "1000.00",
            "years.2024.short_gain": "16000.00",
            "years.2024.form_8949_short_box": "C",
        },
    },
    {
        "id": "us-02-pub544-anniversary",
        "title": "Publication 544: bought 2024-06-17; sold 2025-06-17 is short-term, 2025-06-18 long-term",
        "rules": ["US-HOLD-1Y", "US-HOLD-CAL"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-06-17T15:00:00+00:00", "1", "60000.00"),
            sell("s1", "2025-06-17T15:00:00+00:00", "0.5", "50000.00"),
            sell("s2", "2025-06-18T15:00:00+00:00", "0.5", "50000.00"),
        ],
        "hand_check": {
            "disposals.0.fragments.0.term": "short",
            "disposals.1.fragments.0.term": "long",
            "years.2025.short_gain": "20000.00",
            "years.2025.long_gain": "20000.00",
            "years.2025.form_8949_long_box": "L",
        },
    },
    {
        "id": "us-03-pub550-example",
        "title": "Publication 550: bought 2024-01-31; sold 2025-01-29 short-term, sold 2025-02-06 long-term",
        "rules": ["US-HOLD-1Y", "US-HOLD-CAL"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-01-31T12:00:00+00:00", "1", "40000.00"),
            sell("s1", "2025-01-29T12:00:00+00:00", "0.5", "50000.00"),
            sell("s2", "2025-02-06T12:00:00+00:00", "0.5", "48000.00"),
        ],
        "hand_check": {
            "disposals.0.fragments.0.term": "short",
            "disposals.1.fragments.0.term": "long",
        },
    },
    {
        "id": "us-04-leap-day-acquisition",
        "title": "Acquired 29 February: its anniversary is 28 February, so 1 March is the first long-term day",
        "rules": ["US-HOLD-CAL", "US-HOLD-LEAP"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-02-29T12:00:00+00:00", "0.1", "6000.00"),
            sell("s1", "2025-02-28T12:00:00+00:00", "0.05", "4500.00"),
            sell("s2", "2025-03-01T12:00:00+00:00", "0.05", "4600.00"),
        ],
        "hand_check": {"disposals.0.fragments.0.term": "short", "disposals.1.fragments.0.term": "long"},
    },
    {
        "id": "us-05-365-days-across-leap",
        "title": "365 elapsed days across 29 February are not more than one year (RP2's US plugin says long-term)",
        "rules": ["US-HOLD-CAL", "US-ENGINE-DAYCOUNT"],
        "policy": WALLET,
        "events": [
            buy("b1", "2023-03-01T10:00:00+00:00", "1", "23000.00"),
            sell("s1", "2024-02-29T10:00:00+00:00", "1", "62000.00"),
        ],
        "hand_check": {"disposals.0.fragments.0.term": "short", "years.2024.short_gain": "39000.00"},
    },
    {
        "id": "us-06-utc-calendar",
        "title": "Dates are UTC dates: an evening purchase in New York is the next day's lot (a documented choice)",
        "rules": ["US-HOLD-CAL", "US-CAL-UTC"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-06-17T22:00:00-05:00", "1", "65000.00"),
            sell("s1", "2025-06-18T10:00:00-05:00", "1", "70000.00"),
        ],
        # UTC: acquired 2024-06-18, sold 2025-06-18: the anniversary, short.
        # New York dates (06-17 to 06-18) would make it long.
        "hand_check": {"disposals.0.fragments.0.acquired": "2024-06-18", "disposals.0.fragments.0.term": "short"},
    },
    {
        "id": "us-07-lifo",
        "title": "LIFO (a standing specific identification) takes the latest lot first",
        "rules": ["US-METHOD-SPECID", "US-METHOD-LIFO"],
        "policy": policy("lifo"),
        "events": [
            buy("b1", "2024-01-01T12:00:00+00:00", "1", "40000.00"),
            buy("b2", "2024-02-01T12:00:00+00:00", "1", "45000.00"),
            buy("b3", "2024-03-01T12:00:00+00:00", "1", "60000.00"),
            sell("s1", "2024-04-01T12:00:00+00:00", "1.5", "100500.00"),
        ],
        "hand_check": {
            "disposals.0.fragments.0.lot": "b3",
            "disposals.0.fragments.1.lot": "b2",
            "disposals.0.fragments.1.cost": "22500.00",
            "years.2024.short_gain": "18000.00",
        },
    },
    {
        "id": "us-08-hifo",
        "title": "HIFO takes the highest unit cost first, ties by acquisition",
        "rules": ["US-METHOD-SPECID", "US-METHOD-HIFO"],
        "policy": policy("hifo"),
        "events": [
            buy("b1", "2024-01-01T12:00:00+00:00", "1", "50000.00"),
            buy("b2", "2024-02-01T12:00:00+00:00", "1", "30000.00"),
            buy("b3", "2024-03-01T12:00:00+00:00", "1", "50000.00"),
            sell("s1", "2024-04-01T12:00:00+00:00", "2.5", "150000.00"),
        ],
        "hand_check": {
            "disposals.0.fragments.0.lot": "b1",
            "disposals.0.fragments.1.lot": "b3",
            "disposals.0.fragments.2.lot": "b2",
            "disposals.0.fragments.2.cost": "15000.00",
            "years.2024.short_gain": "35000.00",
        },
    },
    {
        "id": "us-09-lofo",
        "title": "LOFO takes the lowest unit cost first",
        "rules": ["US-METHOD-SPECID", "US-METHOD-LOFO"],
        "policy": policy("lofo"),
        "events": [
            buy("b1", "2024-01-01T12:00:00+00:00", "1", "50000.00"),
            buy("b2", "2024-02-01T12:00:00+00:00", "1", "30000.00"),
            buy("b3", "2024-03-01T12:00:00+00:00", "1", "40000.00"),
            sell("s1", "2024-04-01T12:00:00+00:00", "1.5", "90000.00"),
        ],
        "hand_check": {"disposals.0.fragments.0.lot": "b2", "disposals.0.fragments.1.lot": "b3", "years.2024.short_gain": "40000.00"},
    },
    {
        "id": "us-10-per-wallet-after-transfer",
        "title": "Per wallet: a lot moved away is not the source wallet's any more (RP2's per-wallet engine sells it twice)",
        "rules": ["US-POOL-WALLET", "US-TRANSFER", "US-RP2-PER-WALLET"],
        "policy": WALLET,
        "events": [
            buy("b1", "2025-01-02T12:00:00+00:00", "1", "100000.00"),
            buy("b2", "2025-02-01T12:00:00+00:00", "1", "200000.00"),
            move("m1", "2025-03-01T12:00:00+00:00", "1", "A", "B"),
            sell("s1", "2025-04-01T12:00:00+00:00", "1", "400000.00"),
            sell("s2", "2025-05-01T12:00:00+00:00", "1", "400000.00", w="B"),
        ],
        "hand_check": {
            "disposals.0.fragments.0.lot": "b2",
            "disposals.1.fragments.0.lot": "b1",
            "years.2025.short_cost": "300000.00",
        },
    },
    {
        "id": "us-11-transfer-fee-and-tacked-holding",
        "title": "A move's fee units leave with their basis and realize nothing; the moved units keep their acquisition date and basis",
        "rules": ["US-TRANSFER", "US-TRANSFER-FEE", "US-HOLD-TACK"],
        "policy": WALLET,
        "events": [
            buy("b1", "2023-05-01T12:00:00+00:00", "0.4", "11200.00"),
            buy("b2", "2023-09-01T12:00:00+00:00", "0.6", "15600.00"),
            move("m1", "2024-03-01T12:00:00+00:00", "0.5", "A", "B", fee_q="0.0001", fee_v="6.20"),
            sell("s1", "2024-06-01T12:00:00+00:00", "0.5", "35000.00", w="B"),
        ],
        # Fee 0.0001 from b1 (unit 28000): its 2.80 basis leaves with it, no
        # gain or loss. Move: 0.3999 of b1 then 0.1001 of b2. Sale in B: the
        # b1 part long (2023-05-01 to 2024-06-01), the b2 part short.
        "hand_check": {
            "fees.0.cost": "2.80",
            "fees.0.value": "6.20",
            "years.2024.network_fee_cost": "2.80",
            "disposals.0.fragments.0.lot": "b1",
            "disposals.0.fragments.0.quantity": "0.3999",
            "disposals.0.fragments.0.cost": "11197.20",
            "disposals.0.fragments.0.term": "long",
            "disposals.0.fragments.1.quantity": "0.1001",
            "disposals.0.fragments.1.cost": "2602.60",
            "disposals.0.fragments.1.term": "short",
        },
    },
    {
        "id": "us-12-global-scope-before-2025",
        "title": "Global (universal) scope before 2025: the earliest lot of any wallet goes first",
        "rules": ["US-POOL-GLOBAL", "US-METHOD-FIFO"],
        "policy": GLOBAL,
        "events": [
            buy("b1", "2023-01-05T12:00:00+00:00", "1", "17000.00", w="Cold"),
            buy("b2", "2023-06-05T12:00:00+00:00", "1", "26000.00", w="Hot"),
            sell("s1", "2024-02-05T12:00:00+00:00", "1", "43000.00", w="Hot"),
        ],
        "hand_check": {"disposals.0.fragments.0.lot": "b1", "disposals.0.fragments.0.term": "long", "years.2024.long_gain": "26000.00"},
    },
    {
        "id": "us-13-global-scope-refused-from-2025",
        "title": "Global scope with a 2025 disposal fails closed: § 1.1012-1(j) identifies per wallet from 2025",
        "rules": ["US-POOL-GLOBAL", "US-POOL-2025"],
        "policy": GLOBAL,
        "events": [
            buy("b1", "2024-01-05T12:00:00+00:00", "1", "45000.00", w="Cold"),
            sell("s1", "2025-01-01T00:00:00+00:00", "0.1", "9300.00", w="Hot"),
        ],
    },
    {
        "id": "us-14-staking-income-then-sale",
        "title": "Staking rewards are ordinary income at fair market value on receipt, which is also their basis (Rev. Rul. 2023-14)",
        "rules": ["US-INCOME", "US-INCOME-BASIS", "US-HOLD-1Y"],
        "policy": WALLET,
        "events": [
            inc("r1", "2024-02-10T12:00:00+00:00", "0.01", "480.00", "staking"),
            inc("r2", "2024-05-10T12:00:00+00:00", "0.01", "610.00", "staking"),
            sell("s1", "2025-03-01T12:00:00+00:00", "0.015", "1350.00"),
        ],
        "hand_check": {
            "years.2024.ordinary_income": "1090.00",
            "years.2024.income_by_kind.staking": "1090.00",
            "years.2024.digital_asset_question": "yes",
            "disposals.0.fragments.0.term": "long",
            "disposals.0.fragments.1.term": "short",
            "years.2025.long_gain": "420.00",
            "years.2025.short_gain": "145.00",
        },
    },
    {
        "id": "us-15-income-kinds",
        "title": "Mining, interest and airdrops are ordinary income by kind (Notice 2014-21, Rev. Rul. 2019-24)",
        "rules": ["US-INCOME", "US-INCOME-AIRDROP"],
        "policy": WALLET,
        "events": [
            inc("m1", "2024-01-15T12:00:00+00:00", "0.02", "850.00", "mining"),
            inc("i1", "2024-02-15T12:00:00+00:00", "0.001", "52.00", "interest"),
            inc("a1", "2024-03-15T12:00:00+00:00", "0.0005", "35.00", "airdrop_service", type_="airdrop"),
        ],
        "hand_check": {"years.2024.ordinary_income": "937.00", "years.2024.income_by_kind.mining": "850.00"},
    },
    {
        "id": "us-16-sale-fee-units",
        "title": "A sale's network fee units are disposed of with it at the sale's USD amount",
        "rules": ["US-EVENT-SELL", "US-FEE-SALE"],
        "policy": WALLET,
        "events": [
            buy("b1", "2024-01-02T12:00:00+00:00", "1", "45000.00"),
            sell("s1", "2024-05-02T12:00:00+00:00", "0.4999", "31000.00", fee_q="0.0001"),
        ],
        "hand_check": {"disposals.0.fragments.0.quantity": "0.5", "disposals.0.fragments.0.cost": "22500.00", "years.2024.short_gain": "8500.00"},
    },
    {
        "id": "us-17-spend-is-a-disposal",
        "title": "Paying for goods or services with bitcoin disposes of it at the value received",
        "rules": ["US-EVENT-SPEND"],
        "policy": WALLET,
        "events": [
            buy("b1", "2022-11-20T12:00:00+00:00", "0.1", "1650.00"),
            sell("p1", "2024-04-20T12:00:00+00:00", "0.01", "640.00", kind="spend"),
        ],
        "hand_check": {"disposals.0.kind": "spend", "years.2024.long_gain": "475.00"},
    },
    {
        "id": "us-18-boxes-by-year",
        "title": "Form 8949 boxes: C/F before 2025, I/L for 2025 and 2026; no table yet for 2027 (unknown)",
        "rules": ["US-8949", "US-8949-BOX", "US-YEAR-UNKNOWN"],
        "policy": WALLET,
        "events": [
            buy("b1", "2023-01-02T12:00:00+00:00", "3", "51000.00"),
            sell("s1", "2024-07-01T12:00:00+00:00", "1", "62000.00"),
            sell("s2", "2025-07-01T12:00:00+00:00", "1", "107000.00"),
            sell("s3", "2027-07-01T12:00:00+00:00", "1", "150000.00"),
        ],
        "hand_check": {
            "years.2024.form_8949_long_box": "F",
            "years.2025.form_8949_long_box": "L",
            "years.2025.schedule_d_line_10": "90000.00",
            "years.2027.status": "unknown",
        },
    },
    {
        "id": "us-19-rev-proc-2024-28-wallets",
        "title": "Rev. Proc. 2024-28 example facts per wallet: each wallet sells its own units",
        "rules": ["US-POOL-WALLET", "US-REVPROC-2024-28"],
        "policy": WALLET,
        "events": [
            buy("b1", "2019-07-01T12:00:00+00:00", "10", "10.00", w="XYZ"),
            buy("b2", "2020-09-01T12:00:00+00:00", "20", "100.00", w="GHI"),
            sell("s1", "2024-12-01T12:00:00+00:00", "6", "60.00", w="XYZ"),
            sell("s2", "2025-03-01T12:00:00+00:00", "2", "20.00", w="XYZ"),
        ],
        # Units at $1 (XYZ) and $5 (GHI); the 2024 sale uses XYZ's own $1 units.
        "hand_check": {
            "disposals.0.fragments.0.cost": "6.00",
            "disposals.1.fragments.0.cost": "2.00",
            "holdings.XYZ.BTC.quantity": "2",
            "holdings.GHI.BTC.cost": "100.00",
        },
    },
    {
        "id": "us-20-insufficient-wallet-lots",
        "title": "Per wallet, a sale larger than the wallet's units fails closed even when other wallets hold enough",
        "rules": ["US-POOL-WALLET", "US-FAIL-CLOSED"],
        "policy": WALLET,
        "events": [
            buy("b1", "2025-01-10T12:00:00+00:00", "1", "95000.00", w="A"),
            buy("b2", "2025-01-11T12:00:00+00:00", "1", "96000.00", w="B"),
            sell("s1", "2025-02-01T12:00:00+00:00", "1.5", "150000.00", w="A"),
        ],
    },
    {
        "id": "us-21-move-back-merges-portions",
        "title": "A lot that moves away and back is one portion again; FIFO keeps acquisition order",
        "rules": ["US-TRANSFER", "US-METHOD-FIFO"],
        "policy": WALLET,
        "events": [
            buy("b1", "2025-01-02T12:00:00+00:00", "1", "94000.00"),
            buy("b2", "2025-01-03T12:00:00+00:00", "1", "97000.00"),
            move("m1", "2025-02-01T12:00:00+00:00", "0.5", "A", "B"),
            move("m2", "2025-03-01T12:00:00+00:00", "0.5", "B", "A"),
            sell("s1", "2025-04-01T12:00:00+00:00", "1.2", "100000.00"),
        ],
        "hand_check": {"disposals.0.fragments.0.lot": "b1", "disposals.0.fragments.0.quantity": "1", "disposals.0.fragments.1.quantity": "0.2"},
    },
    {
        "id": "us-22-same-instant-move-funds-sale",
        "title": "At one instant, acquisitions, then moves, then disposals: a move funds a same-instant sale at its destination",
        "rules": ["US-TRANSFER", "US-ORDER"],
        "policy": WALLET,
        "events": [
            buy("b1", "2025-05-01T12:00:00+00:00", "1", "96000.00"),
            sell("s1", "2025-06-01T12:00:00+00:00", "0.5", "52500.00", w="B"),
            move("m1", "2025-06-01T12:00:00+00:00", "0.5", "A", "B"),
        ],
        "hand_check": {"disposals.0.fragments.0.lot": "b1", "years.2025.short_gain": "4500.00"},
    },
]
