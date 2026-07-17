from __future__ import annotations

"""Concrete wallet-source overlap detection.

Descriptor/xpub sources are unbounded, so this module never claims global
non-overlap. It compares only finite evidence Kassiber can name today:
resolved sync targets, address-list scripts, and existing wallet_utxos rows.
"""

import json
import sqlite3
from dataclasses import dataclass, replace
from typing import Any, Mapping, Sequence

from ..errors import AppError
from ..util import normalize_chain_value, normalize_network_value
from ..wallet_descriptors import derive_descriptor_targets
from . import freshness as core_freshness
from .address_scripts import scriptpubkey_for_address_or_none
from .onchain import (
    input_script,
    normalized_script_hex,
    output_script,
    stored_tx_mapping,
)
from .wallets import (
    has_descriptor_sync_material,
    load_wallet_descriptor_plan_from_config,
    normalize_addresses,
    wallet_is_deprecated,
)

MAX_STORED_DESCRIPTOR_TARGETS_PER_BRANCH = 20_000
_CANONICAL_WALLET_KINDS = {"descriptor", "xpub", "silent-payment"}
_SOURCE_PRIORITY = {
    "inventory": 0,
    "descriptor_config": 0,
    "address_list": 1,
    "sync_target": 2,
}


@dataclass(frozen=True, slots=True)
class SourceScript:
    profile_id: str
    wallet_id: str
    wallet_label: str
    wallet_kind: str
    chain: str
    network: str
    script_pubkey: str
    source: str
    deprecated: bool = False
    active_transaction_count: int = 0
    branch_index: int | None = None
    branch_label: str | None = None
    address_index: int | None = None


@dataclass(frozen=True, slots=True)
class ProfileSourceIndex:
    profile_id: str
    sources: tuple[SourceScript, ...]


def _row_get(row: Mapping[str, Any] | sqlite3.Row, key: str, default: Any = None) -> Any:
    try:
        return row[key]
    except (KeyError, IndexError, TypeError):
        getter = getattr(row, "get", None)
        if callable(getter):
            return getter(key, default)
    return default


def _wallet_config(wallet: Mapping[str, Any] | sqlite3.Row) -> dict[str, Any]:
    raw = _row_get(wallet, "config_json") or "{}"
    try:
        loaded = json.loads(raw) if isinstance(raw, str) else dict(raw)
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return loaded if isinstance(loaded, dict) else {}


def _normalize_chain_network(chain: Any, network: Any) -> tuple[str, str]:
    if chain in (None, "") or network in (None, ""):
        return "", ""
    try:
        normalized_chain = normalize_chain_value(chain)
        normalized_network = normalize_network_value(normalized_chain, network)
    except AppError:
        return str(chain or "").strip().lower(), str(network or "").strip().lower()
    return normalized_chain, normalized_network


def _script_pubkey_from_raw_json(value: Any) -> str | None:
    raw = stored_tx_mapping(value)
    return normalized_script_hex(raw.get("script_pubkey")) if raw else None


def _wallet_label_lookup(conn: sqlite3.Connection, profile_id: str) -> dict[str, str]:
    return {
        str(row["id"]): str(row["label"])
        for row in conn.execute(
            "SELECT id, label FROM wallets WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }


def _active_transaction_counts(conn: sqlite3.Connection, profile_id: str) -> dict[str, int]:
    return {
        str(row["wallet_id"]): int(row["count"] or 0)
        for row in conn.execute(
            """
            SELECT wallet_id, COUNT(*) AS count
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            GROUP BY wallet_id
            """,
            (profile_id,),
        ).fetchall()
    }


def _wallets_for_profile(conn: sqlite3.Connection, profile_id: str) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT id, label, kind, config_json
        FROM wallets
        WHERE profile_id = ?
        ORDER BY label ASC, id ASC
        """,
        (profile_id,),
    ).fetchall()


def _address_list_scripts(
    profile_id: str,
    wallet: Mapping[str, Any] | sqlite3.Row,
    config: Mapping[str, Any],
    active_counts: Mapping[str, int],
) -> list[SourceScript]:
    wallet_id = str(_row_get(wallet, "id"))
    chain, network = _normalize_chain_network(config.get("chain"), config.get("network"))
    if not chain or not network:
        return []
    scripts: list[SourceScript] = []
    for index, address in enumerate(normalize_addresses(config.get("addresses"))):
        script_pubkey = scriptpubkey_for_address_or_none(address)
        if not script_pubkey:
            continue
        scripts.append(
            SourceScript(
                profile_id=profile_id,
                wallet_id=wallet_id,
                wallet_label=str(_row_get(wallet, "label") or wallet_id),
                wallet_kind=str(_row_get(wallet, "kind") or "address"),
                chain=chain,
                network=network,
                script_pubkey=script_pubkey,
                source="address_list",
                deprecated=wallet_is_deprecated(dict(config)),
                active_transaction_count=int(active_counts.get(wallet_id, 0)),
                branch_label="address",
                address_index=index,
            )
        )
    return scripts


def _inventory_scripts(
    conn: sqlite3.Connection,
    profile_id: str,
    active_counts: Mapping[str, int],
) -> list[SourceScript]:
    labels = _wallet_label_lookup(conn, profile_id)
    wallet_rows = {
        str(row["id"]): row
        for row in conn.execute(
            "SELECT id, kind, config_json FROM wallets WHERE profile_id = ?",
            (profile_id,),
        ).fetchall()
    }
    rows = conn.execute(
        """
        SELECT DISTINCT
            wallet_id, chain, network, address, script_pubkey, branch_label,
            branch_index, address_index, raw_json
        FROM wallet_utxos
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    scripts: list[SourceScript] = []
    for row in rows:
        wallet_id = str(row["wallet_id"])
        script_pubkey = (
            normalized_script_hex(row["script_pubkey"])
            or _script_pubkey_from_raw_json(row["raw_json"])
            or scriptpubkey_for_address_or_none(row["address"])
        )
        if not script_pubkey:
            continue
        wallet = wallet_rows.get(wallet_id, {})
        config = _wallet_config(wallet)
        chain, network = _normalize_chain_network(row["chain"], row["network"])
        scripts.append(
            SourceScript(
                profile_id=profile_id,
                wallet_id=wallet_id,
                wallet_label=labels.get(wallet_id, wallet_id),
                wallet_kind=str(_row_get(wallet, "kind") or ""),
                chain=chain,
                network=network,
                script_pubkey=script_pubkey,
                source="inventory",
                deprecated=wallet_is_deprecated(config),
                active_transaction_count=int(active_counts.get(wallet_id, 0)),
                branch_index=row["branch_index"],
                branch_label=row["branch_label"],
                address_index=row["address_index"],
            )
        )
    return scripts


def _highest_used_checkpoints(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, dict[int, int]]:
    prefix = f"{core_freshness.SOURCE_ONCHAIN}:"
    rows = conn.execute(
        """
        SELECT source_key, checkpoint_json
        FROM freshness_source_states
        WHERE profile_id = ?
          AND source_key LIKE ?
        """,
        (profile_id, f"{prefix}%"),
    ).fetchall()
    output: dict[str, dict[int, int]] = {}
    for row in rows:
        wallet_id = str(row["source_key"])[len(prefix) :]
        try:
            checkpoint = json.loads(row["checkpoint_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        highest_used = checkpoint.get("highest_used") if isinstance(checkpoint, dict) else None
        if not isinstance(highest_used, dict):
            continue
        branch_values: dict[int, int] = {}
        for branch, value in highest_used.items():
            try:
                branch_values[int(branch)] = int(value)
            except (TypeError, ValueError):
                continue
        if branch_values:
            output[wallet_id] = branch_values
    return output


def _inventory_branch_maxes(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, dict[int, int]]:
    rows = conn.execute(
        """
        SELECT wallet_id, branch_index, MAX(address_index) AS max_index
        FROM wallet_utxos
        WHERE profile_id = ?
          AND branch_index IS NOT NULL
          AND address_index IS NOT NULL
        GROUP BY wallet_id, branch_index
        """,
        (profile_id,),
    ).fetchall()
    output: dict[str, dict[int, int]] = {}
    for row in rows:
        try:
            branch = int(row["branch_index"])
            max_index = int(row["max_index"])
        except (TypeError, ValueError):
            continue
        output.setdefault(str(row["wallet_id"]), {})[branch] = max_index
    return output


def _candidate_branch_maxes(candidate_scripts: Sequence["SourceScript"]) -> dict[str, dict[int, int]]:
    output: dict[str, dict[int, int]] = {}
    for source in candidate_scripts:
        if source.branch_index is None or source.address_index is None:
            continue
        output.setdefault(source.wallet_id, {})[int(source.branch_index)] = max(
            output.get(source.wallet_id, {}).get(int(source.branch_index), -1),
            int(source.address_index),
        )
    return output


def _merged_branch_maxes(
    *sources: Mapping[str, Mapping[int, int]],
) -> dict[str, dict[int, int]]:
    merged: dict[str, dict[int, int]] = {}
    for source in sources:
        for wallet_id, branch_values in source.items():
            wallet_values = merged.setdefault(wallet_id, {})
            for branch, value in branch_values.items():
                wallet_values[int(branch)] = max(wallet_values.get(int(branch), -1), int(value))
    return merged


def _descriptor_config_scripts(
    conn: sqlite3.Connection,
    profile_id: str,
    active_counts: Mapping[str, int],
    candidate_scripts: Sequence["SourceScript"],
) -> list[SourceScript]:
    branch_maxes = _merged_branch_maxes(
        _inventory_branch_maxes(conn, profile_id),
        _highest_used_checkpoints(conn, profile_id),
        _candidate_branch_maxes(candidate_scripts),
    )
    scripts: list[SourceScript] = []
    for wallet in _wallets_for_profile(conn, profile_id):
        config = _wallet_config(wallet)
        if not has_descriptor_sync_material(config):
            continue
        try:
            plan = load_wallet_descriptor_plan_from_config(config)
        except (AppError, ValueError):
            continue
        if plan is None:
            continue
        wallet_id = str(_row_get(wallet, "id"))
        deprecated = wallet_is_deprecated(config)
        for branch in plan.branches:
            known_max = branch_maxes.get(wallet_id, {}).get(branch.branch_index)
            end = plan.gap_limit if known_max is None else known_max + plan.gap_limit + 1
            end = max(0, min(int(end), MAX_STORED_DESCRIPTOR_TARGETS_PER_BRANCH))
            try:
                targets = derive_descriptor_targets(
                    plan,
                    branch_index=branch.branch_index,
                    start=0,
                    end=end,
                )
            except (AppError, ValueError):
                continue
            for target in targets:
                scripts.append(
                    SourceScript(
                        profile_id=profile_id,
                        wallet_id=wallet_id,
                        wallet_label=str(_row_get(wallet, "label") or wallet_id),
                        wallet_kind=str(_row_get(wallet, "kind") or ""),
                        chain=plan.chain,
                        network=plan.network,
                        script_pubkey=target.script_pubkey.lower(),
                        source="descriptor_config",
                        deprecated=deprecated,
                        active_transaction_count=int(active_counts.get(wallet_id, 0)),
                        branch_index=target.branch_index,
                        branch_label=target.branch_label,
                        address_index=target.address_index,
                    )
                )
    return scripts


def _candidate_descriptor_supplements(
    conn: sqlite3.Connection,
    profile_id: str,
    profile_index: ProfileSourceIndex,
    candidate_scripts: Sequence[SourceScript],
) -> list[SourceScript]:
    """Extend only descriptor branches whose candidate horizon grew.

    The operation-scoped index contains each descriptor's baseline finite
    horizon. A widened candidate (for example a repair scan) can require the
    same candidate-dependent gap tail that the uncached path derives. Rebuild
    only that tail instead of re-deriving every indexed descriptor.
    """

    candidate_maxes = _candidate_branch_maxes(candidate_scripts)
    if not candidate_maxes:
        return []
    baseline_ends: dict[tuple[str, int], int] = {}
    active_counts: dict[str, int] = {}
    for source in profile_index.sources:
        active_counts[source.wallet_id] = max(
            active_counts.get(source.wallet_id, 0),
            int(source.active_transaction_count),
        )
        if (
            source.source != "descriptor_config"
            or source.branch_index is None
            or source.address_index is None
        ):
            continue
        key = (source.wallet_id, int(source.branch_index))
        baseline_ends[key] = max(
            baseline_ends.get(key, 0),
            int(source.address_index) + 1,
        )
    supplements: list[SourceScript] = []
    for wallet_id, branch_maxes in candidate_maxes.items():
        wallet = conn.execute(
            """
            SELECT id, label, kind, config_json
            FROM wallets
            WHERE profile_id = ? AND id = ?
            """,
            (profile_id, wallet_id),
        ).fetchone()
        if wallet is None:
            continue
        config = _wallet_config(wallet)
        if not has_descriptor_sync_material(config):
            continue
        try:
            plan = load_wallet_descriptor_plan_from_config(config)
        except (AppError, ValueError):
            continue
        if plan is None:
            continue
        deprecated = wallet_is_deprecated(config)
        for branch in plan.branches:
            candidate_max = branch_maxes.get(branch.branch_index)
            if candidate_max is None:
                continue
            start = baseline_ends.get((wallet_id, branch.branch_index), 0)
            end = max(start, int(candidate_max) + plan.gap_limit + 1)
            end = min(end, MAX_STORED_DESCRIPTOR_TARGETS_PER_BRANCH)
            if end <= start:
                continue
            try:
                targets = derive_descriptor_targets(
                    plan,
                    branch_index=branch.branch_index,
                    start=start,
                    end=end,
                )
            except (AppError, ValueError):
                continue
            for target in targets:
                supplements.append(
                    SourceScript(
                        profile_id=profile_id,
                        wallet_id=wallet_id,
                        wallet_label=str(_row_get(wallet, "label") or wallet_id),
                        wallet_kind=str(_row_get(wallet, "kind") or ""),
                        chain=plan.chain,
                        network=plan.network,
                        script_pubkey=target.script_pubkey.lower(),
                        source="descriptor_config",
                        deprecated=deprecated,
                        active_transaction_count=active_counts.get(wallet_id, 0),
                        branch_index=target.branch_index,
                        branch_label=target.branch_label,
                        address_index=target.address_index,
                    )
                )
    return supplements


def scripts_from_sync_state(
    profile: Mapping[str, Any] | sqlite3.Row,
    wallet: Mapping[str, Any] | sqlite3.Row,
    sync_state: Any,
) -> list[SourceScript]:
    config = _wallet_config(wallet)
    profile_id = str(_row_get(profile, "id"))
    wallet_id = str(_row_get(wallet, "id"))
    scripts: list[SourceScript] = []
    for target in getattr(sync_state, "targets", []) or []:
        script_pubkey = str(target.get("script_pubkey") or "").strip().lower()
        if not script_pubkey:
            continue
        chain, network = _normalize_chain_network(
            getattr(sync_state, "chain", "") or target.get("chain"),
            getattr(sync_state, "network", "") or target.get("network"),
        )
        scripts.append(
            SourceScript(
                profile_id=profile_id,
                wallet_id=wallet_id,
                wallet_label=str(_row_get(wallet, "label") or wallet_id),
                wallet_kind=str(_row_get(wallet, "kind") or ""),
                chain=chain,
                network=network,
                script_pubkey=script_pubkey,
                source="sync_target",
                deprecated=wallet_is_deprecated(config),
                branch_index=target.get("branch_index"),
                branch_label=target.get("branch_label"),
                address_index=target.get("address_index"),
            )
        )
    return scripts


def _include_source(
    source: SourceScript,
    *,
    candidate_wallet_ids: set[str],
    include_deprecated: bool,
) -> bool:
    if source.wallet_id in candidate_wallet_ids:
        return True
    if include_deprecated:
        return True
    if not source.deprecated:
        return True
    return source.active_transaction_count > 0


def _checked_source_counts(sources: Sequence[SourceScript]) -> list[dict[str, Any]]:
    counts: dict[str, dict[str, Any]] = {}
    for source in sources:
        if source.source not in {"descriptor_config", "sync_target"}:
            continue
        row = counts.setdefault(
            source.wallet_id,
            {
                "wallet_id": source.wallet_id,
                "wallet": source.wallet_label,
                "target_count": 0,
            },
        )
        row["target_count"] += 1
    return sorted(counts.values(), key=lambda item: (item["wallet"], item["wallet_id"]))


def _canonical_wallet_sort_key(item: Mapping[str, Any]) -> tuple[int, int, int, int, str, str]:
    kind = str(item.get("kind") or "")
    source_priority = item.get("_source_priority")
    return (
        1 if item.get("deprecated") else 0,
        0 if kind in _CANONICAL_WALLET_KINDS else 1,
        int(source_priority if source_priority is not None else 99),
        0 if item.get("active_transaction_count") else 1,
        str(item.get("label") or ""),
        str(item.get("id") or ""),
    )


def _overlap_payload(
    key: tuple[str, str, str, str],
    sources: Sequence[SourceScript],
) -> dict[str, Any]:
    _profile_id, chain, network, _script = key
    by_wallet: dict[str, dict[str, Any]] = {}
    evidence = set()
    for source in sources:
        evidence.add(source.source)
        row = by_wallet.setdefault(
            source.wallet_id,
            {
                "id": source.wallet_id,
                "label": source.wallet_label,
                "kind": source.wallet_kind,
                "deprecated": source.deprecated,
                "active_transaction_count": source.active_transaction_count,
                "sources": [],
                "_source_priority": 99,
            },
        )
        if source.source not in row["sources"]:
            row["sources"].append(source.source)
        row["_source_priority"] = min(
            int(row["_source_priority"]),
            _SOURCE_PRIORITY.get(source.source, 99),
        )
    raw_wallets = sorted(by_wallet.values(), key=lambda item: (item["label"], item["id"]))
    recommended = sorted(raw_wallets, key=_canonical_wallet_sort_key)[0]
    wallets = [
        {key: value for key, value in wallet.items() if key != "_source_priority"}
        for wallet in raw_wallets
    ]
    address_repairs = []
    for wallet in wallets:
        if wallet["id"] == recommended["id"]:
            continue
        target_count = len(
            {
                source.script_pubkey
                for source in sources
                if source.wallet_id == wallet["id"] and source.source == "address_list"
            }
        )
        if not target_count:
            continue
        address_repairs.append(
            {
                "wallet_id": wallet["id"],
                "wallet": wallet["label"],
                "overlapping_address_list_target_count": target_count,
                "action": "remove_overlapping_address_list_targets",
                "clear_output_inventory": True,
                "reset_onchain_refresh_checkpoint": True,
                "deprecate_if_empty_after_trim": True,
                "requires_confirmation": True,
            }
        )
    return {
        "chain": chain,
        "network": network,
        "wallets": wallets,
        "recommended_canonical_wallet_id": recommended["id"],
        "recommended_canonical_wallet": recommended["label"],
        "recommendation_reason": (
            "Prefer active, non-deprecated descriptor/xpub sources as canonical "
            "when the overlap is otherwise equivalent; this is a recommendation, "
            "not an automatic irreversible repair."
        ),
        "evidence": sorted(evidence),
        "address_list_repair_preview": address_repairs,
        "checked": {
            "scope": "finite_scripts_only",
            "descriptor_global_overlap_proven": False,
            "note": (
                "Only resolved sync targets, address-list scripts, stored "
                "descriptor targets within the checked horizon, and existing "
                "wallet_utxos evidence were compared. Address-list entries beyond "
                "a descriptor's checked horizon are not proven overlapping."
            ),
            "target_counts": _checked_source_counts(sources),
        },
    }


def _source_script_groups(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    candidate_scripts: Sequence[SourceScript] | None = None,
    include_deprecated: bool = False,
    profile_index: ProfileSourceIndex | None = None,
) -> tuple[dict[tuple[str, str, str, str], list[SourceScript]], list[SourceScript]]:
    candidate_scripts = list(candidate_scripts or [])
    candidate_wallet_ids = {source.wallet_id for source in candidate_scripts}
    if profile_index is not None and profile_index.profile_id == profile_id:
        sources = [
            *profile_index.sources,
            *candidate_scripts,
            *_candidate_descriptor_supplements(
                conn,
                profile_id,
                profile_index,
                candidate_scripts,
            ),
        ]
    else:
        active_counts = _active_transaction_counts(conn, profile_id)
        sources = []
        for wallet in _wallets_for_profile(conn, profile_id):
            config = _wallet_config(wallet)
            sources.extend(_address_list_scripts(profile_id, wallet, config, active_counts))
        sources.extend(_inventory_scripts(conn, profile_id, active_counts))
        sources.extend(candidate_scripts)
        sources.extend(
            _descriptor_config_scripts(conn, profile_id, active_counts, candidate_scripts)
        )
    filtered = [
        source
        for source in sources
        if _include_source(
            source,
            candidate_wallet_ids=candidate_wallet_ids,
            include_deprecated=include_deprecated,
        )
    ]
    grouped: dict[tuple[str, str, str, str], list[SourceScript]] = {}
    for source in filtered:
        if not source.chain or not source.network:
            continue
        chain, network = _normalize_chain_network(source.chain, source.network)
        if not chain or not network:
            continue
        key = (source.profile_id, chain, network, source.script_pubkey)
        grouped.setdefault(key, []).append(source)
    return grouped, filtered


def build_profile_source_index(
    conn: sqlite3.Connection,
    profile_id: str,
) -> ProfileSourceIndex:
    """Build one immutable ownership index for a wallet-prefetch operation.

    Callers deliberately rebuild this index for the next sync operation, so a
    wallet config, output inventory, or freshness-checkpoint mutation cannot
    leave a stale cross-operation cache behind.
    """

    active_counts = _active_transaction_counts(conn, profile_id)
    sources: list[SourceScript] = []
    for wallet in _wallets_for_profile(conn, profile_id):
        config = _wallet_config(wallet)
        sources.extend(_address_list_scripts(profile_id, wallet, config, active_counts))
    sources.extend(_inventory_scripts(conn, profile_id, active_counts))
    sources.extend(_descriptor_config_scripts(conn, profile_id, active_counts, ()))
    return ProfileSourceIndex(profile_id=profile_id, sources=tuple(sources))


def detect_profile_source_overlaps(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    candidate_scripts: Sequence[SourceScript] | None = None,
    include_deprecated: bool = False,
    only_wallet_ids: set[str] | None = None,
) -> dict[str, Any]:
    grouped, filtered = _source_script_groups(
        conn,
        profile_id,
        candidate_scripts=candidate_scripts,
        include_deprecated=include_deprecated,
    )
    overlaps = []
    for key, group in grouped.items():
        wallet_ids = {source.wallet_id for source in group}
        if len(wallet_ids) < 2:
            continue
        if only_wallet_ids is not None and not (wallet_ids & only_wallet_ids):
            continue
        overlaps.append(_overlap_payload(key, group))
    overlaps.sort(key=lambda item: (item["chain"], item["network"], item["wallets"][0]["label"]))
    return {
        "overlap_count": len(overlaps),
        "overlaps": overlaps,
        "checked": {
            "scope": "finite_scripts_only",
            "source_count": len(filtered),
            "wallet_count": len({source.wallet_id for source in filtered}),
            "descriptor_global_overlap_proven": False,
        },
    }


def _target_script_pubkey(target: Mapping[str, Any]) -> str:
    return str(target.get("script_pubkey") or "").strip().lower()


def _address_list_repair_scripts(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, set[str]]:
    grouped, _filtered = _source_script_groups(conn, profile_id)
    scripts_by_wallet: dict[str, set[str]] = {}
    for key, group in grouped.items():
        wallet_ids = {source.wallet_id for source in group}
        if len(wallet_ids) < 2:
            continue
        overlap = _overlap_payload(key, group)
        recommended_id = str(overlap.get("recommended_canonical_wallet_id") or "")
        recommended = next(
            (
                wallet
                for wallet in overlap.get("wallets") or []
                if str(wallet.get("id") or "") == recommended_id
            ),
            None,
        )
        if not recommended or str(recommended.get("kind") or "") not in _CANONICAL_WALLET_KINDS:
            continue
        for source in group:
            if source.wallet_id == recommended_id or source.source != "address_list":
                continue
            scripts_by_wallet.setdefault(source.wallet_id, set()).add(source.script_pubkey)
    return scripts_by_wallet


def apply_address_list_overlap_repairs(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, Any]:
    """Trim address-list entries covered by canonical descriptor/xpub sources.

    This deliberately does not delete transactions. Callers that want to remove
    already-imported duplicate economic rows must do that through the audited
    transaction metadata path.
    """
    scripts_by_wallet = _address_list_repair_scripts(conn, profile_id)
    if not scripts_by_wallet:
        return {"wallets_updated": [], "addresses_removed": 0}
    rows = conn.execute(
        """
        SELECT id, label, config_json
        FROM wallets
        WHERE profile_id = ?
        """,
        (profile_id,),
    ).fetchall()
    wallets_updated: list[dict[str, Any]] = []
    removed_total = 0
    for row in rows:
        wallet_id = str(row["id"])
        scripts = scripts_by_wallet.get(wallet_id)
        if not scripts:
            continue
        config = _wallet_config(row)
        addresses = normalize_addresses(config.get("addresses"))
        kept: list[str] = []
        removed_addresses: list[str] = []
        removed = 0
        for address in addresses:
            script_pubkey = scriptpubkey_for_address_or_none(address)
            if script_pubkey and script_pubkey.lower() in scripts:
                removed += 1
                removed_addresses.append(address)
                continue
            kept.append(address)
        if removed == 0:
            continue
        config["addresses"] = kept
        deprecated = False
        if not kept:
            config["deprecated"] = True
            deprecated = True
        conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps(config, sort_keys=True), wallet_id),
        )
        script_placeholders = ", ".join("?" for _ in scripts)
        address_placeholders = ", ".join("?" for _ in removed_addresses)
        conn.execute(
            f"""
            DELETE FROM wallet_utxos
            WHERE profile_id = ?
              AND wallet_id = ?
              AND (
                lower(script_pubkey) IN ({script_placeholders})
                OR address IN ({address_placeholders})
              )
            """,
            (profile_id, wallet_id, *sorted(scripts), *removed_addresses),
        )
        conn.execute(
            """
            DELETE FROM freshness_source_states
            WHERE profile_id = ? AND source_key = ?
            """,
            (
                profile_id,
                core_freshness.source_key(core_freshness.SOURCE_ONCHAIN, wallet_id),
            ),
        )
        removed_total += removed
        wallets_updated.append(
            {
                "wallet_id": wallet_id,
                "wallet": str(row["label"] or wallet_id),
                "addresses_removed": removed,
                "remaining_addresses": len(kept),
                "deprecated": deprecated,
                "output_inventory_cleared": True,
                "refresh_checkpoint_reset": True,
            }
        )
    return {"wallets_updated": wallets_updated, "addresses_removed": removed_total}


def _overlap_scripts_by_wallet(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, set[str]]:
    grouped, _filtered = _source_script_groups(conn, profile_id)
    scripts_by_wallet: dict[str, set[str]] = {}
    for key, group in grouped.items():
        wallet_ids = {source.wallet_id for source in group}
        if len(wallet_ids) < 2:
            continue
        overlap = _overlap_payload(key, group)
        recommended_id = str(overlap.get("recommended_canonical_wallet_id") or "")
        if not recommended_id:
            continue
        for source in group:
            scripts_by_wallet.setdefault(source.wallet_id, set()).add(
                source.script_pubkey
            )
    return scripts_by_wallet


def _transaction_scripts_from_raw(raw_json: Any, direction: str) -> set[str]:
    raw = stored_tx_mapping(raw_json, allow_nested=True)
    if raw is None:
        return set()
    direction = str(direction or "").strip().lower()
    scripts: set[str] = set()
    if direction in {"", "inbound"}:
        for vout in raw.get("vout") or []:
            script = (
                normalized_script_hex(output_script(vout))
                if isinstance(vout, Mapping)
                else None
            )
            if script:
                scripts.add(script)
    if direction in {"", "outbound"}:
        for vin in raw.get("vin") or []:
            if not isinstance(vin, Mapping):
                continue
            script = normalized_script_hex(input_script(vin))
            if script:
                scripts.add(script)
    return scripts


def _row_has_overlap_script_evidence(
    row: sqlite3.Row,
    scripts_by_wallet: Mapping[str, set[str]],
) -> bool:
    scripts = scripts_by_wallet.get(str(row["wallet_id"]))
    if not scripts:
        return False
    raw_scripts = _transaction_scripts_from_raw(
        row["raw_json"],
        str(row["direction"] or ""),
    )
    return bool(raw_scripts & scripts)


def _filter_sync_targets(sync_state: Any, filtered_scripts: set[str]) -> Any:
    targets = list(getattr(sync_state, "targets", []) or [])
    kept_targets = [
        target
        for target in targets
        if _target_script_pubkey(target) not in filtered_scripts
    ]
    if len(kept_targets) == len(targets):
        return sync_state
    tracked_scripts = {}
    for key, target in dict(getattr(sync_state, "tracked_scripts", {}) or {}).items():
        key_script = str(key or "").strip().lower()
        target_script = _target_script_pubkey(target) if isinstance(target, Mapping) else ""
        if key_script in filtered_scripts or target_script in filtered_scripts:
            continue
        tracked_scripts[key] = target
    return replace(sync_state, targets=kept_targets, tracked_scripts=tracked_scripts)


def filter_sync_state_for_canonical_owner(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any] | sqlite3.Row,
    wallet: Mapping[str, Any] | sqlite3.Row,
    sync_state: Any,
    *,
    profile_index: ProfileSourceIndex | None = None,
) -> Any:
    """Remove sync targets whose script is already owned by a better source.

    The filter is deliberately script-level and runs before backend fetches.
    It never compares amounts, timestamps, or transaction counts, so address
    reuse and same-amount transactions cannot collapse distinct history rows.
    """
    profile_id = str(_row_get(profile, "id"))
    wallet_id = str(_row_get(wallet, "id"))
    candidate_scripts = scripts_from_sync_state(profile, wallet, sync_state)
    if not candidate_scripts:
        return sync_state
    try:
        grouped, _filtered = _source_script_groups(
            conn,
            profile_id,
            candidate_scripts=candidate_scripts,
            profile_index=profile_index,
        )
    except sqlite3.OperationalError as exc:
        if "no such table" in str(exc).lower():
            return sync_state
        raise
    filtered_scripts: set[str] = set()
    for key, group in grouped.items():
        wallet_ids = {source.wallet_id for source in group}
        if len(wallet_ids) < 2 or wallet_id not in wallet_ids:
            continue
        current_targets = [
            source
            for source in group
            if source.wallet_id == wallet_id and source.source == "sync_target"
        ]
        if not current_targets:
            continue
        overlap = _overlap_payload(key, group)
        if str(overlap.get("recommended_canonical_wallet_id") or "") == wallet_id:
            continue
        filtered_scripts.update(source.script_pubkey for source in current_targets)
    if not filtered_scripts:
        return sync_state
    return _filter_sync_targets(sync_state, filtered_scripts)


def duplicate_transaction_preview(
    conn: sqlite3.Connection,
    profile_id: str,
    overlaps: Sequence[Mapping[str, Any]],
    *,
    limit: int | None = 20,
) -> dict[str, Any]:
    wallet_ids: set[str] = set()
    canonical_ids: set[str] = set()
    for overlap in overlaps:
        canonical = str(overlap.get("recommended_canonical_wallet_id") or "")
        if canonical:
            canonical_ids.add(canonical)
        for wallet in overlap.get("wallets") or []:
            wallet_id = str(wallet.get("id") or "")
            if wallet_id:
                wallet_ids.add(wallet_id)
    if len(wallet_ids) < 2:
        return {"duplicate_groups": [], "recommended_exclusions": [], "limited": False}
    placeholders = ", ".join("?" for _ in wallet_ids)
    rows = conn.execute(
        f"""
        SELECT id, wallet_id, external_id, occurred_at, direction, asset, amount, fee, raw_json
        FROM transactions
        WHERE profile_id = ?
          AND excluded = 0
          AND wallet_id IN ({placeholders})
          AND external_id IS NOT NULL
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile_id, *sorted(wallet_ids)),
    ).fetchall()
    groups: dict[tuple[Any, ...], list[sqlite3.Row]] = {}
    scripts_by_wallet = _overlap_scripts_by_wallet(conn, profile_id)
    for row in rows:
        if not _row_has_overlap_script_evidence(row, scripts_by_wallet):
            continue
        # Group by the full economic fingerprint, not just the transaction id.
        # A single chain transaction can pay the canonical overlapped script and
        # also a distinct output that only the address-list wallet owns (a batch
        # payment). Because rows store the whole transaction as raw_json, such a
        # row still carries the overlap script (so the evidence guard above
        # passes), yet its net amount is larger than the canonical duplicate.
        # Keying on amount and fee keeps a batch row in its own group so it never
        # shares a group with the canonical row, is never auto-excluded, and its
        # distinct value survives into journals.
        key = (
            str(row["external_id"] or "").strip().lower(),
            row["direction"],
            row["asset"],
            int(row["amount"] or 0),
            int(row["fee"] or 0),
        )
        groups.setdefault(key, []).append(row)
    duplicate_groups = []
    recommended_exclusions = []
    duplicate_group_count = 0
    for key, group in groups.items():
        if len({row["wallet_id"] for row in group}) < 2:
            continue
        keep = next((row for row in group if row["wallet_id"] in canonical_ids), None)
        if keep is None:
            continue
        exclude = [row for row in group if row["wallet_id"] not in canonical_ids]
        if not exclude:
            continue
        duplicate_group_count += 1
        if limit is not None and len(duplicate_groups) >= limit:
            continue
        duplicate_groups.append(
            {
                "external_id": key[0],
                "occurred_at": min(str(row["occurred_at"] or "") for row in group),
                "direction": key[1],
                "asset": key[2],
                "transaction_count": len(group),
                "recommended_keep_transaction_id": keep["id"],
                "recommended_exclude_transaction_ids": [row["id"] for row in exclude],
            }
        )
        recommended_exclusions.extend(row["id"] for row in exclude)
    return {
        "duplicate_groups": duplicate_groups,
        "recommended_exclusions": recommended_exclusions,
        "limited": limit is not None and duplicate_group_count > limit,
        "repair_policy": (
            "Preview only. Exclude duplicate transaction rows through the audited "
            "metadata exclusion path after user review; do not delete wallets or "
            "cascade-delete transactions automatically."
        ),
    }


def _is_address_list_overlap(overlap: Mapping[str, Any]) -> bool:
    evidence = {str(item) for item in (overlap.get("evidence") or [])}
    if "address_list" in evidence:
        return True
    return any(
        str(wallet.get("kind") or "") == "address"
        for wallet in (overlap.get("wallets") or [])
        if isinstance(wallet, Mapping)
    )


def _hard_sync_overlaps(overlaps: Sequence[Mapping[str, Any]]) -> list[Mapping[str, Any]]:
    return [overlap for overlap in overlaps if not _is_address_list_overlap(overlap)]


def raise_for_sync_source_overlap(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any] | sqlite3.Row,
    wallet: Mapping[str, Any] | sqlite3.Row,
    sync_state: Any,
) -> None:
    profile_id = str(_row_get(profile, "id"))
    wallet_id = str(_row_get(wallet, "id"))
    candidate_scripts = scripts_from_sync_state(profile, wallet, sync_state)
    result = detect_profile_source_overlaps(
        conn,
        profile_id,
        candidate_scripts=candidate_scripts,
        only_wallet_ids={wallet_id},
    )
    hard_overlaps = _hard_sync_overlaps(result["overlaps"])
    if not hard_overlaps:
        return
    raise AppError(
        f"Wallet source overlap detected for {_row_get(wallet, 'label') or 'wallet'}",
        code="source_overlap",
        hint=(
            "Review overlapping wallet sources before refreshing. Prefer one "
            "canonical source and retire or trim duplicate address-list targets; "
            "Kassiber only proves overlap within the checked finite scripts."
        ),
        details={
            "overlap_count": len(hard_overlaps),
            "overlaps": hard_overlaps,
            "checked": result["checked"],
        },
        retryable=False,
    )


__all__ = [
    "ProfileSourceIndex",
    "SourceScript",
    "apply_address_list_overlap_repairs",
    "build_profile_source_index",
    "detect_profile_source_overlaps",
    "duplicate_transaction_preview",
    "filter_sync_state_for_canonical_owner",
    "raise_for_sync_source_overlap",
    "scripts_from_sync_state",
]
