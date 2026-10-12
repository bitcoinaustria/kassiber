"""How exports present one stored asset.

Kassiber has no asset registry yet; plan 19 ("Asset model") adds one with
namespaced ids, per-asset decimals and exact base units. Until then this module
is the single place an export learns an asset's identity, display code, chain
and units, so the registry replaces this module instead of every export.

Today every asset quantity is stored as an integer at msat scale (10**11 per
whole unit) through ``msat.btc_to_msat``, including Liquid issued assets.
Exports group by the stored asset value (``key``), which is unique within a
book; display codes are labels only and may collide across chains.
"""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Decimal
from typing import Any

# Storage scale shared by every asset until the registry supplies base units.
STORED_DECIMALS = 11
_MSAT_PER_SAT = 1000


@dataclass(frozen=True)
class AssetPresentation:
    key: str
    asset_id: str
    display_code: str
    chain: str
    display_decimals: int
    stored_decimals: int
    bitcoin_denominated: bool

    @property
    def stored_units_per_whole(self) -> int:
        return 10**self.stored_decimals

    def quantity(self, stored: Any) -> Decimal:
        """Whole-unit quantity of a stored integer amount, exactly."""
        return Decimal(int(stored or 0)).scaleb(-self.stored_decimals)

    def exact(self, stored: Any) -> str:
        """Lossless decimal text: at least the display decimals, more only when needed."""
        text = f"{self.quantity(stored):.{self.stored_decimals}f}"
        whole, _, fraction = text.partition(".")
        fraction = fraction.rstrip("0").ljust(self.display_decimals, "0")
        return f"{whole}.{fraction}" if fraction else whole

    def sats(self, stored: Any) -> Decimal | None:
        """Satoshis for bitcoin-denominated assets; None for every other asset."""
        if not self.bitcoin_denominated:
            return None
        return Decimal(int(stored or 0)) / _MSAT_PER_SAT


def _is_liquid_asset_id(value: str) -> bool:
    return len(value) == 64 and all(char in "0123456789abcdef" for char in value.lower())


def describe_asset(asset: Any) -> AssetPresentation:
    key = str(asset or "")
    code = key.strip()
    upper = code.upper()
    if upper == "BTC":
        return AssetPresentation(key, "bitcoin:btc", "BTC", "bitcoin", 8, STORED_DECIMALS, True)
    if upper in {"LBTC", "L-BTC"}:
        return AssetPresentation(key, "liquid:lbtc", "L-BTC", "liquid", 8, STORED_DECIMALS, True)
    if _is_liquid_asset_id(code):
        # An issued Liquid asset is identified by its asset id; a ticker
        # would only be a claim, so the short id is the display code.
        return AssetPresentation(
            key, f"liquid:{code.lower()}", f"{code.lower()[:8]}…", "liquid", 8, STORED_DECIMALS, False
        )
    # A namespaced value (`ext:<id>:<code>`) already is its own id; keep it
    # verbatim. A bare ticker is case-insensitive, so `usdt` and `USDT` group.
    asset_id = code if ":" in code else upper
    return AssetPresentation(key, asset_id, asset_id.rsplit(":", 1)[-1].upper(), "", 8, STORED_DECIMALS, False)


__all__ = ["AssetPresentation", "STORED_DECIMALS", "describe_asset"]
