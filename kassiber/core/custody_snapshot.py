"""Version-1 raw custody facts. Loading is the only SQL/derivation boundary.

Never persist this document implicitly: it contains private accounting evidence.
It is neither an AI projection nor an import/authority admission API.
"""
from __future__ import annotations

from dataclasses import asdict
import json
from typing import Any, Mapping

from ..errors import AppError
from . import ownership

SCHEMA_VERSION = 1
_CONFIG_FIELDS = {"chain", "network", "samourai", "chain_instance_id", "bitcoin_network", "chain_network"}


def config_facts(value):
    if value in (None, ""):
        return None
    try:
        config = json.loads(value) if isinstance(value, str) else value
    except (TypeError, ValueError):
        return {"$unparsable": True}
    if not isinstance(config, Mapping):
        return {"$not_an_object": type(config).__name__}
    facts = {key: val for key, val in config.items() if key in _CONFIG_FIELDS}
    if "samourai" in facts:
        block = facts["samourai"]
        facts["samourai"] = {key: block[key] for key in ("role", "group_id", "section") if key in block} if isinstance(block, Mapping) else None
    facts["$text_mentions_samourai"] = "samourai" in str(value).lower()
    return facts


def normalized_config(value):
    facts = config_facts(value) or {}
    return json.dumps({key: val for key, val in facts.items() if not key.startswith("$")}, ensure_ascii=False)


def _safe_row(row):
    item = dict(row)
    for key in ("config_json", "wallet_config_json"):
        if key in item:
            if key == "config_json":
                item["config_text_mentions_samourai"] = bool((config_facts(item[key]) or {}).get("$text_mentions_samourai"))
            item[key] = normalized_config(item[key])
    item.pop("_closed_confirmation_observation", None)
    return item


def _referenced_strings(rows):
    from .onchain import parse_ownership_tx
    found = set()
    for row in rows:
        graph = parse_ownership_tx(row.get("raw_json"))
        if graph is None:
            continue
        for entry in (*graph["inputs"], *graph["outputs"]):
            if entry.get("script"):
                found.add(str(entry["script"]).lower())
            if entry.get("outpoint"):
                outpoint = str(entry["outpoint"]).lower()
                found.update((outpoint, outpoint.split(":", 1)[0]))
        if graph.get("txid"):
            found.add(str(graph["txid"]).lower())
    return found


def _match_facts(match):
    return {key: getattr(match, key) for key in ("wallet_id", "wallet_label", "account", "chain", "network", "source", "wallet_kind")}


def encode_owned_index(index, rows):
    if index is None:
        return None
    references = _referenced_strings(rows)
    result = {}
    for name in ("by_script", "by_outpoint"):
        result[name] = {key: [_match_facts(item) for item in (items if isinstance(items, list) else [items])] for key, items in getattr(index, name).items() if key.lower() in references}
    for name in ("by_scoped_outpoint", "txid_wallets_by_scope"):
        result[name] = [{"key": list(key), "matches": [_match_facts(item) for item in items] if name == "by_scoped_outpoint" else sorted(items)} for key, items in sorted(getattr(index, name).items()) if key[2].lower() in references]
    result["txid_wallets"] = {key: sorted(items) for key, items in index.txid_wallets.items() if key.lower() in references}
    return result


def decode_owned_index(facts):
    if facts is None:
        return None
    index = ownership.OwnedIndex()
    for name in ("by_script", "by_outpoint"):
        setattr(index, name, {key: [ownership.OwnedMatch(branch_label="", address_index=None, derivation_path=None, **item) for item in items] for key, items in facts[name].items()})
    index.by_scoped_outpoint = {tuple(row["key"]): [ownership.OwnedMatch(branch_label="", address_index=None, derivation_path=None, **item) for item in row["matches"]] for row in facts["by_scoped_outpoint"]}
    index.txid_wallets_by_scope = {tuple(row["key"]): {tuple(item) for item in row["matches"]} for row in facts["txid_wallets_by_scope"]}
    index.txid_wallets = {key: {tuple(item) for item in items} for key, items in facts["txid_wallets"].items()}
    return index


def _network_facts(conn, profile_id):
    if not conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='book_network_bindings'").fetchone():
        return None
    binding = conn.execute("SELECT * FROM book_network_bindings WHERE profile_id=?", (profile_id,)).fetchone()
    observations = [_safe_row(row) for row in conn.execute("SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id", (profile_id,))]
    references = []
    for table in ("chain_analysis_observations", "chain_observer_instances", "chain_observation_provenance", "wallet_policy_epochs", "wallet_utxos"):
        if not conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone():
            continue
        columns = {row["name"] for row in conn.execute(f'PRAGMA table_info("{table}")')}
        if not {"profile_id", "chain", "network"} <= columns:
            continue
        # Private policy bytes never cross the seam; only their presence is
        # needed to distinguish an empty retired placeholder epoch.
        for row in conn.execute(f'SELECT * FROM "{table}" WHERE profile_id=?', (profile_id,)):
            source = dict(row)
            item = {"table": table, "chain": source["chain"], "network": source["network"], "payload_json": source.get("payload_json")}
            if table == "wallet_policy_epochs":
                from .book_network import _json
                item["private_material_present"] = bool(_json(source.get("private_material_json")))
                item["source_present"] = bool(conn.execute("SELECT 1 FROM wallet_policy_sources WHERE epoch_id=? LIMIT 1", (source["id"],)).fetchone())
            references.append(item)
    return {"binding": dict(binding) if binding else None, "observations": observations, "references": references}


def require_snapshot_accounting(snapshot):
    from .book_network import _error, _evidence, _json
    facts = snapshot["book_network"]
    if facts is None:
        return
    environments, instances = set(), set()
    conflict = False
    by_wallet = {}
    for row in facts["observations"]:
        by_wallet.setdefault(row["wallet_id"], []).append(row)
    for wallet in snapshot["wallets"]:
        for row in by_wallet.get(wallet["id"]) or [{"wallet_kind": wallet["kind"], "config_json": wallet["config_json"]}]:
            environment, instance, valid = _evidence(row)
            conflict |= not valid
            if valid and environment:
                environments.add(environment)
                if environment == "regtest" and instance:
                    instances.add(instance)
    for source in facts["references"]:
        evidence = {"raw_json": _json(source["payload_json"]), "config_json": {"chain": source["chain"], "network": source["network"]}}
        if source["table"] == "wallet_policy_epochs" and not source["private_material_present"] and not source["source_present"]:
            evidence = {}
        environment, instance, valid = _evidence(evidence)
        conflict |= not valid
        if valid and environment:
            environments.add(environment)
            if instance:
                instances.add(instance)
    state = "conflicted" if conflict else "mixed" if len(environments) > 1 or len(instances) > 1 else "unbound"
    if state in {"mixed", "conflicted"}:
        _error("This book contains incompatible network histories", code="book_network_review_required", details={"state": state})
    binding = facts["binding"]
    if binding and (any(value != binding["environment"] for value in environments) or any(value != binding["chain_instance_id"] for value in instances)):
        _error("Book observations no longer match its network binding", code="book_network_review_required")


def load_custody_snapshot(conn, profile, *, as_of: str):
    """Load raw facts inside the journal transaction, after its repairs/pricing.

    Direct builder callers keep their existing transaction ownership. No
    repairs, writes, clock reads or external I/O happen here.
    """
    from .custody_journal import CustodyJournalBuilder, transaction_rate_rows
    from . import book_reset, custody_gap_reviews, custody_gaps, source_overlap
    from .import_identity import duplicate_review_quarantines
    builder = CustodyJournalBuilder(conn, profile, as_of=as_of)
    profile_id = str(profile["id"])
    network = _network_facts(conn, profile_id)
    raw_rows = builder._transactions()
    rows = [_safe_row(row) for row in raw_rows]
    wallets = [_safe_row(row) for row in conn.execute("SELECT w.id, w.label, w.kind, w.config_json, w.account_id AS wallet_account_id, COALESCE(a.code, 'treasury') AS account_code, COALESCE(a.label, 'Treasury') AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=?", (profile_id,))]
    owned_index, ownership_warnings = None, []
    if any(row["observation_authority_version"] is not None for row in rows):
        index, ownership_warnings = ownership.build_owned_index(conn, profile_id, ownership.load_profile_wallets(conn, profile_id))
        owned_index = encode_owned_index(index, rows)
    headers = [dict(row) for row in conn.execute("SELECT * FROM custody_components WHERE profile_id=? AND state='active' ORDER BY created_at,revision,id", (profile_id,))]
    components, anchor_ids, wallet_ids, revision_ids = [], set(), set(), set()
    for header in headers:
        item = {"header": header}
        for key, table in (("legs", "custody_component_legs"), ("allocations", "custody_component_allocations"), ("economic_terms", "custody_component_economic_terms"), ("commitments", "custody_component_evidence_commitments")):
            scope = " AND profile_id=?" if key == "commitments" else ""
            parameters = (header["id"], profile_id) if key == "commitments" else (header["id"],)
            item[key] = [dict(row) for row in conn.execute(f"SELECT * FROM {table} WHERE component_id=?{scope} ORDER BY ordinal,id", parameters)]
        for leg in item["legs"]:
            anchor_ids.update(str(value) for value in (leg.get("transaction_id"), leg.get("anchor_transaction_id")) if value is not None)
            if leg.get("wallet_id") is not None:
                wallet_ids.add(str(leg["wallet_id"]))
        revision_ids.update(header[key] for key in ("supersedes_component_id", "superseded_by_component_id") if header[key] is not None)
        components.append(item)
    anchors = {}
    for transaction_id in sorted(anchor_ids):
        row = conn.execute("SELECT t.*,w.kind AS wallet_kind,w.config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.id=?", (transaction_id,)).fetchone()
        anchors[transaction_id] = _safe_row(row) if row is not None else None
    anchor_wallets = []
    for wallet_id in sorted(wallet_ids):
        row = conn.execute("SELECT id,kind,config_json FROM wallets WHERE id=?", (wallet_id,)).fetchone()
        if row is not None:
            anchor_wallets.append(_safe_row(row))
    revision_headers = []
    # INTEGER and REAL share Python's numeric ordering; TEXT and BLOB do not.
    # Refuse before sorting the set so hash randomization cannot pick an error.
    revision_types = {"number" if isinstance(value, (int, float)) else type(value).__name__ for value in revision_ids}
    if len(revision_types) > 1:
        raise AppError("Custody revision references contain incompatible types", code="validation")
    for component_id in sorted(revision_ids):
        row = conn.execute("SELECT * FROM custody_components WHERE id=?", (component_id,)).fetchone()
        if row is not None:
            revision_headers.append(dict(row))
    channel_records = [_safe_row(row) for row in conn.execute("SELECT r.txid,r.outpoint,r.tag,r.wallet_id,r.channel_id,r.amount_msat,r.raw_json,w.config_json,b.chain,b.network FROM lightning_node_records r JOIN wallets w ON w.id=r.wallet_id LEFT JOIN backends b ON b.name=r.backend_name WHERE r.profile_id=? AND r.record_type='channel'", (profile_id,))]
    worklist, large_book = custody_gaps.load_gap_search_worklist(conn, profile_id)
    # Preserve rate precedence independently of the active-row tie order.
    rates = [dict(row) for row in transaction_rate_rows(conn, profile_id)]
    snapshot = {
        "schema_version": SCHEMA_VERSION, "operation": "custody", "as_of": as_of,
        "profile": dict(profile), "book_network": network, "wallets": wallets,
        "transactions": rows, "rate_rows": rates, "owned_index": owned_index,
        "ownership_warnings": list(ownership_warnings or ()), "components": components,
        "anchors": anchors, "anchor_wallets": anchor_wallets, "revision_headers": revision_headers,
        "receipt_sources": [], "channel_records": channel_records,
        "loan_legs": [dict(row) for row in conn.execute("SELECT transaction_id,role FROM loan_legs WHERE profile_id=? AND deleted_at IS NULL", (profile_id,))],
        "evidence_snapshots": [dict(row) for row in conn.execute("SELECT subject_id,detail_hash,quantity_hash,payload_json,created_at FROM custody_authored_evidence_snapshots WHERE profile_id=? AND subject_kind='custody_component' ORDER BY subject_id,quantity_hash,detail_hash", (profile_id,))],
        "migration_issues": [dict(row) for row in conn.execute("SELECT * FROM custody_authored_migration_issues WHERE profile_id=? AND resolved_at IS NULL ORDER BY created_at,id", (profile_id,))],
        # Held ID-less import duplicates, read from transactions and batches
        # like the gap worklist; compute treats them as migration holds.
        "import_duplicate_quarantines": list(duplicate_review_quarantines(conn, profile_id)),
        # Kept decisions a reset could not bind, shaped as quarantines;
        # compute treats them as migration holds.
        "reset_hold_quarantines": list(book_reset.held_quarantines(conn, profile_id)),
        "gap_reviews": custody_gap_reviews.latest_review_rows(conn, profile_id),
        "pair_dismissals": [dict(row) for row in conn.execute("SELECT * FROM transaction_pair_dismissals WHERE profile_id=?", (profile_id,))],
        "gap_worklist": {"large_book": large_book, "rows": [_safe_row(row) for row in worklist]},
    }
    try:
        receipt_sources = []
        receipt_groups = source_overlap._receipt_overlap_candidate_groups(rows)
        if receipt_groups:
            from .onchain import parse_ownership_tx
            scripts = {
                output.get("script")
                for group in receipt_groups
                for row in group
                for output in (parse_ownership_tx(row.get("raw_json")) or {}).get("outputs", ())
            }
            receipt_sources = [asdict(source) for source in source_overlap.build_profile_source_index(conn, profile_id).sources if source.script_pubkey in scripts]
    except Exception:
        # Loading receipt sources used to follow component materialization.
        # On this exceptional path preserve its error precedence using only
        # the raw snapshot; successful builds compute exactly once, after load.
        from .custody_compute import compute_components_stage
        require_snapshot_accounting(snapshot)
        from ..tax_policy import require_tax_processing_supported
        require_tax_processing_supported(profile)
        compute_components_stage(snapshot)
        raise
    snapshot["receipt_sources"] = receipt_sources
    return snapshot
