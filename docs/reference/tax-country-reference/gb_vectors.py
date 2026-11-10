"""Inputs for the United Kingdom (gb) vectors; expected values come from gb.py.

gb-01..gb-08 replay HMRC's published worked examples (CRYPTO22251-22257 and
CRYPTO22280); HMRC gives no year ("20XX"), so a year is chosen and noted.
"""

P = {"method": "uk_share_matching", "pool_scope": "global", "rail_policy": None}


def buy(i, t, q, gbp, asset="BTC", w="A", fee=None):
    e = {"id": i, "time": t, "wallet": w, "type": "buy", "asset": asset, "quantity": q, "fiat": gbp}
    if fee:
        e["fee_fiat"] = fee
    return e


def sell(i, t, q, gbp, asset="BTC", w="A", fee=None, fee_q=None, fee_v=None):
    e = {"id": i, "time": t, "wallet": w, "type": "sell", "asset": asset, "quantity": q, "fiat": gbp}
    if fee:
        e["fee_fiat"] = fee
    if fee_q:
        e["fee_quantity"], e["fee_fiat_value"] = fee_q, fee_v
    return e


def swap(i, t, give, gq, gv, get, rq, rv, fee=None, w="A"):
    e = {"id": i, "time": t, "wallet": w, "type": "swap", "asset": give, "quantity": gq, "fiat": gv,
         "in_asset": get, "in_quantity": rq, "in_fiat": rv}
    if fee:
        e["fee_fiat"] = fee
    return e


def inc(i, t, q, gbp, kind, w="A"):
    return {"id": i, "time": t, "wallet": w, "type": "income", "subkind": kind, "asset": "BTC", "quantity": q, "fiat": gbp}


VECTORS = [
    {
        "id": "gb-01-hmrc-s104-basic",
        "title": "HMRC CRYPTO22251 (Victoria): basic section 104 pool disposal",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22251",
        "rules": ["GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-05-01T10:00:00+01:00", "100", "1000.00", asset="TOKA"),
            buy("b1", "2024-09-18T10:00:00+01:00", "50", "125000.00", asset="TOKA"),
            sell("s1", "2024-12-01T10:00:00+00:00", "50", "300000.00", asset="TOKA"),
        ],
        "hand_check": {"gains": {"TOKA@2024-12-01": 258000}, "pools": {"TOKA": ("100", 84000)}},
    },
    {
        "id": "gb-02-hmrc-same-day",
        "title": "HMRC CRYPTO22252 (Martyn): same-day rule, excess acquisition to the pool",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22252",
        "rules": ["GB-SAME-DAY", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-15T10:00:00+00:00", "5000", "500.00", asset="TOKB"),
            sell("s1", "2024-06-23T09:00:00+01:00", "1000", "800.00", asset="TOKB"),
            buy("b1", "2024-06-23T14:00:00+01:00", "1600", "1000.00", asset="TOKB"),
            sell("s2", "2024-06-23T20:00:00+01:00", "500", "600.00", asset="TOKB"),
        ],
        "hand_check": {"gains": {"TOKB@2024-06-23": 462}, "pools": {"TOKB": ("5100", 562)}},
    },
    {
        "id": "gb-03-hmrc-thirty-day",
        "title": "HMRC CRYPTO22253 (Rachel): 30-day rule across two disposals (and across the 5 April year end)",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22253",
        "rules": ["GB-30-DAY", "GB-30-DAY-ORDER", "GB-TAX-YEAR"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-10T10:00:00+00:00", "2000", "1000.00", asset="TOKC"),
            sell("s1", "2024-03-31T10:00:00+01:00", "1000", "400.00", asset="TOKC"),
            sell("s2", "2024-04-20T10:00:00+01:00", "500", "150.00", asset="TOKC"),
            buy("b1", "2024-04-21T10:00:00+01:00", "700", "175.00", asset="TOKC"),
            buy("b2", "2024-04-28T10:00:00+01:00", "500", "100.00", asset="TOKC"),
            buy("b3", "2024-05-01T10:00:00+01:00", "500", "150.00", asset="TOKC"),
        ],
        "hand_check": {"gains": {"TOKC@2024-03-31": 165, "TOKC@2024-04-20": 20}, "pools": {"TOKC": ("2200", 1060)}},
    },
    {
        "id": "gb-04-hmrc-same-day-and-pool",
        "title": "HMRC CRYPTO22254 (Daniel): same-day rule then part disposal from the pool",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22254",
        "rules": ["GB-SAME-DAY", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-06-01T10:00:00+01:00", "8000", "1000.00", asset="TOKD"),
            sell("s1", "2025-01-31T09:00:00+00:00", "5000", "500.00", asset="TOKD"),
            buy("b1", "2025-01-31T10:00:00+00:00", "4000", "320.00", asset="TOKD"),
            buy("b2", "2025-01-31T11:00:00+00:00", "1000", "75.00", asset="TOKD"),
            buy("b3", "2025-01-31T12:00:00+00:00", "1000", "70.00", asset="TOKD"),
            sell("s2", "2025-01-31T13:00:00+00:00", "2000", "142.00", asset="TOKD"),
            buy("b4", "2025-01-31T14:00:00+00:00", "500", "35.00", asset="TOKD"),
        ],
        "hand_check": {"gains": {"TOKD@2025-01-31": 79}, "pools": {"TOKD": ("7500", 937)}},
    },
    {
        "id": "gb-05-hmrc-thirty-day-and-pool",
        "title": "HMRC CRYPTO22255 (Melanie): 30-day rule then the pool",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22255",
        "rules": ["GB-30-DAY", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-10T10:00:00+00:00", "14000", "200000.00", asset="TOKE"),
            sell("s1", "2024-08-30T10:00:00+01:00", "4000", "160000.00", asset="TOKE"),
            buy("b1", "2024-09-11T10:00:00+01:00", "500", "17500.00", asset="TOKE"),
        ],
        "hand_check": {"gains": {"TOKE@2024-08-30": 92500}, "pools": {"TOKE": ("10500", 150000)}},
    },
    {
        "id": "gb-06-hmrc-all-three-rules",
        "title": "HMRC CRYPTO22256 (Gulferaz): same-day, 30-day and pool interacting",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22256",
        "rules": ["GB-SAME-DAY", "GB-30-DAY", "GB-30-DAY-ORDER", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-10T10:00:00+00:00", "100000", "300000.00", asset="TOKF"),
            buy("b1", "2024-07-31T09:00:00+01:00", "10000", "45000.00", asset="TOKF"),
            sell("s1", "2024-07-31T10:00:00+01:00", "30000", "150000.00", asset="TOKF"),
            sell("s2", "2024-08-05T10:00:00+01:00", "20000", "100000.00", asset="TOKF"),
            buy("b2", "2024-08-06T10:00:00+01:00", "50000", "225000.00", asset="TOKF"),
            sell("s3", "2024-08-07T10:00:00+01:00", "100000", "150000.00", asset="TOKF"),
        ],
        "hand_check": {
            "gains": {"TOKF@2024-07-31": 15000, "TOKF@2024-08-05": 10000, "TOKF@2024-08-07": -163637},
            "pools": {"TOKF": ("10000", 31363)},
        },
    },
    {
        "id": "gb-07-hmrc-crypto-to-crypto",
        "title": "HMRC CRYPTO22257 (Elina): token-to-token swaps touching two pools",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22257",
        "rules": ["GB-SWAP", "GB-SWAP-VALUES", "GB-SAME-DAY", "GB-30-DAY", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-10T10:00:00+00:00", "100000", "300000.00", asset="TOKG"),
            swap("x1", "2024-08-31T09:00:00+01:00", "TOKG", "1000", "3200.00", "TOKH", "10000", "3200.00"),
            swap("x2", "2024-08-31T10:00:00+01:00", "TOKH", "5000", "1700.00", "TOKG", "600", "1920.00"),
            swap("x3", "2024-08-31T11:00:00+01:00", "TOKG", "550", "1760.00", "TOKH", "5000", "1650.00"),
            swap("x4", "2024-09-04T10:00:00+01:00", "TOKH", "2000", "560.00", "TOKG", "180", "558.00"),
            swap("x5", "2024-09-16T10:00:00+01:00", "TOKG", "400", "1080.00", "TOKH", "4000", "1080.00"),
            swap("x6", "2024-10-27T10:00:00+00:00", "TOKH", "12000", "2400.00", "TOKG", "900", "2430.00"),
        ],
        "hand_check": {
            "gains": {"TOKG@2024-08-31": 280, "TOKH@2024-08-31": 267, "TOKH@2024-09-04": 18, "TOKG@2024-09-16": -120, "TOKH@2024-10-27": -1417},
            "pools": {"TOKG": ("99730", 298890), "TOKH": ("0", 0)},
        },
    },
    {
        "id": "gb-08-hmrc-fee-in-tokens",
        "title": "HMRC CRYPTO22280 (Terri): a fee paid in the same token is a same-day disposal at market value",
        "source": "https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22280",
        "rules": ["GB-FEE-TOKENS", "GB-SAME-DAY"],
        "policy": P,
        "events": [
            buy("b0", "2024-01-10T10:00:00+00:00", "10000", "20000.00", asset="TOKT"),
            sell("s1", "2024-07-01T10:00:00+01:00", "1000", "5000.00", asset="TOKT", fee_q="1", fee_v="5.00"),
        ],
        "hand_check": {"gains": {"TOKT@2024-07-01": 2998}, "pools": {"TOKT": ("8999", 17998)}},
    },
    {
        "id": "gb-09-tax-year-boundary-bst",
        "title": "23:30Z on 5 April 2025 is 00:30 BST on 6 April: the disposal falls in 2025-26",
        "rules": ["GB-TAX-YEAR", "GB-TZ"],
        "policy": P,
        "events": [
            buy("b1", "2024-01-10T10:00:00+00:00", "1", "30000.00"),
            sell("s1", "2025-04-05T23:30:00Z", "0.1", "7000.00"),
        ],
    },
    {
        "id": "gb-10-thirty-day-window-edge",
        "title": "An acquisition on day 30 after the disposal is matched; day 31 goes to the pool",
        "rules": ["GB-30-DAY", "GB-30-DAY-WINDOW"],
        "policy": P,
        "events": [
            buy("b1", "2024-01-10T10:00:00+00:00", "1", "30000.00"),
            sell("s1", "2024-06-01T10:00:00+01:00", "0.5", "35000.00"),
            buy("b2", "2024-07-01T10:00:00+01:00", "0.2", "12000.00"),
            buy("b3", "2024-07-02T10:00:00+01:00", "0.2", "12500.00"),
        ],
    },
    {
        "id": "gb-11-bed-and-breakfast-loss",
        "title": "Selling at a loss and rebuying within 30 days: the loss is measured against the rebuy, not the pool",
        "rules": ["GB-30-DAY"],
        "policy": P,
        "events": [
            buy("b1", "2024-01-10T10:00:00+00:00", "1", "50000.00"),
            sell("s1", "2024-08-05T10:00:00+01:00", "1", "40000.00"),
            buy("b2", "2024-08-20T10:00:00+01:00", "1", "41000.00"),
        ],
    },
    {
        "id": "gb-12-one-pool-across-wallets",
        "title": "One section 104 pool per token for the person, across wallets",
        "rules": ["GB-POOL-GLOBAL", "GB-POOL-S104"],
        "policy": P,
        "events": [
            buy("b1", "2024-02-01T10:00:00+00:00", "0.5", "10000.00", w="A"),
            buy("b2", "2024-03-01T10:00:00+00:00", "0.5", "20000.00", w="B"),
            sell("s1", "2024-09-01T10:00:00+01:00", "0.4", "24000.00", w="B"),
        ],
    },
    {
        "id": "gb-13-swap-fee-split",
        "title": "A token swap fee is split 50/50 between the disposal and the acquisition (CRYPTO22150)",
        "rules": ["GB-SWAP", "GB-FEE-SWAP"],
        "policy": P,
        "events": [
            buy("b1", "2024-01-10T10:00:00+00:00", "1", "30000.00"),
            swap("x1", "2024-05-01T10:00:00+01:00", "BTC", "0.5", "24850.00", "USDT", "31000", "24800.00", fee="50.00"),
            sell("s1", "2024-07-01T10:00:00+01:00", "31000", "24900.00", asset="USDT"),
        ],
    },
    {
        "id": "gb-14-own-transfer-fee-disposal",
        "title": "Moving coins between own wallets is no disposal; the network fee tokens are",
        "rules": ["GB-TRANSFER", "GB-FEE-TRANSFER"],
        "policy": P,
        "events": [
            buy("b1", "2024-01-10T10:00:00+00:00", "1", "30000.00"),
            {"id": "t1", "time": "2024-03-01T10:00:00+00:00", "wallet": "A", "type": "transfer", "asset": "BTC", "quantity": "0.5",
             "to_wallet": "B", "fee_quantity": "0.0001", "fee_fiat_value": "4.50"},
        ],
    },
    {
        "id": "gb-15-misc-income-into-pool",
        "title": "Non-trade mining and routing income are miscellaneous income; the coins join the pool at that value",
        "rules": ["GB-INC-MISC", "GB-INC-ACQ"],
        "policy": P,
        "events": [
            inc("i1", "2024-05-10T10:00:00+01:00", "0.01", "400.00", "mining"),
            inc("i2", "2024-06-10T10:00:00+01:00", "0.002", "90.00", "routing_income"),
            sell("s1", "2024-09-01T10:00:00+01:00", "0.012", "600.00"),
        ],
    },
    {
        "id": "gb-16-aea-2024-25-exactly",
        "title": "Net gains of exactly £3,000 in 2024-25: nothing above the annual exempt amount",
        "rules": ["GB-AEA"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "0.1", "5000.00"),
            sell("s1", "2024-12-01T10:00:00+00:00", "0.1", "8000.00"),
        ],
    },
    {
        "id": "gb-17-aea-2023-24-crossed",
        "title": "2023-24 net gains of £6,000.01 exceed that year's £6,000 by one penny",
        "rules": ["GB-AEA", "GB-REPORT-THRESHOLD"],
        "policy": P,
        "events": [
            buy("b1", "2023-05-01T10:00:00+01:00", "0.2", "4000.00"),
            sell("s1", "2023-12-01T10:00:00+00:00", "0.2", "10000.01"),
        ],
    },
    {
        "id": "gb-18-losses-offset-before-aea",
        "title": "Same-year losses reduce gains before the annual exempt amount; the loss still has to be reported",
        "rules": ["GB-LOSS", "GB-AEA"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "0.1", "5000.00", asset="BTC"),
            buy("b2", "2024-05-01T10:00:00+01:00", "1000", "9000.00", asset="TOKX"),
            sell("s1", "2024-08-01T10:00:00+01:00", "0.1", "10000.00", asset="BTC"),
            sell("s2", "2024-09-01T10:00:00+01:00", "1000", "5000.00", asset="TOKX"),
        ],
    },
    {
        "id": "gb-19-rate-change-split-2024-25",
        "title": "2024-25 gains are split at 30 October 2024 for the box 51 rate adjustment",
        "rules": ["GB-RATE-SPLIT", "GB-TZ"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "1", "40000.00"),
            sell("s1", "2024-10-29T23:30:00Z", "0.1", "6000.00"),
            sell("s2", "2024-10-30T00:30:00Z", "0.1", "5500.00"),
        ],
    },
    {
        "id": "gb-20-proceeds-threshold",
        "title": "Proceeds over £50,000 require the SA108 pages even when the gain is small",
        "rules": ["GB-REPORT-THRESHOLD"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "1", "49900.00"),
            sell("s1", "2024-11-01T10:00:00+00:00", "1", "50000.01"),
        ],
    },
    {
        "id": "gb-21-insufficient-holdings-refused",
        "title": "A disposal larger than every matching source fails closed",
        "rules": ["GB-FAILCLOSED"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "0.1", "5000.00"),
            sell("s1", "2024-06-01T10:00:00+01:00", "0.2", "11000.00"),
        ],
    },
    {
        "id": "gb-22-airdrop-unclassified-refused",
        "title": "An airdrop needs a reviewed classification (service or not) before it can be booked",
        "rules": ["GB-INC-AIRDROP", "GB-FAILCLOSED"],
        "policy": P,
        "events": [
            {"id": "a1", "time": "2024-06-01T10:00:00+01:00", "wallet": "A", "type": "airdrop", "subkind": "airdrop_unclassified", "asset": "TOKZ", "quantity": "100", "fiat": "40.00"},
        ],
    },
    {
        "id": "gb-23-gift-unclassified-refused",
        "title": "An outgoing gift needs the recipient class (spouse, charity, other) before it can be booked",
        "rules": ["GB-GIFT", "GB-FAILCLOSED"],
        "policy": P,
        "events": [
            buy("b1", "2024-05-01T10:00:00+01:00", "0.1", "5000.00"),
            {"id": "g1", "time": "2024-07-01T10:00:00+01:00", "wallet": "A", "type": "gift_out", "asset": "BTC", "quantity": "0.05", "fiat": "2800.00"},
        ],
    },
]
