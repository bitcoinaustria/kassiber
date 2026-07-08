"""Machine markers embedded in journal entry descriptions.

The engine annotates journal descriptions with ``key=value`` tokens: the
regime the disposed fee slice draws from, the per-regime moved quantities of
a transfer, the pool id, the swap link. Emission and parsing live HERE so
producers (the rp2 engine) and consumers (UI snapshots, exit tax) cannot
drift when a marker is added or renamed. The GUI strips any ``at_*=`` token
generically before rendering.

The ``at_`` prefix is historical (Austrian markers came first); the channel
itself is country-agnostic — a future country module adds its markers here.
"""

from __future__ import annotations

MARKER_REGIME = "at_regime"
#: Why a disposal carries its at_regime. ``wahlrecht`` = the disposing wallet
#: held BOTH Alt and Neu inventory and Kassiber exercised the
#: KryptowährungsVO designation right (Neu-first) on the taxpayer's behalf;
#: absent = the regime was forced by the wallet's holdings or set by an
#: explicit user override. Kassiber-side audit marker only — rp2 matches its
#: own markers by exact token / distinct prefix and ignores this one.
MARKER_REGIME_BASIS = "at_regime_basis"
REGIME_BASIS_ELECTION = "wahlrecht"
MARKER_POOL = "at_pool"
MARKER_SWAP_LINK = "at_swap_link"
#: Tax-free (alt) share of a transfer's quantities, in msat: what left the
#: source (carried + fee slice) / what the destination received. at_regime
#: above only describes the FEE slice of a mixed-regime MOVE.
MARKER_ALT_OUT = "at_alt_out"
MARKER_ALT_IN = "at_alt_in"


def marker_token(key: str, value: object) -> str:
    return f"{key}={value}"


def parse_marker(description: object, key: str) -> str | None:
    """Return ``key``'s value from a marker-bearing description, or None."""
    prefix = f"{key}="
    for token in str(description or "").split():
        if token.startswith(prefix):
            return token[len(prefix):]
    return None


def parse_marker_int(description: object, key: str) -> int | None:
    raw = parse_marker(description, key)
    if raw is None:
        return None
    try:
        return int(raw)
    except ValueError:
        return None
