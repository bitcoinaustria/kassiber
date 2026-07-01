from __future__ import annotations

import json
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any


class TapeMiss(AssertionError):
    """Raised when replay asks for an interaction absent from the tape."""


def _canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), default=str)


def rpc_key(method: str, params: Sequence[Any] | None = None, *, wallet_name: str | None = None) -> str:
    return _canonical_json(
        {
            "transport": "bitcoinrpc",
            "wallet_name": wallet_name or "",
            "method": method,
            "params": list(params or []),
        }
    )


@dataclass(frozen=True)
class RecordedTape:
    path: Path
    provenance: Mapping[str, Any]
    interactions: Mapping[str, Any]

    @classmethod
    def load(cls, path: str | Path) -> "RecordedTape":
        tape_path = Path(path)
        payload = json.loads(tape_path.read_text(encoding="utf-8"))
        provenance = payload.get("provenance") or {}
        interactions = payload.get("interactions") or {}
        if not isinstance(provenance, dict) or not isinstance(interactions, dict):
            raise ValueError(f"Invalid tape shape in {tape_path}")
        return cls(
            path=tape_path,
            provenance=provenance,
            interactions=interactions,
        )

    def lookup(self, key: str) -> Any:
        if key not in self.interactions:
            known = "\n".join(sorted(self.interactions)[:20])
            raise TapeMiss(
                f"Recorded tape {self.path} is missing interaction {key}."
                f"\nKnown interactions:\n{known}"
            )
        value = self.interactions[key]
        # Return a deep copy so adapter code cannot mutate the in-memory tape.
        return json.loads(json.dumps(value))


class BitcoinRpcTape:
    def __init__(self, tape: RecordedTape):
        self.tape = tape
        self.calls: list[str] = []
        self.backends: list[dict[str, Any]] = []
        self.timeouts: list[int | None] = []

    def call(
        self,
        backend: Mapping[str, Any],
        method: str,
        params: Sequence[Any] | None = None,
        wallet_name: str | None = None,
        timeout: int | None = None,
    ) -> Any:
        if backend.get("source") != "database":
            raise AssertionError("Bitcoin RPC tape must be driven by a DB-backed backend row")
        if backend.get("timeout") != 30:
            raise AssertionError("Bitcoin RPC tape expected backend timeout 30")
        self.backends.append(dict(backend))
        self.timeouts.append(timeout)
        key = rpc_key(method, params, wallet_name=wallet_name)
        self.calls.append(key)
        return self.tape.lookup(key)

    def unused_interactions(self) -> list[str]:
        used = set(self.calls)
        return sorted(key for key in self.tape.interactions if key not in used)


__all__ = [
    "BitcoinRpcTape",
    "RecordedTape",
    "TapeMiss",
    "rpc_key",
]
