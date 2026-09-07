"""Read the durable local graph through one short, consistent SQL snapshot.

The SQL adapter implements the same graph mappings as the pure rebuild oracle.
Visibility is selected at lookup/adjacency time, before traversal or filtering.
Views never escape ``read_index``; a caller cannot accidentally retain a mutable
SQLite-backed object and mistake it for a frozen historical investigation.
"""
from __future__ import annotations

from collections.abc import Mapping
from contextlib import contextmanager
import json

from .index import _freeze, thaw
from .projection_store import synchronize


class _Rows(Mapping):
    def __init__(self, view, table, column):
        self.view, self.table, self.column = view, table, column
        self.cache = {}

    def __getitem__(self, key):
        self.view.check()
        if key not in self.cache:
            row = self.view.conn.execute(f"SELECT {self.column} FROM {self.table} WHERE profile_id=? AND id=?", (self.view.profile_id, key)).fetchone()
            self.cache[key] = _freeze(json.loads(row[0])) if row and row[0] is not None else None
        if self.cache[key] is None:
            raise KeyError(key)
        return self.cache[key]

    def __iter__(self):
        self.view.check()
        return (row[0] for row in self.view.conn.execute(f"SELECT id FROM {self.table} WHERE profile_id=? AND {self.column} IS NOT NULL ORDER BY id", (self.view.profile_id,)))

    def __len__(self):
        self.view.check()
        return self.view.conn.execute(f"SELECT count(*) FROM {self.table} WHERE profile_id=? AND {self.column} IS NOT NULL", (self.view.profile_id,)).fetchone()[0]


class _Adjacency(Mapping):
    def __init__(self, view, direction):
        self.view, self.direction = view, direction

    def __getitem__(self, key):
        self.view.check()
        values = tuple(row[0] for row in self.view.conn.execute(f"SELECT id FROM chain_index_edges WHERE profile_id=? AND {self.direction}=? AND {self.view.edge_column} IS NOT NULL ORDER BY id", (self.view.profile_id, key)))
        if not values:
            raise KeyError(key)
        return values

    def __iter__(self):
        self.view.check()
        return (row[0] for row in self.view.conn.execute(f"SELECT DISTINCT {self.direction} FROM chain_index_edges WHERE profile_id=? AND {self.view.edge_column} IS NOT NULL ORDER BY {self.direction}", (self.view.profile_id,)))

    def __len__(self):
        return sum(1 for _ in self)


class _Subjects(Mapping):
    def __init__(self, view):
        self.view = view

    def __getitem__(self, key):
        self.view.check()
        values = tuple(row[0] for row in self.view.conn.execute("SELECT node_id FROM chain_index_aliases WHERE profile_id=? AND observer=? AND alias=? ORDER BY node_id", (self.view.profile_id, self.view.visibility, key)))
        if not values:
            raise KeyError(key)
        return values

    def __iter__(self):
        self.view.check()
        return (row[0] for row in self.view.conn.execute("SELECT DISTINCT alias FROM chain_index_aliases WHERE profile_id=? AND observer=? ORDER BY alias", (self.view.profile_id, self.view.visibility)))

    def __len__(self):
        return sum(1 for _ in self)


class _View:
    def __init__(self, conn, profile_id, state, observer="owner", lifetime=None):
        self.conn, self.profile_id, self.state = conn, profile_id, state
        self.visibility = "public" if observer == "public" else "owner"
        self.lifetime = lifetime if lifetime is not None else [True]
        self.snapshot_id = state["snapshot_id"]
        self.edge_column = "public_json" if self.visibility == "public" else "payload_json"
        public = self.visibility == "public"
        self.nodes = _Rows(self, "chain_index_nodes", self.edge_column)
        self.edges = _Rows(self, "chain_index_edges", self.edge_column)
        self.transaction_facts = _Rows(self, "chain_index_nodes", "public_tx_json" if public else "tx_json")
        self.output_facts = _Rows(self, "chain_index_nodes", "public_output_json" if public else "output_json")
        self.outgoing, self.incoming = _Adjacency(self, "source"), _Adjacency(self, "target")
        self.subjects = _Subjects(self)
        coverage = json.loads(state["coverage_json"])
        if public:
            datasets = coverage.get("datasets")
            if isinstance(datasets, dict):
                partitions = datasets.pop("visibility_coverage", {})
                if "public" in partitions:
                    datasets.update(partitions["public"])
            coverage.update(observer_knowledge="public_chain_facts", hidden_private_node_count=conn.execute("SELECT count(*) FROM chain_index_nodes WHERE profile_id=? AND public_json IS NULL", (profile_id,)).fetchone()[0],
                hidden_private_relation_count=conn.execute("SELECT count(*) FROM chain_index_edges WHERE profile_id=? AND public_json IS NULL", (profile_id,)).fetchone()[0],
                public_liquid_value_policy="explicit_public_values_only", private_labels_withheld=True)
        self.coverage = _freeze(coverage)

    def check(self):
        if not self.lifetime[0]:
            raise RuntimeError("Indexed graph view used outside its read snapshot")

    def for_observer(self, observer):
        self.check()
        if (observer == "public") == (self.visibility == "public"):
            return self
        # A public view never upgrades itself to private knowledge.
        if self.visibility == "public":
            return self
        return _View(self.conn, self.profile_id, self.state, observer, self.lifetime)

    @property
    def profile_seeds(self):
        return self.seed_nodes({})

    def seed_nodes(self, query):
        self.check()
        clause, values = "", [self.profile_id]
        for field in ("chain", "network"):
            if field in query:
                from ...util import normalize_network
                value = normalize_network(query.get("chain") or "bitcoin", query[field]) if field == "network" else query[field]
                clause += f" AND json_extract(payload_json,'$.{field}')=?"
                values.append(value)
        return tuple(row[0] for row in self.conn.execute(f"SELECT id FROM chain_index_nodes WHERE profile_id=? AND seed=1 AND {self.edge_column} IS NOT NULL{clause} ORDER BY id", values))

    def _related(self, table, link, link_id, nodes, *, include_global=False):
        self.check()
        column = self.edge_column if table == "chain_index_findings" else "payload_json"
        found = {}
        ids = sorted(nodes)
        for offset in range(0, len(ids), 200):
            part = ids[offset:offset + 200]
            rows = self.conn.execute(f"SELECT DISTINCT f.id,f.{column} FROM {table} f JOIN {link} n ON n.profile_id=f.profile_id AND n.{link_id}=f.id WHERE f.profile_id=? AND f.{column} IS NOT NULL AND n.node_id IN ({','.join('?' for _ in part)})", (self.profile_id, *part))
            found.update((ident, json.loads(payload)) for ident, payload in rows)
        if include_global:
            for ident, payload in self.conn.execute(f"SELECT id,{column} FROM {table} WHERE profile_id=? AND {column} IS NOT NULL AND json_array_length(json_extract(payload_json,'$.node_ids'))=0", (self.profile_id,)):
                found[ident] = json.loads(payload)
        return tuple(_freeze(found[ident]) for ident in sorted(found))

    def findings_for_nodes(self, nodes):
        return self._related("chain_index_findings", "chain_index_finding_nodes", "finding_id", nodes, include_global=True)

    def labels_for_nodes(self, nodes):
        return self._visible_labels(self._related("chain_index_labels", "chain_index_label_nodes", "label_id", nodes))

    def owned_output_ids(self):
        self.check()
        if self.visibility == "public":
            return set()
        return {row[0] for row in self.conn.execute("SELECT id FROM chain_index_nodes WHERE profile_id=? AND json_extract(output_json,'$.ownership_known')=1 AND coalesce(json_extract(output_json,'$.ownership_ambiguous'),0)=0 AND json_array_length(json_extract(payload_json,'$.wallet_ids'))=1 AND json_extract(payload_json,'$.status') NOT IN ('missing','stale','conflicting')", (self.profile_id,))}

    def available_transaction_ids(self):
        self.check()
        return {row[0] for row in self.conn.execute(f"SELECT id FROM chain_index_nodes WHERE profile_id=? AND {self.edge_column} IS NOT NULL AND json_extract(payload_json,'$.kind')='transaction' AND json_extract(payload_json,'$.status')!='missing'", (self.profile_id,))}

    def _visible_labels(self, labels):
        return tuple(row for row in labels if self.visibility != "public" or row.get("dataset_id") and row.get("visibility") == "public")

    @property
    def labels(self):
        self.check()
        return self._visible_labels(tuple(_freeze(json.loads(row[0])) for row in self.conn.execute("SELECT payload_json FROM chain_index_labels WHERE profile_id=? ORDER BY id", (self.profile_id,))))

    @property
    def findings(self):
        self.check()
        return tuple(_freeze(json.loads(row[0])) for row in self.conn.execute(f"SELECT {self.edge_column} FROM chain_index_findings WHERE profile_id=? AND {self.edge_column} IS NOT NULL ORDER BY id", (self.profile_id,)))


@contextmanager
def read_index(conn, profile_id, *, observer="owner", rebuild=False):
    """Consistent, lazy, observer-scoped graph; never commit caller work."""
    from ..chain_analysis_cases import atomic
    with atomic(conn):
        state = synchronize(conn, profile_id, rebuild=rebuild)
        view = _View(conn, profile_id, state, observer)
        try:
            yield view
        finally:
            view.lifetime[0] = False


def index_revision(conn, profile_id):
    """Local watch cursor. All observations/overlays share one committed revision.

    A private change can advance a public watch's cursor; notifications must
    compare actual semantic findings. ``next_expiry`` schedules local validity
    transitions without requiring a network poll or an unrelated book write.
    """
    with read_index(conn, profile_id) as view:
        return {"schema_version": 1, "revision": view.state["revision"], "snapshot_id": view.snapshot_id, "next_expiry": view.state["next_expiry"]}


def rebuild_index(conn, profile_id):
    """Atomically replace derived state from source observations after recovery."""
    with read_index(conn, profile_id, rebuild=True) as view:
        return {"revision": view.state["revision"], "snapshot_id": view.snapshot_id, "coverage": thaw(view.coverage)}
