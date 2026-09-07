"""Book-wide network policy. Authored scope never upgrades observation authority."""
from __future__ import annotations

import hashlib
import json
import sqlite3
import uuid
from collections.abc import Mapping
from typing import Any

from ..errors import AppError
from ..time_utils import now_iso
from ..transfers import bitcoin_network_domain_evidence
from ..wallet_descriptors import normalize_network

ENVIRONMENTS = {
    "main": {"bitcoin": "main", "liquid": "liquidv1"},
    "test": {"bitcoin": "test", "liquid": "liquidtestnet"},
    "signet": {"bitcoin": "signet"},
    "regtest": {"bitcoin": "regtest", "liquid": "elementsregtest"},
}


def _json(value):
    if isinstance(value, Mapping):
        return dict(value)
    try:
        result = json.loads(value or "{}")
        return result if isinstance(result, dict) else {}
    except (ValueError, TypeError):
        return {}


def _digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), default=str).encode()).hexdigest()


def _error(message, *, code="book_network_mismatch", details=None):
    raise AppError(message, code=code, details=details or {}, hint="Review the active book's network in Settings. Existing history cannot be relabeled.")


def _binding(conn, profile_id):
    try:
        row = conn.execute("SELECT * FROM book_network_bindings WHERE profile_id=?", (profile_id,)).fetchone()
    except sqlite3.OperationalError as exc:
        if "no such table" not in str(exc):
            raise
        return None
    return dict(row) if row else None


def _instance_evidence(row):
    raw, config = _json(row.get("raw_json")), _json(row.get("wallet_config_json") or row.get("config_json"))
    values = {str(value).strip() for value in (raw.get("chain_instance_id"), config.get("chain_instance_id")) if value}
    return (next(iter(values)) if len(values) == 1 else None), len(values) <= 1


def _evidence(row):
    environment, valid = bitcoin_network_domain_evidence(row)
    instance, instance_valid = _instance_evidence(row)
    return environment, instance, valid and instance_valid


def inventory_book_network(conn, profile_id):
    profile = conn.execute("SELECT id, journal_input_version FROM profiles WHERE id=?", (profile_id,)).fetchone()
    if profile is None:
        _error("Book not found", code="not_found")
    wallets = [dict(row) for row in conn.execute("SELECT id,label,kind,config_json FROM wallets WHERE profile_id=? ORDER BY id", (profile_id,))]
    observations = [dict(row) for row in conn.execute("SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id", (profile_id,))]
    by_wallet = {wallet["id"]: [] for wallet in wallets}
    for row in observations:
        by_wallet[row["wallet_id"]].append(row)
    rows = []
    fingerprint = [dict(profile), wallets, observations]
    all_environments, all_instances = set(), set()
    for wallet in wallets:
        evidence = by_wallet[wallet["id"]] or [{"wallet_kind": wallet["kind"], "config_json": wallet["config_json"]}]
        known, instances, conflicts, unknown = set(), set(), [], []
        for row in evidence:
            environment, instance, valid = _evidence(row)
            if not valid:
                conflicts.append(row.get("id"))
            elif environment:
                known.add(environment)
                if environment == "regtest" and instance:
                    instances.add(instance)
            else:
                unknown.append(row.get("id"))
        all_environments.update(known)
        all_instances.update(instances)
        rows.append({"wallet_id": wallet["id"], "label": wallet["label"], "environments": sorted(known), "chain_instances": sorted(instances), "transaction_count": len(by_wallet[wallet["id"]]), "unknown_count": len(unknown), "conflict_count": len(conflicts), "conflicting_transaction_ids": [value for value in conflicts if value], "requires_declaration": bool(unknown) or ("regtest" in known and not instances)})
    references = []
    reference_conflicts = 0
    for table in ("chain_analysis_observations", "chain_observer_instances", "chain_observation_provenance", "wallet_policy_epochs", "wallet_utxos"):
        if not conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (table,)).fetchone():
            continue
        columns = {row["name"] for row in conn.execute(f'PRAGMA table_info("{table}")')}
        if not {"profile_id", "chain", "network"} <= columns:
            continue
        source_rows = [dict(row) for row in conn.execute(f'SELECT * FROM "{table}" WHERE profile_id=?', (profile_id,))]
        fingerprint.append([table, source_rows])
        for source in source_rows:
            evidence = {"raw_json": _json(source.get("payload_json")), "config_json": {"chain": source["chain"], "network": source["network"]}}
            environment, instance, valid = _evidence(evidence)
            if not valid:
                reference_conflicts += 1
            elif environment:
                all_environments.add(environment)
                if instance:
                    all_instances.add(instance)
            references.append({"source": table, "environment": environment, "chain_instance_id": instance, "valid": valid})
    state = "conflicted" if reference_conflicts or any(row["conflict_count"] for row in rows) else "mixed" if len(all_environments) > 1 or len(all_instances) > 1 else "unbound"
    return {"profile_id": profile_id, "state": state, "environments": sorted(all_environments), "chain_instances": sorted(all_instances), "wallets": rows, "reference_scopes": references, "inventory_digest": _digest(fingerprint), "transaction_count": len(observations)}


def resolve_book_environment(conn, profile_id):
    binding = _binding(conn, profile_id)
    if not binding:
        return {"profile_id": profile_id, "state": "unbound", "environment_id": None, "revision": 0, "domains": []}
    return {"profile_id": profile_id, "state": "bound", "environment_id": binding["environment_id"], "environment": binding["environment"], "revision": binding["revision"], "chain_instance_id": binding["chain_instance_id"], "domains": json.loads(binding["domains_json"]), "declared_wallet_ids": json.loads(binding["acknowledgements_json"])}


def _domains(environment, instance):
    return [{"domain_id": _digest([chain, network, instance]), "chain": chain, "network": network, "chain_instance_id": instance} for chain, network in ENVIRONMENTS[environment].items()]


def plan_book_network(conn, profile_id, args):
    environment = args.get("environment")
    if environment not in ENVIRONMENTS:
        _error("Choose a supported book environment", code="validation")
    instance = args.get("chain_instance_id") or None
    if environment == "regtest":
        try:
            instance = str(uuid.UUID(str(instance)))
        except (ValueError, TypeError, AttributeError):
            _error("A local regtest instance needs an explicit UUID", code="validation")
    elif instance is not None:
        _error("Standard networks cannot have a local instance identity", code="validation")
    declared = args.get("declared_wallet_ids", [])
    if not isinstance(declared, list) or any(not isinstance(value, str) for value in declared):
        _error("Wallet declarations must be wallet IDs", code="validation")
    inventory = inventory_book_network(conn, profile_id)
    ids = {row["wallet_id"] for row in inventory["wallets"]}
    if not set(declared) <= ids:
        _error("Wallet declaration is outside this book", code="validation")
    blockers = []
    if _binding(conn, profile_id):
        blockers.append({"code": "already_bound"})
    for row in inventory["wallets"]:
        if row["conflict_count"]:
            blockers.append({"code": "contradictory_evidence", "wallet_id": row["wallet_id"]})
        if any(value != environment for value in row["environments"]):
            blockers.append({"code": "different_environment", "wallet_id": row["wallet_id"]})
        if any(value != instance for value in row["chain_instances"]):
            blockers.append({"code": "different_instance", "wallet_id": row["wallet_id"]})
        if row["requires_declaration"] and row["wallet_id"] not in declared:
            blockers.append({"code": "scope_declaration_required", "wallet_id": row["wallet_id"]})
    if any(not item["valid"] or item["environment"] not in (None, environment) or item["chain_instance_id"] not in (None, instance) for item in inventory["reference_scopes"]):
        blockers.append({"code": "reference_domain_mismatch"})
    recipe = {"profile_id": profile_id, "environment": environment, "chain_instance_id": instance, "declared_wallet_ids": sorted(set(declared)), "inventory_digest": inventory["inventory_digest"]}
    return {**recipe, "plan_id": _digest(recipe), "domains": _domains(environment, instance), "blockers": blockers, "can_apply": not blockers, "inventory": inventory}


def apply_book_network(conn, profile_id, args):
    plan = plan_book_network(conn, profile_id, args)
    if args.get("plan_id") != plan["plan_id"]:
        _error("Book network preview is stale", code="stale_context")
    if not plan["can_apply"]:
        _error("Book network binding needs review", code="book_network_review_required", details={"blockers": plan["blockers"]})
    conn.execute("INSERT INTO book_network_bindings(profile_id,environment_id,environment,revision,chain_instance_id,domains_json,acknowledgements_json,inventory_digest,created_at) VALUES(?,?,?,1,?,?,?,?,?)", (profile_id, str(uuid.uuid4()), plan["environment"], plan["chain_instance_id"], json.dumps(plan["domains"], sort_keys=True), json.dumps(plan["declared_wallet_ids"]), plan["inventory_digest"], now_iso()))
    from .repo.context import invalidate_journals
    invalidate_journals(conn, profile_id)
    return resolve_book_environment(conn, profile_id)


def require_chain_domain(conn, profile_id, chain, network, *, chain_instance_id=None, operation="analysis"):
    binding = resolve_book_environment(conn, profile_id)
    if binding["state"] != "bound":
        _error("Bind this book to a network environment first", code="book_network_unbound")
    base_chain = "bitcoin" if chain in {"bitcoin", "lightning"} else chain
    try:
        canonical = normalize_network(base_chain, network)
    except ValueError:
        _error("Unsupported chain domain", code="validation")
    match = next((domain for domain in binding["domains"] if domain["chain"] == base_chain and domain["network"] == canonical), None)
    if match is None or (chain_instance_id is not None and chain_instance_id != match["chain_instance_id"]):
        _error("Source belongs to a different book network", details={"operation": operation, "profile_id": profile_id})
    return {**match, "environment_id": binding["environment_id"], "revision": binding["revision"]}


def guard_observation(conn, profile_id, row, *, operation="import"):
    binding = _binding(conn, profile_id)
    environment, instance, valid = _evidence(dict(row))
    if not valid:
        _error("Observation contradicts its configured network", details={"operation": operation})
    if not binding:
        return
    if environment and environment != binding["environment"]:
        _error("Observation belongs to a different book environment", details={"operation": operation})
    if instance and instance != binding["chain_instance_id"]:
        _error("Observation belongs to a different local chain instance", details={"operation": operation})
    # Missing source evidence stays missing. This admission policy grants no
    # ownership, physical-network evidence, or custody authority to the row.


def guard_wallet(conn, profile_id, config, *, previous_config=None, operation="wallet"):
    if previous_config is not None:
        old, new = _evidence({"config_json": previous_config}), _evidence({"config_json": config})
        if old != new and (old[0] is not None or old[1] is not None):
            _error("A connection's historical network cannot be changed; create a new connection")
    guard_observation(conn, profile_id, {"config_json": config}, operation=operation)


def require_book_accounting(conn, profile_id):
    if not conn.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='book_network_bindings'").fetchone():
        return
    inventory = inventory_book_network(conn, profile_id)
    binding = _binding(conn, profile_id)
    if inventory["state"] in {"mixed", "conflicted"}:
        _error("This book contains incompatible network histories", code="book_network_review_required", details={"state": inventory["state"]})
    if binding and (any(value != binding["environment"] for value in inventory["environments"]) or any(value != binding["chain_instance_id"] for value in inventory["chain_instances"])):
        _error("Book observations no longer match its network binding", code="book_network_review_required")


def new_wallet_config(conn, profile_id, kind, config):
    config = dict(config)
    binding = _binding(conn, profile_id)
    if not binding:
        return config
    if any(config.get(key) for key in ("chain", "network", "descriptor", "change_descriptor", "xpub", "addresses")) or kind in {"coreln", "lnd", "lightning", "nwc", "phoenix"}:
        chain = "liquid" if config.get("chain") in {"liquid", "lbtc", "elements"} or kind == "liquid-electrum" else "bitcoin"
        if chain not in ENVIRONMENTS[binding["environment"]]:
            _error("This chain has no domain in the book's environment")
        config.setdefault("chain", chain)
        config.setdefault("network", ENVIRONMENTS[binding["environment"]][chain])
        if binding["chain_instance_id"]:
            config.setdefault("chain_instance_id", binding["chain_instance_id"])
    return config


def observation_matches_binding(binding, row, *, unscoped=False):
    """Fast read filter. Shared caches cannot borrow a local instance identity."""
    if binding.get("state") != "bound":
        return True
    environment, instance, valid = _evidence(dict(row))
    if not valid or environment != binding["environment"]:
        return False
    expected_instance = binding.get("chain_instance_id")
    if instance is not None and instance != expected_instance:
        return False
    return not (unscoped and expected_instance and instance != expected_instance)
