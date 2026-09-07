"""Shared factual transaction classification; no custody or tax computation."""

from typing import Any, Mapping


# Explicit inbound economics recognized by the RP2 adapter. A generic deposit
# remains ambiguous: its default BUY treatment does not prove an external origin.
INBOUND_KIND_TO_RP2_TYPE = {
    "buy": "BUY",
    "airdrop": "AIRDROP",
    "hardfork": "HARDFORK",
    "hard_fork": "HARDFORK",
    "income": "INCOME",
    "interest": "INTEREST",
    "lending_interest": "INTEREST",
    "mining": "MINING",
    "mining_reward": "MINING",
    "routing_income": "INCOME",
    "staking": "STAKING",
    # Compensation stays factual provenance; its reviewed value is acquisition
    # basis. Employment-income reporting remains outside Kassiber.
    "wages": "BUY",
}


def normalized_transaction_kind(row: Mapping[str, Any]) -> str:
    """Respect an explicit classification while retaining raw import provenance."""

    keys = row.keys() if hasattr(row, "keys") else ()
    kind = row["kind_override"] if "kind_override" in keys else None
    if kind in (None, ""):
        kind = row["kind"] if "kind" in keys else None
    if kind is None:
        return ""
    return str(kind).strip().lower().replace("-", "_").replace(" ", "_")
