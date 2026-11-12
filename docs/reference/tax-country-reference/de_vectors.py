"""Inputs for the Germany vectors; expected values come from de.py."""

FIFO = {"method": "fifo", "pool_scope": "wallet", "rail_policy": None}
AVG = {"method": "average", "pool_scope": "wallet", "rail_policy": None}


def buy(i, t, q, eur, w="A", fee=None, asset="BTC"):
    e = {"id": i, "time": t, "wallet": w, "type": "buy", "asset": asset, "quantity": q, "fiat": eur}
    if fee:
        e["fee_fiat"] = fee
    return e


def sell(i, t, q, eur, w="A", fee=None, fee_q=None, fee_v=None, asset="BTC", kind="sell"):
    e = {"id": i, "time": t, "wallet": w, "type": kind, "asset": asset, "quantity": q, "fiat": eur}
    if fee:
        e["fee_fiat"] = fee
    if fee_q:
        e["fee_quantity"], e["fee_fiat_value"] = fee_q, fee_v
    return e


def inc(i, t, q, eur, kind, w="A"):
    return {"id": i, "time": t, "wallet": w, "type": "income", "subkind": kind, "asset": "BTC", "quantity": q, "fiat": eur}


VECTORS = [
    {
        "id": "de-01-fifo-within-year",
        "title": "Sale within the year: taxable; acquisition fee raises cost, sale fee is Werbungskosten",
        "rules": ["DE-EVENT-SELL", "DE-HOLD-1Y", "DE-FEE-ACQ", "DE-FEE-DISP", "DE-FG-23"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "1", "40000.00", fee="40.00"),
            sell("s1", "2024-06-15T12:00:00+02:00", "0.5", "30000.00", fee="30.00"),
        ],
    },
    {
        "id": "de-02-anniversary-day-taxable",
        "title": "Disposal on the anniversary date is still within the year (taxable)",
        "rules": ["DE-HOLD-1Y", "DE-HOLD-CAL"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-03-15T09:00:00+01:00", "1", "20000.00"),
            sell("s1", "2024-03-15T18:00:00+01:00", "1", "60000.00"),
        ],
    },
    {
        "id": "de-03-day-after-anniversary-exempt",
        "title": "Disposal on the day after the anniversary is outside the period (not taxable)",
        "rules": ["DE-HOLD-1Y", "DE-HOLD-CAL"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-03-15T09:00:00+01:00", "1", "20000.00"),
            sell("s1", "2024-03-16T00:05:00+01:00", "1", "60000.00"),
        ],
    },
    {
        "id": "de-04-leap-day-acquisition",
        "title": "Acquired 29 Feb: the period ends 28 Feb of the next year (§188 Abs. 3 BGB)",
        "rules": ["DE-HOLD-CAL", "DE-HOLD-LEAP"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-02-29T12:00:00+01:00", "0.1", "5000.00"),
            sell("s1", "2025-02-28T12:00:00+01:00", "0.05", "4500.00"),
            sell("s2", "2025-03-01T12:00:00+01:00", "0.05", "4600.00"),
        ],
    },
    {
        "id": "de-05-365-days-still-taxable",
        "title": "365 elapsed days across 29 Feb are not more than one year: the generic day count would call it long-term",
        "rules": ["DE-HOLD-CAL", "DE-ENGINE-DAYCOUNT"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-03-01T10:00:00+01:00", "1", "21000.00"),
            sell("s1", "2024-02-29T10:00:00+01:00", "1", "55000.00"),
        ],
    },
    {
        "id": "de-06-berlin-calendar-day",
        "title": "Holding period uses Europe/Berlin dates: a UTC-date implementation gets the opposite answer",
        "rules": ["DE-HOLD-CAL", "DE-TZ"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-06-10T22:30:00Z", "0.2", "5000.00"),
            sell("s1", "2024-06-11T21:30:00Z", "0.2", "13000.00"),
        ],
    },
    {
        "id": "de-07-mixed-lots-expense-split",
        "title": "One sale consumes an old (exempt) and a new (taxable) lot; Werbungskosten split by quantity (Rn. 57)",
        "rules": ["DE-METHOD-FIFO", "DE-HOLD-1Y", "DE-FEE-SPLIT"],
        "policy": FIFO,
        "events": [
            buy("b1", "2022-05-01T10:00:00+02:00", "0.3", "9000.00"),
            buy("b2", "2024-01-20T10:00:00+01:00", "0.3", "12000.00"),
            sell("s1", "2024-09-01T10:00:00+02:00", "0.4", "24000.00", fee="40.00"),
        ],
    },
    {
        "id": "de-08-freigrenze-2024-below",
        "title": "2024 total gain 999.99 EUR is below the 1,000 EUR Freigrenze: not taxable",
        "rules": ["DE-FG-23", "DE-FG-23-2024"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-02-01T10:00:00+01:00", "0.1", "4000.00"),
            sell("s1", "2024-05-01T10:00:00+02:00", "0.1", "4999.99"),
        ],
    },
    {
        "id": "de-09-freigrenze-2024-at-limit",
        "title": "2024 total gain exactly 1,000.00 EUR is not below the Freigrenze: fully taxable",
        "rules": ["DE-FG-23", "DE-FG-23-2024"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-02-01T10:00:00+01:00", "0.1", "4000.00"),
            sell("s1", "2024-05-01T10:00:00+02:00", "0.1", "5000.00"),
        ],
    },
    {
        "id": "de-10-freigrenze-2023-600",
        "title": "2023 used the 600 EUR Freigrenze: a 600.00 gain is taxable (it would be exempt under the 2024 limit)",
        "rules": ["DE-FG-23", "DE-FG-23-2023"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-02-01T10:00:00+01:00", "0.1", "2000.00"),
            sell("s1", "2023-07-01T10:00:00+02:00", "0.1", "2600.00"),
        ],
    },
    {
        "id": "de-11-loss-netting-under-freigrenze",
        "title": "Same-year §23 loss nets against §23 gain; the net 800 EUR is below the Freigrenze",
        "rules": ["DE-LOSS-23", "DE-FG-23"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-05T10:00:00+01:00", "0.1", "4000.00"),
            buy("b2", "2024-03-01T10:00:00+01:00", "0.1", "6500.00"),
            sell("s1", "2024-04-01T10:00:00+02:00", "0.1", "5500.00"),
            sell("s2", "2024-08-01T10:00:00+02:00", "0.1", "5800.00"),
        ],
    },
    {
        "id": "de-12-net-loss-year",
        "title": "A net §23 loss is reported as a loss (only offsettable against §23 gains, §23 Abs. 3 Sätze 7-8)",
        "rules": ["DE-LOSS-23"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-03-01T10:00:00+01:00", "0.1", "6500.00"),
            sell("s1", "2024-08-01T10:00:00+02:00", "0.1", "6200.00"),
        ],
    },
    {
        "id": "de-13-swap-btc-usdt",
        "title": "Crypto-to-crypto swap: proceeds at the received token's value, its cost at the given token's value; new period for USDT",
        "rules": ["DE-EVENT-SWAP", "DE-SWAP-VALUES", "DE-HOLD-RESTART"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "0.5", "20000.00"),
            {"id": "x1", "time": "2024-04-10T10:00:00+02:00", "wallet": "A", "type": "swap", "asset": "BTC", "quantity": "0.25",
             "fiat": "14250.00", "in_asset": "USDT", "in_quantity": "15500", "in_fiat": "14300.00"},
            sell("s1", "2024-05-10T10:00:00+02:00", "15500", "14400.00", asset="USDT"),
        ],
    },
    {
        "id": "de-14-swap-received-value-unknown",
        "title": "Swap where the received token has no market value: the given token's value is used (Rn. 58 Satz 3)",
        "rules": ["DE-EVENT-SWAP", "DE-SWAP-FALLBACK"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "0.5", "20000.00"),
            {"id": "x1", "time": "2024-04-10T10:00:00+02:00", "wallet": "A", "type": "swap", "asset": "BTC", "quantity": "0.25",
             "fiat": "14250.00", "in_asset": "USDT", "in_quantity": "15500", "in_fiat": None},
        ],
    },
    {
        "id": "de-15-lending-income-under-256",
        "title": "Lending interest totalling 255.99 EUR is below the §22 Nr. 3 Freigrenze; the coins are acquisitions at that value",
        "rules": ["DE-INC-LENDING", "DE-FG-22-3", "DE-INC-ACQ", "DE-FG-23"],
        "policy": FIFO,
        "events": [
            inc("i1", "2024-02-01T10:00:00+01:00", "0.001", "60.00", "lending_interest"),
            inc("i2", "2024-06-01T10:00:00+02:00", "0.002", "130.00", "lending_interest"),
            inc("i3", "2024-11-01T10:00:00+01:00", "0.001", "65.99", "lending_interest"),
            sell("s1", "2024-12-01T10:00:00+01:00", "0.004", "380.00"),
        ],
    },
    {
        "id": "de-16-routing-income-at-256",
        "title": "Leistung income of exactly 256.00 EUR is not below the Freigrenze: fully taxable",
        "rules": ["DE-INC-ROUTING", "DE-FG-22-3"],
        "policy": FIFO,
        "events": [
            inc("i1", "2024-03-01T10:00:00+01:00", "0.002", "120.00", "routing_income"),
            inc("i2", "2024-09-01T10:00:00+02:00", "0.002", "136.00", "routing_income"),
        ],
    },
    {
        "id": "de-17-wallet-based-fifo",
        "title": "FIFO runs per wallet (Rn. 62): a sale from wallet B takes B's lot even though A holds an older one",
        "rules": ["DE-POOL-WALLET", "DE-METHOD-FIFO"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-01-10T10:00:00+01:00", "1", "15000.00", w="A"),
            buy("b2", "2024-02-01T10:00:00+01:00", "1", "38000.00", w="B"),
            sell("s1", "2024-07-01T10:00:00+02:00", "0.5", "28000.00", w="B"),
        ],
    },
    {
        "id": "de-18-own-transfer-keeps-dates-fee-is-disposal",
        "title": "Own-wallet transfer carries lots with their dates; the network fee units are a disposal at market value",
        "rules": ["DE-TRANSFER", "DE-FEE-TRANSFER", "DE-POOL-WALLET"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "1", "40000.00", w="A"),
            {"id": "t1", "time": "2024-03-01T10:00:00+01:00", "wallet": "A", "type": "transfer", "asset": "BTC", "quantity": "0.5",
             "to_wallet": "B", "fee_quantity": "0.0001", "fee_fiat_value": "5.80"},
            sell("s1", "2024-05-01T10:00:00+02:00", "0.5", "31000.00", w="B"),
        ],
    },
    {
        "id": "de-19-average-method",
        "title": "Rn. 61 main rule: FIFO decides the holding period, the wallet's average cost the value",
        "rules": ["DE-METHOD-AVG", "DE-HOLD-1Y"],
        "policy": AVG,
        "events": [
            buy("b1", "2023-01-10T10:00:00+01:00", "0.5", "8000.00"),
            buy("b2", "2024-02-01T10:00:00+01:00", "0.5", "20000.00"),
            sell("s1", "2024-06-01T10:00:00+02:00", "0.6", "36000.00"),
        ],
    },
    {
        "id": "de-20-spend-agreed-price",
        "title": "Paying for goods: proceeds are the agreed EUR price (Rn. 60)",
        "rules": ["DE-EVENT-SPEND"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "0.1", "4000.00"),
            sell("p1", "2024-03-01T10:00:00+01:00", "0.01", "650.00", kind="spend"),
        ],
    },
    {
        "id": "de-21-network-fee-on-sale",
        "title": "Network fee paid in BTC: the fee units are a disposal of their own (taken first), and their value is Werbungskosten split over the sale",
        "rules": ["DE-FEE-NETWORK", "DE-FEE-SPLIT", "DE-HOLD-1Y"],
        "policy": FIFO,
        "events": [
            buy("b1", "2023-01-10T10:00:00+01:00", "1", "15000.00"),
            buy("b2", "2024-02-01T10:00:00+01:00", "0.5", "20000.00"),
            sell("s1", "2024-06-01T10:00:00+02:00", "1", "60000.00", fee_q="0.0002", fee_v="12.00"),
        ],
    },
    {
        "id": "de-22-mining-needs-classification",
        "title": "Mining income is reported apart from §22 Nr. 3 until classified (Rn. 34-45: commercial or not)",
        "rules": ["DE-INC-MINING"],
        "policy": FIFO,
        "events": [inc("i1", "2024-05-01T10:00:00+02:00", "0.01", "600.00", "mining")],
    },
    {
        "id": "de-23-airdrop-unclassified-refused",
        "title": "An airdrop without a reviewed classification fails closed",
        "rules": ["DE-INC-AIRDROP", "DE-FAILCLOSED"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "0.1", "4000.00"),
            {"id": "a1", "time": "2024-02-10T10:00:00+01:00", "wallet": "A", "type": "airdrop", "subkind": "airdrop_unclassified",
             "asset": "XYZ", "quantity": "100", "fiat": "50.00"},
        ],
    },
    {
        "id": "de-24-rail-change-needs-policy",
        "title": "A BTC to LBTC peg fails closed until the book records an explicit decision",
        "rules": ["DE-RAIL", "DE-FAILCLOSED"],
        "policy": FIFO,
        "events": [
            buy("b1", "2024-01-10T10:00:00+01:00", "0.1", "4000.00"),
            {"id": "g1", "time": "2024-02-10T10:00:00+01:00", "wallet": "A", "type": "peg", "asset": "BTC", "quantity": "0.05",
             "in_asset": "LBTC", "in_quantity": "0.04999"},
        ],
    },
    {
        "id": "de-25-hardfork-refused",
        "title": "A hard fork needs a cost split by market values (Rn. 67), which is not modelled: fail closed",
        "rules": ["DE-HARDFORK", "DE-FAILCLOSED"],
        "policy": FIFO,
        "events": [
            buy("b1", "2017-01-10T10:00:00+01:00", "1", "900.00"),
            {"id": "h1", "time": "2017-08-01T15:16:00+02:00", "wallet": "A", "type": "hardfork", "asset": "BCH", "quantity": "1"},
        ],
    },
    {
        "id": "de-26-gift-without-donor-data-refused",
        "title": "Received gift without the donor's acquisition data fails closed (§23 Abs. 1 Satz 3)",
        "rules": ["DE-GIFT-IN", "DE-FAILCLOSED"],
        "policy": FIFO,
        "events": [
            {"id": "g1", "time": "2024-02-10T10:00:00+01:00", "wallet": "A", "type": "gift_in", "asset": "BTC", "quantity": "0.1", "fiat": "4200.00"},
        ],
    },
]
