"""Inputs for the Switzerland (ch) vectors; expected values come from ch.py.

Year-end values are illustrative inputs, not the published ESTV Kursliste
figures; `source` says which kind of value the implementation received.
"""

P = {"method": "fifo", "pool_scope": "global"}


def buy(i, t, q, chf, w="A", asset="BTC", fee=None):
    e = {"id": i, "time": t, "wallet": w, "type": "buy", "asset": asset, "quantity": q, "fiat": chf}
    if fee:
        e["fee_fiat"] = fee
    return e


def sell(i, t, q, chf, w="A", asset="BTC"):
    return {"id": i, "time": t, "wallet": w, "type": "sell", "asset": asset, "quantity": q, "fiat": chf}


def inc(i, t, q, chf, kind, w="A", asset="BTC"):
    return {"id": i, "time": t, "wallet": w, "type": "income", "subkind": kind, "asset": asset, "quantity": q, "fiat": chf}


def estv(year, chf, asset="BTC"):
    return {f"{year}-12-31": {asset: {"chf": chf, "source": "estv_kursliste"}}}


VECTORS = [
    {
        "id": "ch-01-private-gain-tax-free",
        "title": "A private capital gain is tax-free (Art. 16 Abs. 3 DBG); shown for information only",
        "rules": ["CH-CG-PRIVATE", "CH-KS36-C1", "CH-WEALTH"],
        "policy": P,
        "events": [buy("b1", "2024-01-10T10:00:00+01:00", "1", "40000.00"), sell("s1", "2024-09-01T10:00:00+02:00", "0.5", "30000.00")],
        "valuations": estv(2024, "80000.00"),
    },
    {
        "id": "ch-02-private-loss-not-deductible",
        "title": "A private capital loss is not deductible; taxable capital gains stay 0",
        "rules": ["CH-CG-PRIVATE"],
        "policy": P,
        "events": [buy("b1", "2024-01-10T10:00:00+01:00", "1", "40000.00"), sell("s1", "2024-09-01T10:00:00+02:00", "1", "35000.00")],
    },
    {
        "id": "ch-03-lending-interest-income",
        "title": "Interest from lending BTC is income from movable assets at its CHF value when received (Art. 20 DBG)",
        "rules": ["CH-INC-MOVABLE", "CH-INC-VALUE", "CH-WEALTH"],
        "policy": P,
        "events": [
            inc("i1", "2024-03-31T12:00:00+02:00", "0.002", "120.40", "lending_interest"),
            inc("i2", "2024-06-30T12:00:00+02:00", "0.002", "112.10", "lending_interest"),
            inc("i3", "2024-09-30T12:00:00+02:00", "0.002", "115.75", "lending_interest"),
        ],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-04-airdrop-income",
        "title": "An airdrop is income at its market value on allocation; the CHF 1,000 lottery exemption is not assumed",
        "rules": ["CH-INC-AIRDROP"],
        "policy": P,
        "events": [{"id": "a1", "time": "2024-05-02T10:00:00+02:00", "wallet": "A", "type": "airdrop", "subkind": "airdrop_unclassified", "asset": "TOKZ", "quantity": "500", "fiat": "640.00"}],
    },
    {
        "id": "ch-05-mining-income",
        "title": "Mining rewards are income; whether they are self-employment income needs review",
        "rules": ["CH-INC-MINING"],
        "policy": P,
        "events": [inc("i1", "2024-02-01T10:00:00+01:00", "0.01", "420.00", "mining"), inc("i2", "2024-08-01T10:00:00+02:00", "0.01", "550.00", "mining")],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-06-wealth-estv-value",
        "title": "Year-end wealth at the ESTV tax value per 31 December, per wallet",
        "rules": ["CH-WEALTH", "CH-WEALTH-ESTV"],
        "policy": P,
        "events": [
            buy("b1", "2024-02-01T10:00:00+01:00", "0.5", "21000.00", w="Cold"),
            buy("b2", "2024-04-01T10:00:00+02:00", "0.3", "18000.00", w="Hot"),
        ],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-07-wealth-market-value",
        "title": "Without an ESTV value the market close is used and labelled as not the ESTV tax value",
        "rules": ["CH-WEALTH", "CH-WEALTH-FALLBACK"],
        "policy": P,
        "events": [buy("b1", "2024-02-01T10:00:00+01:00", "0.5", "21000.00")],
        "valuations": {"2024-12-31": {"BTC": {"chf": "85210.33", "source": "market_close"}}},
    },
    {
        "id": "ch-08-wealth-missing-value",
        "title": "No year-end value at all: wealth stays unknown (never 0, never silently cost)",
        "rules": ["CH-WEALTH", "CH-FAILCLOSED"],
        "policy": P,
        "events": [buy("b1", "2024-02-01T10:00:00+01:00", "0.5", "21000.00")],
    },
    {
        "id": "ch-09-new-year-zurich",
        "title": "23:30Z on 31 December is 00:30 on 1 January in Zurich: the receipt belongs to the next year",
        "rules": ["CH-TZ", "CH-WEALTH", "CH-INC-VALUE"],
        "policy": P,
        "events": [buy("b1", "2024-02-01T10:00:00+01:00", "0.5", "21000.00"), inc("i1", "2024-12-31T23:30:00Z", "0.001", "88.00", "lending_interest")],
        "valuations": {**estv(2024, "84000.00"), **estv(2025, "90000.00")},
    },
    {
        "id": "ch-10-ks36-holding-under-6-months",
        "title": "KS 36 criterion 1: a sale one day short of six months fails the safe harbour",
        "rules": ["CH-KS36-C1"],
        "policy": P,
        "events": [buy("b1", "2024-01-31T10:00:00+01:00", "1", "38000.00"), sell("s1", "2024-07-30T10:00:00+02:00", "1", "55000.00")],
    },
    {
        "id": "ch-11-ks36-holding-exactly-6-months",
        "title": "KS 36 criterion 1: bought 31 August, sold 28 February (month-end) counts as six months",
        "rules": ["CH-KS36-C1"],
        "policy": P,
        "events": [buy("b1", "2024-08-31T10:00:00+02:00", "1", "50000.00"), sell("s1", "2025-02-28T10:00:00+01:00", "1", "80000.00")],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-12-ks36-volume-at-limit",
        "title": "KS 36 criterion 2: purchases plus sales equal to five times the start-of-year value are within the limit",
        "rules": ["CH-KS36-C2"],
        "policy": P,
        "events": [
            buy("b0", "2023-06-01T10:00:00+02:00", "1", "25000.00"),
            buy("b1", "2024-03-01T10:00:00+01:00", "1", "60000.00"),
            sell("s1", "2024-10-01T10:00:00+02:00", "1.5", "120000.00"),
        ],
        "valuations": {**estv(2023, "36000.00"), **estv(2024, "84000.00")},
    },
    {
        "id": "ch-13-ks36-volume-over-limit",
        "title": "KS 36 criterion 2: one centime over five times the start-of-year value fails",
        "rules": ["CH-KS36-C2"],
        "policy": P,
        "events": [
            buy("b0", "2023-06-01T10:00:00+02:00", "1", "25000.00"),
            buy("b1", "2024-03-01T10:00:00+01:00", "1", "60000.00"),
            sell("s1", "2024-10-01T10:00:00+02:00", "1.5", "120000.01"),
        ],
        "valuations": {**estv(2023, "36000.00"), **estv(2024, "84000.00")},
    },
    {
        "id": "ch-14-ks36-gains-half-income",
        "title": "KS 36 criterion 3: realized gains equal to half the net income are not 'less than 50%'",
        "rules": ["CH-KS36-C3"],
        "policy": {**P, "net_income_chf": "60000.00", "declared": {"debt_financed_without_cover": False, "derivatives_only_hedging": True}},
        "events": [buy("b1", "2023-01-10T10:00:00+01:00", "1", "20000.00"), sell("s1", "2024-03-01T10:00:00+01:00", "1", "50000.00")],
        "valuations": estv(2023, "38000.00"),
    },
    {
        "id": "ch-15-ks36-all-met",
        "title": "Every criterion met when the user declares income, financing and derivatives",
        "rules": ["CH-KS36-C1", "CH-KS36-C2", "CH-KS36-C3", "CH-KS36-C4", "CH-KS36-C5"],
        "policy": {**P, "net_income_chf": "120000.00", "declared": {"debt_financed_without_cover": False, "derivatives_only_hedging": True}},
        "events": [buy("b1", "2023-01-10T10:00:00+01:00", "1", "20000.00"), sell("s1", "2024-03-01T10:00:00+01:00", "0.5", "30000.00")],
        "valuations": {**estv(2023, "38000.00"), **estv(2024, "84000.00")},
    },
    {
        "id": "ch-16-gift-received-not-income",
        "title": "A received gift is not income; the coins are wealth",
        "rules": ["CH-GIFT", "CH-WEALTH"],
        "policy": P,
        "events": [{"id": "g1", "time": "2024-04-01T10:00:00+02:00", "wallet": "A", "type": "gift_in", "asset": "BTC", "quantity": "0.1", "fiat": "6100.00"}],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-17-wages-in-btc",
        "title": "Salary paid in BTC is employment income on the Lohnausweis, outside Kassiber's income total",
        "rules": ["CH-WAGES"],
        "policy": P,
        "events": [{"id": "w1", "time": "2024-01-25T10:00:00+01:00", "wallet": "A", "type": "wages", "asset": "BTC", "quantity": "0.05", "fiat": "2000.00"}],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-18-lbtc-valued-as-btc",
        "title": "L-BTC has no ESTV value; it is valued at the BTC tax value and flagged",
        "rules": ["CH-WEALTH", "CH-RAIL"],
        "policy": P,
        "events": [
            buy("b1", "2024-02-01T10:00:00+01:00", "0.5", "21000.00"),
            {"id": "g1", "time": "2024-03-01T10:00:00+01:00", "wallet": "A", "type": "peg", "asset": "BTC", "quantity": "0.2", "in_asset": "LBTC", "in_quantity": "0.2"},
        ],
        "valuations": estv(2024, "84000.00"),
    },
    {
        "id": "ch-19-hardfork-refused",
        "title": "A hard fork fails closed: no ESTV rule settles its income or cost treatment",
        "rules": ["CH-HARDFORK", "CH-FAILCLOSED"],
        "policy": P,
        "events": [buy("b1", "2017-01-10T10:00:00+01:00", "1", "900.00"), {"id": "h1", "time": "2017-08-01T15:16:00+02:00", "wallet": "A", "type": "hardfork", "asset": "BCH", "quantity": "1"}],
    },
    {
        "id": "ch-20-professional-trader-refused",
        "title": "Business-asset (professional trader) accounting is not offered: fail closed",
        "rules": ["CH-PROFESSIONAL", "CH-FAILCLOSED"],
        "policy": {**P, "professional_trader": True},
        "events": [buy("b1", "2024-01-10T10:00:00+01:00", "1", "40000.00")],
    },
]
