"""Raw component repository used by the SQL adapter and the pure custody oracle.

Selectors name domain reads, not SQL text. The snapshot implementation has no
connection or query interpreter. SQL callers retain their original loaders.
"""
from __future__ import annotations

from typing import Any, Callable, Mapping


def component_fact(repository, name: str, parameters, sql_loader: Callable[[], Any]):
    if isinstance(repository, ComponentFacts):
        return repository.read(name, parameters)
    return sql_loader()


def coalesced_anchor(leg):
    anchor = leg.get("anchor_transaction_id")
    return leg.get("transaction_id") if anchor is None else anchor


class ComponentFacts:
    def __init__(self, snapshot: Mapping[str, Any]):
        self.snapshot = snapshot
        self.components = snapshot["components"]
        self.headers = {item["header"]["id"]: item["header"] for item in self.components}
        self.headers.update({row["id"]: row for row in snapshot["revision_headers"]})
        self.anchors = snapshot["anchors"]
        self.wallets = {row["id"]: row for row in snapshot["anchor_wallets"]}

    def execute(self, *_args, **_kwargs):
        raise AssertionError("pure custody attempted a database query")

    @staticmethod
    def transaction_order(row):
        return row["occurred_at"], row["created_at"], row["id"]

    @staticmethod
    def without_provenance(row):
        # These two historical reads intentionally do not join the closed
        # provenance table. Granting authority here would change commitments.
        return {k: v for k, v in row.items() if not k.startswith("observation_")}

    def read(self, name, parameters):
        p = tuple(parameters)
        if name == "header":
            return self.headers.get(p[0])
        if name == "wallet":
            row = self.wallets.get(p[0])
            return None if row is None else {"id": row["id"], "wallet_kind": row["kind"], "config_json": row["config_json"]}
        if name == "anchor":
            return self.anchors.get(p[0])
        if name in {"legs", "allocations", "economic_terms", "commitments"}:
            component = next((c for c in self.components if c["header"]["id"] == p[0]), None)
            rows = [] if component is None else component[name]
            return [row for row in rows if row["profile_id"] == p[1]] if name == "commitments" else rows
        if name == "active_headers":
            rows = [c["header"] for c in self.components if c["header"]["profile_id"] == p[0] and c["header"]["state"] == "active"]
            if len(p) > 1:
                matching = {c["header"]["id"] for c in self.components for leg in c["legs"] if coalesced_anchor(leg) == p[1]}
                rows = [row for row in rows if row["id"] in matching]
            return sorted(rows, key=lambda row: (row["created_at"], row["revision"], row["id"]))
        if name == "competing_headers":
            return sorted([h for h in self.headers.values() if h["profile_id"] == p[0] and h["lineage_id"] == p[1] and h["state"] == "active" and h["id"] != p[2]], key=lambda h: (h["revision"], h["id"]))
        if name in {"quantity_legs", "quantity_allocations"}:
            child = name.removeprefix("quantity_")
            return sorted([row for c in self.components if c["header"]["profile_id"] == p[0] and c["header"]["state"] == "active" and c["header"]["conservation_mode"] == "quantity" and (len(p) == 1 or c["header"]["id"] != p[1]) for row in c[child]], key=lambda row: (row["component_id"], row["ordinal"], row["id"]))
        if name == "membership_conflicts":
            mine = self.read("legs", (p[0],))
            ids = {coalesced_anchor(leg) for leg in mine if leg["profile_id"] == p[1]}
            pairs = {(coalesced_anchor(leg), c["header"]["id"]) for c in self.components if c["header"]["state"] == "active" and c["header"]["id"] != p[0] for leg in c["legs"] if leg["profile_id"] == p[1] and coalesced_anchor(leg) in ids and coalesced_anchor(leg) is not None}
            return [{"transaction_id": tx, "component_id": component} for tx, component in sorted(pairs)]
        if name in {"canonical_anchors", "native_rows"}:
            source = self.anchors.values() if name == "canonical_anchors" else self.snapshot["transactions"]
            rows = [self.without_provenance(row) for row in source if row is not None and row["profile_id"] == p[0] and (row["id"] in p[1:] if name == "canonical_anchors" else not row["excluded"])]
            return sorted(rows, key=self.transaction_order)
        raise AssertionError(f"unknown custody component fact: {name}")
