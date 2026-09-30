"""Classify BTCPay payment methods, including plugin-provided ones.

BTCPay Server 2.x names payment methods ``{CURRENCY}-{HANDLER}``:
``BTC-CHAIN`` (on-chain), ``BTC-LN`` (BOLT11 Lightning), ``BTC-LNURL``
(LNURL-pay / Lightning Address), ``LBTC-CHAIN`` (Liquid). Plugins add more:
Liquid assets (``USDT-CHAIN``), altcoins (``XMR-CHAIN``, ``LTC-CHAIN``), and
new rails with their own handler names. Lightning plugins such as Boltz,
Breez, Blink, Strike, or Nostr Wallet Connect keep the ``BTC-LN`` id and only
change the connection behind it, which Kassiber deliberately never reads.

BTCPay 1.x used different names (``BTC``, ``BTC-LightningNetwork``,
``BTC_LNURLPAY``). ``canonical_payment_method_id`` maps them onto the 2.x
names so stored routes and invoice payments line up across versions.
"""

from __future__ import annotations

from typing import Any

from ..errors import AppError

WALLET_HISTORY_PAYMENT_METHOD_IDS = frozenset({"BTC-CHAIN", "LBTC-CHAIN"})
BITCOIN_ASSET_CHAINS = {"BTC": "bitcoin", "LBTC": "liquid"}

RAIL_ONCHAIN = "onchain"
RAIL_LIGHTNING = "lightning"
RAIL_LNURL = "lnurl"
RAIL_PLUGIN = "plugin"

_HANDLER_RAILS = {
    "CHAIN": RAIL_ONCHAIN,
    "ONCHAIN": RAIL_ONCHAIN,
    "BTCLIKE": RAIL_ONCHAIN,
    "LN": RAIL_LIGHTNING,
    "LIGHTNINGNETWORK": RAIL_LIGHTNING,
    "LIGHTNINGLIKE": RAIL_LIGHTNING,
    "LNURL": RAIL_LNURL,
    "LNURLPAY": RAIL_LNURL,
}
_CANONICAL_HANDLER = {
    RAIL_ONCHAIN: "CHAIN",
    RAIL_LIGHTNING: "LN",
    RAIL_LNURL: "LNURL",
}

SETTLEMENT_HINTS = {
    "btc_onchain": (
        "Customers pay into this store's on-chain wallet. Kassiber can import "
        "BTCPay's wallet history (needs the wallet-history key) or recognise "
        "the watch-only wallet you already track for this store."
    ),
    "liquid_onchain": (
        "Customers pay into this store's Liquid wallet. Kassiber imports it the "
        "same way as on-chain Bitcoin, or maps it to a Liquid wallet you track."
    ),
    "lightning": (
        "Lightning payments settle in the node or wallet configured for this "
        "store: your own LND or Core Lightning node, or a plugin such as Boltz "
        "(Liquid), Breez, Blink, Strike, or Nostr Wallet Connect. Add that node "
        "or wallet as its own connection; Kassiber links BTCPay payments to it "
        "by payment hash."
    ),
    "lnurl": (
        "LNURL and Lightning Address payments settle through the same Lightning "
        "connection as BOLT11 invoices. Kassiber keeps them as invoice records "
        "and links them to your Lightning node or wallet by payment hash."
    ),
    "plugin": (
        "This payment method comes from a BTCPay plugin Kassiber cannot import "
        "directly. Invoices stay available as provenance; add the account that "
        "receives the funds separately."
    ),
    "non_bitcoin": (
        "Kassiber tracks Bitcoin and Liquid Bitcoin only. Payments in this "
        "currency are skipped; invoices can still be kept as provenance."
    ),
}


def split_payment_method_id(value: Any) -> tuple[str, str]:
    raw = str(value or "").strip()
    if not raw:
        return "", ""
    for separator in ("-", "_"):
        if separator in raw:
            currency, handler = raw.split(separator, 1)
            return currency.strip().upper(), handler.strip().upper().replace("-", "").replace("_", "")
    # 1.x servers used a bare crypto code for on-chain methods.
    return raw.upper(), ""


def canonical_payment_method_id(value: Any) -> str:
    """Map legacy and differently-cased ids to BTCPay 2.x names."""

    raw = str(value or "").strip()
    if not raw:
        return ""
    currency, handler = split_payment_method_id(raw)
    if not handler:
        # A short bare crypto code (``BTC``) is a 1.x on-chain method; any
        # other bare id is a plugin rail whose name must stay intact.
        if len(currency) <= 5 and currency.isalpha():
            return f"{currency}-CHAIN"
        return raw.upper()
    rail = _HANDLER_RAILS.get(handler)
    if rail is None:
        return raw.upper()
    return f"{currency}-{_CANONICAL_HANDLER[rail]}"


def classify_payment_method(value: Any) -> dict[str, Any]:
    """Describe what a payment method means for Kassiber."""

    payment_method_id = canonical_payment_method_id(value)
    currency, handler = split_payment_method_id(payment_method_id)
    rail = _HANDLER_RAILS.get(handler, RAIL_PLUGIN) if handler else RAIL_PLUGIN
    chain = BITCOIN_ASSET_CHAINS.get(currency)
    bitcoin_asset = chain is not None
    if rail in {RAIL_LIGHTNING, RAIL_LNURL} and currency != "BTC":
        bitcoin_asset = False
    if not bitcoin_asset:
        hint_key = "non_bitcoin" if rail != RAIL_PLUGIN else "plugin"
    elif rail == RAIL_ONCHAIN:
        hint_key = "liquid_onchain" if chain == "liquid" else "btc_onchain"
    elif rail == RAIL_LIGHTNING:
        hint_key = "lightning"
    elif rail == RAIL_LNURL:
        hint_key = "lnurl"
    else:
        hint_key = "plugin"
    wallet_history_supported = payment_method_id in WALLET_HISTORY_PAYMENT_METHOD_IDS
    return {
        "ledger_group": ledger_group(payment_method_id, rail=rail, bitcoin_asset=bitcoin_asset),
        "payment_method_id": payment_method_id,
        "currency": currency,
        "rail": rail,
        "chain": chain if bitcoin_asset else None,
        "bitcoin_asset": bitcoin_asset,
        "wallet_history_supported": wallet_history_supported,
        "provenance_supported": True,
        "settlement": hint_key,
        "settlement_hint": SETTLEMENT_HINTS[hint_key],
        "label": payment_method_label(payment_method_id, rail=rail, currency=currency),
    }


def ledger_group(payment_method_id: Any, *, rail: str | None = None, bitcoin_asset: bool | None = None) -> str | None:
    """Which BTCPay payment ledger a method's payments belong to.

    On-chain methods have real wallet history and never use a ledger. BOLT11
    and LNURL payments settle into the same Lightning node, so one store keeps
    one Lightning ledger; every plugin rail keeps its own.
    """

    if rail is None or bitcoin_asset is None:
        info = classify_payment_method(payment_method_id)
        rail, bitcoin_asset = info["rail"], info["bitcoin_asset"]
    if not bitcoin_asset or rail == RAIL_ONCHAIN:
        return None
    if rail in {RAIL_LIGHTNING, RAIL_LNURL}:
        return "lightning"
    return canonical_payment_method_id(payment_method_id)


def payment_method_label(payment_method_id: str, *, rail: str | None = None, currency: str | None = None) -> str:
    if rail is None or currency is None:
        currency, handler = split_payment_method_id(payment_method_id)
        rail = _HANDLER_RAILS.get(handler, RAIL_PLUGIN) if handler else RAIL_PLUGIN
    names = {
        ("BTC", RAIL_ONCHAIN): "Bitcoin on-chain",
        ("BTC", RAIL_LIGHTNING): "Lightning",
        ("BTC", RAIL_LNURL): "LNURL / Lightning Address",
        ("LBTC", RAIL_ONCHAIN): "Liquid Bitcoin",
    }
    if (currency, rail) in names:
        return names[(currency, rail)]
    if rail == RAIL_ONCHAIN:
        return f"{currency} on-chain"
    if rail == RAIL_PLUGIN:
        return f"{payment_method_id} (plugin)"
    return payment_method_id


def is_wallet_history_payment_method(payment_method_id: Any) -> bool:
    return canonical_payment_method_id(payment_method_id) in WALLET_HISTORY_PAYMENT_METHOD_IDS


def require_wallet_history_payment_method(payment_method_id: Any) -> str:
    value = str(payment_method_id or "")
    if is_wallet_history_payment_method(value):
        return canonical_payment_method_id(value)
    info = classify_payment_method(value) if value else None
    rail = info["rail"] if info else None
    if rail in {RAIL_LIGHTNING, RAIL_LNURL}:
        hint = (
            "Lightning payments cannot be imported as wallet history. Keep this "
            "method as invoice provenance and add the Lightning node or wallet "
            "that receives the payments as its own connection."
        )
    else:
        hint = (
            "Use an on-chain method such as BTC-CHAIN or LBTC-CHAIN, or keep "
            "this method as invoice provenance only."
        )
    raise AppError(
        f"BTCPay payment method '{payment_method_id}' is not available through wallet-history sync",
        code="validation",
        hint=hint,
    )


def asset_for_payment_method(payment_method_id: Any) -> str | None:
    currency, _ = split_payment_method_id(canonical_payment_method_id(payment_method_id))
    return currency or None


__all__ = [
    "BITCOIN_ASSET_CHAINS",
    "RAIL_LIGHTNING",
    "RAIL_LNURL",
    "RAIL_ONCHAIN",
    "RAIL_PLUGIN",
    "SETTLEMENT_HINTS",
    "WALLET_HISTORY_PAYMENT_METHOD_IDS",
    "asset_for_payment_method",
    "canonical_payment_method_id",
    "classify_payment_method",
    "is_wallet_history_payment_method",
    "ledger_group",
    "payment_method_label",
    "require_wallet_history_payment_method",
    "split_payment_method_id",
]
