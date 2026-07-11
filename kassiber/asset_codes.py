"""Shared asset-code vocabulary for exchange and broker imports."""

import re

BTC_ASSET_ALIASES = frozenset({"BTC", "XBT", "XXBT"})
LBTC_ASSET_ALIASES = frozenset({"LBTC", "L-BTC"})
BITCOIN_FAMILY_ASSETS = BTC_ASSET_ALIASES | LBTC_ASSET_ALIASES

FIAT_CURRENCIES = frozenset(
    {
        "AED",
        "ARS",
        "AUD",
        "BRL",
        "CAD",
        "CHF",
        "CLP",
        "CNY",
        "CZK",
        "DKK",
        "EUR",
        "GBP",
        "HKD",
        "HUF",
        "ILS",
        "INR",
        "JPY",
        "KRW",
        "MXN",
        "NOK",
        "NZD",
        "PLN",
        "RON",
        "SEK",
        "SGD",
        "THB",
        "TRY",
        "USD",
        "ZAR",
    }
)


def canonical_bitcoin_asset(value):
    code = str(value or "").strip().upper()
    if code in BTC_ASSET_ALIASES:
        return "BTC"
    if code in LBTC_ASSET_ALIASES:
        return "LBTC"
    return None


# Liquid asset ids are 64-hex strings (normalize_asset_code lowercases them).
_LIQUID_ASSET_ID_RE = re.compile(r"^[0-9a-f]{64}$")


def is_tax_engine_asset(value):
    """True for assets Kassiber's tax engine books.

    The tax core is Bitcoin-only: BTC, LBTC, and 64-hex Liquid asset ids
    (Liquid wallets can carry issued assets). Any other stored asset code is a
    legacy-holdings overlay asset — visible in overview surfaces but excluded
    from journals, capital gains, and every tax report.
    """
    code = str(value or "").strip()
    if not code:
        return False
    if code.upper() in {"BTC", "LBTC"}:
        return True
    return bool(_LIQUID_ASSET_ID_RE.fullmatch(code.lower()))


__all__ = [
    "BITCOIN_FAMILY_ASSETS",
    "BTC_ASSET_ALIASES",
    "FIAT_CURRENCIES",
    "LBTC_ASSET_ALIASES",
    "canonical_bitcoin_asset",
    "is_tax_engine_asset",
]
