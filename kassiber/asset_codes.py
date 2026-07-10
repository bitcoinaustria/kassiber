"""Shared asset-code vocabulary for exchange and broker imports."""

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


__all__ = [
    "BITCOIN_FAMILY_ASSETS",
    "BTC_ASSET_ALIASES",
    "FIAT_CURRENCIES",
    "LBTC_ASSET_ALIASES",
    "canonical_bitcoin_asset",
]
