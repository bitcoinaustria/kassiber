"""Rebuildable, book-local graph projection and transactional source change feed.

Only derived tables are written here. Source contributions are replaceable; a
withdrawn observation cannot leave behind ownership, edges or conflict flags.
The ordinary builder remains the single normalization/interpretation engine.
"""
from __future__ import annotations

import json
import uuid
from collections import defaultdict

from ...time_utils import now_iso
from ..onchain import stored_tx_mapping
from .index import _Builder, canonical_txid, digest, observer_index, thaw


VERSION = 1
PHYSICAL = ("transactions", "transaction_graph_cache", "wallet_utxos", "chain_analysis_observations", "chain_analysis_reference_assertions")
OVERLAYS = ("wallets", "profiles", "journal_custody_decisions", "journal_custody_economic_relations", "chain_analysis_labels", "chain_analysis_datasets", "chain_analysis_dataset_claims", "book_network_bindings")
SOURCE_TABLES = PHYSICAL + OVERLAYS
DDL = (
    "CREATE TABLE IF NOT EXISTS chain_index_clock(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL)",
    "INSERT OR IGNORE INTO chain_index_clock VALUES(1,0)",
    "CREATE TABLE IF NOT EXISTS chain_index_dirty(source_table TEXT NOT NULL, source_key INTEGER NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(source_table,source_key))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_dirty_revision ON chain_index_dirty(revision)",
    "CREATE TABLE IF NOT EXISTS chain_index_state(profile_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL, version INTEGER NOT NULL, watermark INTEGER NOT NULL, revision INTEGER NOT NULL, snapshot_id TEXT NOT NULL, coverage_json TEXT NOT NULL, next_expiry TEXT)",
    "CREATE TABLE IF NOT EXISTS chain_index_sources(profile_id TEXT NOT NULL, source_table TEXT NOT NULL, source_key INTEGER NOT NULL, payload_json TEXT NOT NULL, invalid_count INTEGER NOT NULL, rejected_count INTEGER NOT NULL, PRIMARY KEY(profile_id,source_table,source_key))",
    "CREATE TABLE IF NOT EXISTS chain_index_source_nodes(profile_id TEXT NOT NULL, source_table TEXT NOT NULL, source_key INTEGER NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,source_table,source_key,node_id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_contributors ON chain_index_source_nodes(profile_id,node_id)",
    "CREATE TABLE IF NOT EXISTS chain_index_nodes(profile_id TEXT NOT NULL, id TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, tx_json TEXT, public_tx_json TEXT, output_json TEXT, public_output_json TEXT, seed INTEGER NOT NULL, status TEXT NOT NULL, kind TEXT NOT NULL, complete INTEGER NOT NULL, PRIMARY KEY(profile_id,id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_node_counts ON chain_index_nodes(profile_id,status,complete)",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_private_nodes ON chain_index_nodes(profile_id) WHERE public_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_source_counts ON chain_index_sources(profile_id,invalid_count,rejected_count)",
    "CREATE TABLE IF NOT EXISTS chain_index_edges(profile_id TEXT NOT NULL, id TEXT NOT NULL, source TEXT NOT NULL, target TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, PRIMARY KEY(profile_id,id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_outgoing ON chain_index_edges(profile_id,source,id)",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_incoming ON chain_index_edges(profile_id,target,id)",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_private_edges ON chain_index_edges(profile_id) WHERE public_json IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_custody ON chain_index_edges(profile_id) WHERE json_extract(payload_json,'$.kind')='custody'",
    "CREATE TABLE IF NOT EXISTS chain_index_aliases(profile_id TEXT NOT NULL, observer TEXT NOT NULL, alias TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,observer,alias,node_id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_alias_node ON chain_index_aliases(profile_id,node_id)",
    "CREATE TABLE IF NOT EXISTS chain_index_findings(profile_id TEXT NOT NULL, id TEXT NOT NULL, payload_json TEXT NOT NULL, public_json TEXT, PRIMARY KEY(profile_id,id))",
    "CREATE TABLE IF NOT EXISTS chain_index_finding_nodes(profile_id TEXT NOT NULL, finding_id TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,finding_id,node_id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_finding_node ON chain_index_finding_nodes(profile_id,node_id)",
    "CREATE TABLE IF NOT EXISTS chain_index_labels(profile_id TEXT NOT NULL, id TEXT NOT NULL, payload_json TEXT NOT NULL, PRIMARY KEY(profile_id,id))",
    "CREATE TABLE IF NOT EXISTS chain_index_label_nodes(profile_id TEXT NOT NULL, label_id TEXT NOT NULL, node_id TEXT NOT NULL, PRIMARY KEY(profile_id,label_id,node_id))",
    "CREATE INDEX IF NOT EXISTS idx_chain_index_label_node ON chain_index_label_nodes(profile_id,node_id)",
)


def encoded(value):
    return json.dumps(thaw(value), sort_keys=True, separators=(",", ":"), allow_nan=False)


def install(conn):
    """Install the closed source allowlist, including optional later migrations.

    No executescript: installing a derived projection must not commit a caller's
    import/review transaction. Triggers capture ordinary SQL and replication too.
    """
    objects = {row[0] for row in conn.execute("SELECT name FROM sqlite_master WHERE type IN ('table','trigger')")}
    if "chain_index_clock" not in objects:
        for statement in DDL:
            statement = statement.replace("profile_id TEXT NOT NULL", "profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE")
            statement = statement.replace("profile_id TEXT PRIMARY KEY", "profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE")
            conn.execute(statement)
    if "chain_analysis_dataset_claims" in objects:
        for field in ("valid_from", "valid_until"):
            conn.execute(f"CREATE INDEX IF NOT EXISTS idx_chain_index_claim_{field} ON chain_analysis_dataset_claims({field},dataset_id) WHERE {field} IS NOT NULL")
    for table in SOURCE_TABLES:
        if table not in objects:
            continue
        added = False
        for operation, prefix in (("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")):
            name = f"chain_index_track_{table}_{operation.lower()}"
            if name in objects:
                continue
            # Source tables have SQLite rowids, including composite primary-key
            # reference assertions. A changed rowid withdraws the old source too.
            keys = (("OLD.rowid", "NEW.rowid") if operation == "UPDATE" else (f"{prefix}.rowid",)) if table in PHYSICAL else ("-1",)
            writes = "".join(
                f"INSERT INTO chain_index_dirty VALUES('{table}',{key},(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;"
                for key in keys
            )
            conn.execute(f"CREATE TRIGGER {name} AFTER {operation} ON {table} BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; {writes} END")
            added = True
        if added:
            conn.execute("UPDATE chain_index_clock SET revision=revision+1 WHERE id=1")
            conn.execute("INSERT OR REPLACE INTO chain_index_dirty SELECT ?,-1,revision FROM chain_index_clock WHERE id=1", (table,))
    return objects & set(SOURCE_TABLES)


def _rows(conn, table, profile_id, keys=None):
    if keys is not None:
        for offset in range(0, len(keys), 200):
            part = keys[offset:offset + 200]
            yield from (dict(row) for row in conn.execute(f"SELECT rowid AS _source_key,* FROM {table} WHERE rowid IN ({','.join('?' for _ in part)})", part))
    else:
        scoped = table != "transaction_graph_cache"
        yield from (dict(row) for row in conn.execute(f"SELECT rowid AS _source_key,* FROM {table}" + (" WHERE profile_id=?" if scoped else ""), (profile_id,) if scoped else ()))


def _apply(builder, table, row):
    if table == "transactions":
        builder.transaction(row)
    elif table == "wallet_utxos":
        builder.inventory(row)
    else:
        builder.reference(row, acquired=table != "transaction_graph_cache")


def _contribute(conn, profile_id, table, key, row):
    old = {value[0] for value in conn.execute("SELECT node_id FROM chain_index_source_nodes WHERE profile_id=? AND source_table=? AND source_key=?", (profile_id, table, key))}
    conn.execute("DELETE FROM chain_index_source_nodes WHERE profile_id=? AND source_table=? AND source_key=?", (profile_id, table, key))
    conn.execute("DELETE FROM chain_index_sources WHERE profile_id=? AND source_table=? AND source_key=?", (profile_id, table, key))
    if row is None or table != "transaction_graph_cache" and row.get("profile_id") != profile_id or row.get("active") == 0:
        return old
    if table == "transactions":
        wallet = conn.execute("SELECT * FROM wallets WHERE id=?", (row.get("wallet_id"),)).fetchone()
        wallet = dict(wallet) if wallet else {}
        row.update(wallet_kind=wallet.get("kind"), wallet_config_json=wallet.get("config_json"))
    builder = _Builder(profile_id)
    _apply(builder, table, row)
    conn.execute("INSERT INTO chain_index_sources VALUES(?,?,?,?,?,?)", (profile_id, table, key, encoded(row), builder.coverage["invalid_observations"], builder.coverage["cache_rejected"]))
    conn.executemany("INSERT INTO chain_index_source_nodes VALUES(?,?,?,?)", ((profile_id, table, key, node) for node in builder.nodes))
    return old | set(builder.nodes)


def _contributors(conn, profile_id, nodes=None):
    if nodes is None:
        return {(row[0], row[1]): json.loads(row[2]) for row in conn.execute("SELECT source_table,source_key,payload_json FROM chain_index_sources WHERE profile_id=?", (profile_id,))}
    result = {}
    nodes = sorted(nodes)
    for offset in range(0, len(nodes), 200):
        part = nodes[offset:offset + 200]
        rows = conn.execute(f"SELECT s.source_table,s.source_key,s.payload_json FROM chain_index_source_nodes n JOIN chain_index_sources s USING(profile_id,source_table,source_key) WHERE n.profile_id=? AND n.node_id IN ({','.join('?' for _ in part)})", (profile_id, *part))
        for table, key, payload in rows:
            result[(table, key)] = json.loads(payload)
    return result


def _relations(conn, builder, tables):
    profile_row = conn.execute("SELECT * FROM profiles WHERE id=?", (builder.profile_id,)).fetchone() if "profiles" in tables else None
    profile = dict(profile_row) if profile_row else {}
    count = conn.execute("SELECT count(*) FROM transactions WHERE profile_id=? AND excluded=0", (builder.profile_id,)).fetchone()[0] if "transactions" in tables else 0
    fresh = bool(profile.get("last_processed_at")) and profile.get("journal_input_version") == profile.get("last_processed_input_version") and profile.get("last_processed_tx_count") == count
    # Record aliases are indexed; relation endpoints do not require reading
    # every transaction or rebuilding their physical facts.
    rows = []
    for table in ("journal_custody_decisions", "journal_custody_economic_relations"):
        rows.append([dict(row) for row in conn.execute(f"SELECT * FROM {table} WHERE profile_id=?", (builder.profile_id,))] if table in tables else [])
    anchors = {row.get(key) for group in rows for row in group for key in ("source_transaction_id", "target_transaction_id")}
    for record in anchors - builder.records.keys():
        if record is None or not conn.execute("SELECT 1 FROM transactions WHERE profile_id=? AND id=?", (builder.profile_id, record)).fetchone():
            continue
        match = conn.execute("SELECT node_id FROM chain_index_aliases WHERE profile_id=? AND observer='owner' AND alias=?", (builder.profile_id, f"record:{record}")).fetchone()
        if match:
            builder.records[record] = match[0]
    builder.relations(*rows, fresh)
    if not fresh:
        builder.finding("custody_projection_stale", [], "Canonical custody relations require a current journal rebuild before they can be traversed.", severity="info")


def _assemble(conn, profile_id, core, tables, full=False):
    if not full:
        # Changing an output changes value/completeness facts of its creator and
        # all spenders. This is one dependency step, not an entire component walk.
        for node in tuple(core):
            if ":out:" in node:
                core.update(row[0] for row in conn.execute("SELECT target FROM chain_index_edges WHERE profile_id=? AND source=?", (profile_id, node)))
    sources = _contributors(conn, profile_id, None if full else core)
    builder = _Builder(profile_id)
    for (table, key), row in sorted(sources.items(), key=lambda value: (PHYSICAL.index(value[0][0]), value[0][1])):
        _apply(builder, table, row)
    if full:
        core.update(builder.nodes)
    # All custody edges are a small, separate authored overlay. Freshness may
    # invalidate them globally; physical parsing remains incremental.
    for source, target in conn.execute("SELECT source,target FROM chain_index_edges WHERE profile_id=? AND json_extract(payload_json,'$.kind')='custody'", (profile_id,)):
        core.update((source, target))
    _relations(conn, builder, tables)
    for edge in builder.edges.values():
        if edge["kind"] == "custody":
            core.update((edge["source"], edge["target"]))
    # Custody endpoints added after source selection need their source facts.
    more = _contributors(conn, profile_id, core)
    if more.keys() - sources.keys():
        sources.update(more)
        builder = _Builder(profile_id)
        for (table, key), row in sorted(sources.items(), key=lambda value: (PHYSICAL.index(value[0][0]), value[0][1])):
            _apply(builder, table, row)
        _relations(conn, builder, tables)
    # Context outside the changed region must retain canonical values/status;
    # a partial prevout in one contributor cannot downgrade a known ancestor.
    for ident in set(builder.nodes) - core:
        row = conn.execute("SELECT payload_json,output_json FROM chain_index_nodes WHERE profile_id=? AND id=?", (profile_id, ident)).fetchone()
        if row:
            builder.nodes[ident] = json.loads(row[0])
            if row[1]:
                builder.output_facts[ident] = json.loads(row[1])
    # A relation can refer to a retained endpoint absent from these contributors.
    for edge in builder.edges.values():
        for ident in (edge["source"], edge["target"]):
            if ident not in builder.nodes:
                row = conn.execute("SELECT payload_json FROM chain_index_nodes WHERE profile_id=? AND id=?", (profile_id, ident)).fetchone()
                if row:
                    builder.nodes[ident] = json.loads(row[0])
    for ident, edge in tuple(builder.edges.items()):
        output = edge["target"] if edge["kind"] == "creates" else edge["source"] if edge["kind"] == "spends" else None
        if output is not None and output not in core:
            row = conn.execute("SELECT payload_json FROM chain_index_edges WHERE profile_id=? AND id=?", (profile_id, ident)).fetchone()
            if row:
                builder.edges[ident] = json.loads(row[0])
    return builder.finish(), core


def _save(conn, profile_id, index, core):
    public = observer_index(index, "public")
    core = set(core)
    for ident in core:
        conn.execute("DELETE FROM chain_index_nodes WHERE profile_id=? AND id=?", (profile_id, ident))
        conn.execute("DELETE FROM chain_index_aliases WHERE profile_id=? AND node_id=?", (profile_id, ident))
        conn.execute("DELETE FROM chain_index_edges WHERE profile_id=? AND (source=? OR target=?)", (profile_id, ident, ident))
        affected = [row[0] for row in conn.execute("SELECT finding_id FROM chain_index_finding_nodes WHERE profile_id=? AND node_id=?", (profile_id, ident))]
        for finding in affected:
            stored = conn.execute("SELECT payload_json FROM chain_index_findings WHERE profile_id=? AND id=?", (profile_id, finding)).fetchone()
            if stored and json.loads(stored[0])["node_ids"][0] not in core:
                continue
            conn.execute("DELETE FROM chain_index_findings WHERE profile_id=? AND id=?", (profile_id, finding))
            conn.execute("DELETE FROM chain_index_finding_nodes WHERE profile_id=? AND finding_id=?", (profile_id, finding))
        node = index.nodes.get(ident)
        if node is not None:
            values = (node, public.nodes.get(ident), index.transaction_facts.get(ident), public.transaction_facts.get(ident), index.output_facts.get(ident), public.output_facts.get(ident))
            conn.execute("INSERT INTO chain_index_nodes VALUES(?,?,?,?,?,?,?,?,?,?,?,?)", (profile_id, ident, *(encoded(value) if value is not None else None for value in values), int(ident in index.profile_seeds), node["status"], node["kind"], int(bool(index.transaction_facts.get(ident, {}).get("complete")))))
    for ident, edge in index.edges.items():
        if edge["source"] in core or edge["target"] in core:
            conn.execute("INSERT OR REPLACE INTO chain_index_edges VALUES(?,?,?,?,?,?)", (profile_id, ident, edge["source"], edge["target"], encoded(edge), encoded(public.edges[ident]) if ident in public.edges else None))
    for observer, view in (("owner", index), ("public", public)):
        conn.executemany("INSERT OR IGNORE INTO chain_index_aliases VALUES(?,?,?,?)", ((profile_id, observer, alias, ident) for alias, ids in view.subjects.items() for ident in ids if ident in core))
        conn.executemany("INSERT OR IGNORE INTO chain_index_aliases VALUES(?,?,?,?)", ((profile_id, observer, f"{'tx' if node['kind'] == 'transaction' else 'out'}:{node.get('txid') if node['kind'] == 'transaction' else node.get('outpoint')}", ident) for ident, node in view.nodes.items() if ident in core and node["kind"] in {"transaction", "output"}))
    # Global findings belong to the freshly evaluated custody overlay.
    conn.execute("DELETE FROM chain_index_findings WHERE profile_id=? AND json_array_length(json_extract(payload_json,'$.node_ids'))=0", (profile_id,))
    public_findings = {row["id"]: row for row in public.findings}
    for finding in index.findings:
        if not finding["node_ids"] or finding["node_ids"][0] in core:
            ident = finding["id"]
            conn.execute("INSERT OR REPLACE INTO chain_index_findings VALUES(?,?,?,?)", (profile_id, ident, encoded(finding), encoded(public_findings[ident]) if ident in public_findings else None))
            conn.executemany("INSERT OR IGNORE INTO chain_index_finding_nodes VALUES(?,?,?)", ((profile_id, ident, node) for node in finding["node_ids"]))


def _next_expiry(conn, profile_id, tables, instant):
    if "chain_analysis_datasets" not in tables:
        return None
    # SQL indexes on validity columns keep this independent of pack size.
    values = []
    for field in ("valid_from", "valid_until"):
        row = conn.execute(f"SELECT min(c.{field}) FROM chain_analysis_dataset_claims c JOIN chain_analysis_datasets d ON d.id=c.dataset_id WHERE d.profile_id=? AND d.status='active' AND c.{field}>?", (profile_id, instant)).fetchone()
        if row and row[0]:
            values.append(row[0])
    for row in conn.execute("SELECT manifest_json FROM chain_analysis_datasets WHERE profile_id=? AND status='active'", (profile_id,)):
        manifest = json.loads(row[0])
        values.extend(value for field in ("valid_from", "valid_until") if (value := manifest.get(field)) and value > instant)
    return min(values, default=None)


def _labels(conn, profile_id, tables, coverage):
    labels = []
    if "chain_analysis_labels" in tables:
        from ..custody_evidence import resolve_protocol_scope
        from ...errors import AppError
        for row in conn.execute("SELECT * FROM chain_analysis_labels WHERE profile_id=? AND deleted=0 ORDER BY id", (profile_id,)):
            row = dict(row)
            try:
                scope = resolve_protocol_scope(row)
            except (ValueError, AppError):
                coverage["invalid_observations"] += 1
                continue
            labels.append({**row, "chain": scope.protocol_chain, "network": scope.network})
    if "chain_analysis_datasets" in tables:
        from ..chain_analysis_datasets import match_subjects, MAX_SUBJECTS, MAX_SCRIPT_BYTES
        subjects = set()
        skipped = 0
        active = conn.execute("SELECT 1 FROM chain_analysis_datasets WHERE profile_id=? AND status='active' LIMIT 1", (profile_id,)).fetchone()
        # Empty packs need no whole-graph attribution work. Physical indexing
        # and exact-label lookup remain usable without imported datasets.
        if active:
            for payload, fact in conn.execute("SELECT payload_json,output_json FROM chain_index_nodes WHERE profile_id=?", (profile_id,)):
                node = json.loads(payload)
                if node.get("chain") not in {"bitcoin", "liquid"}:
                    continue
                if node["kind"] == "transaction":
                    subjects.add((node["chain"], node["network"], f"tx:{node['txid']}"))
                elif node["kind"] == "output":
                    subjects.add((node["chain"], node["network"], f"out:{node['outpoint']}"))
                    script = json.loads(fact).get("script") if fact else None
                    if script and len(script) <= MAX_SCRIPT_BYTES * 2:
                        subjects.add((node["chain"], node["network"], f"script:{script}"))
                    elif script:
                        skipped += 1
        matched = match_subjects(conn, profile_id, sorted(subjects)[:MAX_SUBJECTS])
        labels.extend(matched["claims"])
        coverage["datasets"] = {"state_digest": matched["dataset_state_digest"], **{key: value for key, value in matched["coverage"].items() if key != "as_of"}, "subject_limit": MAX_SUBJECTS, "subjects_truncated": len(subjects) > MAX_SUBJECTS, "scripts_skipped": skipped}
    conn.execute("DELETE FROM chain_index_labels WHERE profile_id=?", (profile_id,))
    conn.execute("DELETE FROM chain_index_label_nodes WHERE profile_id=?", (profile_id,))
    for label in labels:
        ids = []
        for ident, payload in conn.execute("SELECT n.id,n.payload_json FROM chain_index_aliases a JOIN chain_index_nodes n ON n.profile_id=a.profile_id AND n.id=a.node_id WHERE a.profile_id=? AND a.observer='owner' AND a.alias=? ORDER BY n.id", (profile_id, label.get("subject"))):
            node = json.loads(payload)
            if node["chain"] == label.get("chain") and node["network"] == label.get("network"):
                ids.append(ident)
        label["node_ids"] = ids
        conn.execute("INSERT OR REPLACE INTO chain_index_labels VALUES(?,?,?)", (profile_id, label["id"], encoded(label)))
        conn.executemany("INSERT OR IGNORE INTO chain_index_label_nodes VALUES(?,?,?)", ((profile_id, label["id"], ident) for ident in ids))


def synchronize(conn, profile_id, *, rebuild=False):
    """Publish one projection generation inside the caller's savepoint."""
    tables = install(conn)
    state_row = conn.execute("SELECT * FROM chain_index_state WHERE profile_id=?", (profile_id,)).fetchone()
    state = dict(state_row) if state_row else None
    watermark = conn.execute("SELECT revision FROM chain_index_clock WHERE id=1").fetchone()[0]
    instant = now_iso()
    full = rebuild or state is None or state["version"] != VERSION
    expired = bool(state and state["next_expiry"] and state["next_expiry"] <= instant)
    if not full and state["watermark"] == watermark and not expired:
        return state
    dirty = defaultdict(set)
    if not full:
        for table, key in conn.execute("SELECT source_table,source_key FROM chain_index_dirty WHERE revision>?", (state["watermark"],)):
            dirty[table].add(key)
    if dirty.keys() & {"wallets", "book_network_bindings"}:
        # Configuration changes can affect every observation in the wallet;
        # retain source identity and re-normalize rather than relabel old nodes.
        dirty["transactions"].update(row[0] for row in conn.execute("SELECT rowid FROM transactions WHERE profile_id=?", (profile_id,)))
    core = set()
    if full:
        for table in ("chain_index_sources", "chain_index_source_nodes", "chain_index_nodes", "chain_index_edges", "chain_index_aliases", "chain_index_findings", "chain_index_finding_nodes"):
            conn.execute(f"DELETE FROM {table} WHERE profile_id=?", (profile_id,))
    changed = full or expired or bool(dirty.keys() & set(OVERLAYS))
    for table in PHYSICAL:
        if table not in tables:
            continue
        keys = None if full or -1 in dirty.get(table, ()) else sorted(dirty.get(table, ()))
        if keys == []:
            continue
        rows = {row["_source_key"]: row for row in _rows(conn, table, profile_id, keys)}
        selected = set(rows) | {row[0] for row in conn.execute("SELECT source_key FROM chain_index_sources WHERE profile_id=? AND source_table=?", (profile_id, table))} if keys is None else keys
        for key in sorted(selected):
            affected = _contribute(conn, profile_id, table, key, rows.get(key))
            core.update(affected)
            changed |= bool(affected)
    if not changed:
        conn.execute("UPDATE chain_index_state SET watermark=? WHERE profile_id=?", (watermark, profile_id))
        state["watermark"] = watermark
        return state
    index, core = _assemble(conn, profile_id, core, tables, full=full)
    _save(conn, profile_id, index, core)
    coverage = thaw(index.coverage)
    coverage["source_rows"] = {table: conn.execute(f"SELECT count(*) FROM {table}" + (" WHERE profile_id=?" if table != "transaction_graph_cache" else ""), (profile_id,) if table != "transaction_graph_cache" else ()).fetchone()[0] for table in ("wallets", *PHYSICAL[:4], "chain_analysis_labels", "journal_custody_decisions", "journal_custody_economic_relations") if table in tables}
    coverage["missing_tables"] = [table for table in ("wallets", "transactions", "transaction_graph_cache", "wallet_utxos", "journal_custody_decisions", "journal_custody_economic_relations") if table not in tables]
    totals = conn.execute("SELECT coalesce(sum(invalid_count),0),coalesce(sum(rejected_count),0) FROM chain_index_sources WHERE profile_id=?", (profile_id,)).fetchone()
    coverage["invalid_observations"], coverage["cache_rejected"] = totals
    counts = conn.execute("SELECT count(*),sum(status='missing'),sum(status='conflicting'),sum(status='reference'),sum(complete) FROM chain_index_nodes WHERE profile_id=?", (profile_id,)).fetchone()
    for key, value in zip(("node_count", "missing_node_count", "conflicting_node_count", "reference_node_count", "complete_transaction_count"), counts):
        coverage[key] = value or 0
    coverage["edge_count"] = conn.execute("SELECT count(*) FROM chain_index_edges WHERE profile_id=?", (profile_id,)).fetchone()[0]
    _labels(conn, profile_id, tables, coverage)
    revision = (state["revision"] if state else 0) + int(not rebuild or state is None or state["watermark"] != watermark or expired)
    identity = state["instance_id"] if state else uuid.uuid4().hex
    snapshot = digest([identity, profile_id, VERSION, revision])
    expiry = _next_expiry(conn, profile_id, tables, instant)
    conn.execute("INSERT OR REPLACE INTO chain_index_state VALUES(?,?,?,?,?,?,?,?)", (profile_id, identity, VERSION, watermark, revision, snapshot, encoded(coverage), expiry))
    return dict(conn.execute("SELECT * FROM chain_index_state WHERE profile_id=?", (profile_id,)).fetchone())
