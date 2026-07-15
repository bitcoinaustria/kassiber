"""Durable policy epochs above disposable chain-observer state.

An observer can prove only that one imported policy source was scanned through
an exclusive derivation boundary.  It cannot prove that the profile owner has
no other wallets.  This module deliberately exposes technical coverage facts,
never a global ownership-completeness predicate.
"""

from __future__ import annotations

import json
import sqlite3
import uuid
from typing import Any, Mapping, Sequence

from ..time_utils import now_iso
from ..util import normalize_chain_value, normalize_network_value


_PRIVATE_MATERIAL_FIELDS = frozenset(
    {
        "descriptor",
        "change_descriptor",
        "xpub",
        "script_types",
        "addresses",
        "blinding_key",
        "chain",
        "network",
        "samourai",
        "ownership_scan_to_index",
        "gap_limit",
        "synthesize_change",
    }
)
_PUBLIC_SCRIPT_FAMILIES = frozenset({"p2pkh", "p2sh-p2wpkh", "p2wpkh", "p2tr"})
_PUBLIC_SAMOURAI_SECTIONS = frozenset(
    {"deposit", "badbank", "premix", "postmix", "ricochet"}
)
_PUBLIC_BRANCHES = ("receive", "change")


def _value(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    if hasattr(row, "keys") and key not in row.keys():
        return default
    if hasattr(row, "get"):
        return row.get(key, default)
    return row[key]


def _public_policy_source_name(source_key: Any) -> str:
    """Reduce a private observer source key to an allowlisted structural class."""

    parts = str(source_key or "").split(":")
    if parts[0] == "descriptor":
        return "descriptor-policy"
    if parts[0] == "xpub":
        script_family = parts[1] if len(parts) > 1 else "unknown"
        if script_family in _PUBLIC_SCRIPT_FAMILIES:
            return f"extended-key:{script_family}"
        return "extended-key-policy"
    if parts[0] == "samourai":
        section = parts[1] if len(parts) > 1 else "source"
        if section in _PUBLIC_SAMOURAI_SECTIONS:
            return f"samourai:{section}"
        return "samourai:source"
    return "imported-policy"


def _public_observer_name(observer_kind: Any) -> str:
    """Expose only built-in observer classes, never configured source names."""

    value = str(observer_kind or "").lower()
    if value in {"bdk", "lwk", "bitcoinrpc"}:
        return value
    if value.startswith("compatibility"):
        return "compatibility"
    return "observer"


def _empty_coverage_branch(branch: str) -> dict[str, Any]:
    return {
        "branch": branch,
        "scanned_to_exclusive": None,
        "highest_used": None,
        "observed_at": None,
    }


def _declared_coverage_branches(raw: Any) -> list[dict[str, Any]]:
    try:
        declared = json.loads(raw or "[]")
    except (TypeError, ValueError, json.JSONDecodeError):
        return []
    if not isinstance(declared, list):
        return []
    return [
        _empty_coverage_branch(branch)
        for branch in _PUBLIC_BRANCHES
        if branch in declared
    ]


def private_policy_material(config: Mapping[str, Any] | None) -> dict[str, Any]:
    """Return only encrypted material needed to recognize a policy later."""

    if not isinstance(config, Mapping):
        return {}
    return {
        key: config[key]
        for key in sorted(_PRIVATE_MATERIAL_FIELDS)
        if config.get(key) not in (None, "", [])
    }


def _wallet_config(wallet: Mapping[str, Any]) -> dict[str, Any]:
    raw = _value(wallet, "config_json", "{}")
    if isinstance(raw, Mapping):
        return dict(raw)
    try:
        parsed = json.loads(str(raw or "{}"))
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _scope(
    config: Mapping[str, Any],
    *,
    chain: str | None = None,
    network: str | None = None,
) -> tuple[str, str]:
    normalized_chain = normalize_chain_value(chain or config.get("chain") or "bitcoin")
    normalized_network = normalize_network_value(
        normalized_chain,
        network or config.get("network"),
    )
    return normalized_chain, normalized_network


def ensure_active_wallet_epoch(
    conn: sqlite3.Connection,
    wallet: Mapping[str, Any],
    *,
    material: Mapping[str, Any] | None = None,
    chain: str | None = None,
    network: str | None = None,
) -> str:
    """Return the random active epoch id, creating it without committing."""

    wallet_id = str(_value(wallet, "id") or "")
    row = conn.execute(
        """
        SELECT id FROM wallet_policy_epochs
        WHERE wallet_id = ? AND status = 'active'
        """,
        (wallet_id,),
    ).fetchone()
    if row is not None:
        return str(row["id"])
    config = _wallet_config(wallet)
    epoch_material = private_policy_material(material if material is not None else config)
    epoch_chain, epoch_network = _scope(config, chain=chain, network=network)
    epoch_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO wallet_policy_epochs(
            id, workspace_id, profile_id, wallet_id, chain, network, status,
            private_material_json, created_at, retired_at
        ) VALUES(?, ?, ?, ?, ?, ?, 'active', ?, ?, NULL)
        """,
        (
            epoch_id,
            str(_value(wallet, "workspace_id") or ""),
            str(_value(wallet, "profile_id") or ""),
            wallet_id,
            epoch_chain,
            epoch_network,
            json.dumps(epoch_material, sort_keys=True),
            now_iso(),
        ),
    )
    return epoch_id


def roll_wallet_policy_epoch(
    conn: sqlite3.Connection,
    wallet: Mapping[str, Any],
    old_material: Mapping[str, Any],
    new_material: Mapping[str, Any],
) -> tuple[str, str]:
    """Retire the old policy and create the replacement atomically."""

    old_epoch_id = ensure_active_wallet_epoch(conn, wallet, material=old_material)
    retired_at = now_iso()
    conn.execute(
        """
        UPDATE wallet_policy_epochs
        SET status = 'retired', retired_at = ?
        WHERE id = ? AND status = 'active'
        """,
        (retired_at, old_epoch_id),
    )
    replacement = {key: wallet[key] for key in wallet.keys()}
    replacement["config_json"] = json.dumps(dict(new_material), sort_keys=True)
    new_epoch_id = ensure_active_wallet_epoch(
        conn,
        replacement,
        material=new_material,
    )
    return old_epoch_id, new_epoch_id


def record_observer_policy_coverage(
    conn: sqlite3.Connection,
    identity: Any,
    coverage: Sequence[Any],
) -> str:
    """Project private observer coverage into the active durable epoch."""

    wallet = conn.execute(
        "SELECT * FROM wallets WHERE id = ?",
        (str(identity.source_wallet_id),),
    ).fetchone()
    if wallet is None:
        raise ValueError("observer policy source wallet does not exist")
    epoch_id = ensure_active_wallet_epoch(
        conn,
        wallet,
        chain=str(identity.chain),
        network=str(identity.network),
    )
    source = conn.execute(
        """
        SELECT id FROM wallet_policy_sources
        WHERE epoch_id = ? AND source_wallet_id = ? AND source_key = ?
        """,
        (epoch_id, str(identity.source_wallet_id), str(identity.source_key)),
    ).fetchone()
    observed_at = now_iso()
    branch_keys = tuple(dict.fromkeys(str(item) for item in identity.branch_keys))
    if source is None:
        source_id = str(uuid.uuid4())
        conn.execute(
            """
            INSERT INTO wallet_policy_sources(
                id, epoch_id, source_wallet_id, source_key, observer_kind,
                branch_keys_json, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                source_id,
                epoch_id,
                str(identity.source_wallet_id),
                str(identity.source_key),
                str(identity.observer_kind),
                json.dumps(branch_keys),
                observed_at,
                observed_at,
            ),
        )
    else:
        source_id = str(source["id"])
        conn.execute(
            """
            UPDATE wallet_policy_sources
            SET observer_kind = ?, branch_keys_json = ?, updated_at = ?
            WHERE id = ?
            """,
            (
                str(identity.observer_kind),
                json.dumps(branch_keys),
                observed_at,
                source_id,
            ),
        )
    for point in coverage:
        conn.execute(
            """
            INSERT INTO wallet_policy_coverage_witnesses(
                source_id, branch_key, scanned_to_exclusive, highest_used,
                observer_kind, observed_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_id, branch_key) DO UPDATE SET
                scanned_to_exclusive = excluded.scanned_to_exclusive,
                highest_used = excluded.highest_used,
                observer_kind = excluded.observer_kind,
                observed_at = excluded.observed_at
            """,
            (
                source_id,
                str(point.branch_key),
                int(point.scanned_to),
                point.highest_used,
                str(identity.observer_kind),
                observed_at,
            ),
        )
    return epoch_id


def retired_policy_materials(
    conn: sqlite3.Connection,
    wallet_id: str,
) -> tuple[Mapping[str, Any], ...]:
    """Load private retired material for offline historical recognition."""

    rows = conn.execute(
        """
        SELECT private_material_json
        FROM wallet_policy_epochs
        WHERE wallet_id = ? AND status = 'retired'
        ORDER BY created_at ASC, id ASC
        """,
        (wallet_id,),
    ).fetchall()
    output: list[Mapping[str, Any]] = []
    for row in rows:
        try:
            value = json.loads(row["private_material_json"] or "{}")
        except (TypeError, ValueError, json.JSONDecodeError):
            continue
        if isinstance(value, Mapping) and value:
            output.append(dict(value))
    return tuple(output)


def technical_coverage_snapshot(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, Any]:
    """Return redacted scan diagnostics without an ownership-complete claim."""

    rows = conn.execute(
        """
        SELECT
            epoch.id AS epoch_id,
            epoch.wallet_id,
            wallet.label AS wallet_label,
            epoch.chain,
            epoch.network,
            epoch.status,
            epoch.created_at AS epoch_created_at,
            epoch.retired_at,
            source.id AS source_id,
            source.source_key,
            source.observer_kind,
            source.branch_keys_json,
            coverage.branch_key,
            coverage.scanned_to_exclusive,
            coverage.highest_used,
            coverage.observed_at
        FROM wallet_policy_epochs epoch
        JOIN wallets wallet ON wallet.id = epoch.wallet_id
        LEFT JOIN wallet_policy_sources source ON source.epoch_id = epoch.id
        LEFT JOIN wallet_policy_coverage_witnesses coverage ON coverage.source_id = source.id
        WHERE epoch.profile_id = ?
        ORDER BY wallet.label, epoch.created_at, epoch.id, source.source_key, coverage.branch_key
        """,
        (profile_id,),
    ).fetchall()

    wallets_by_id: dict[str, dict[str, Any]] = {}
    epochs_by_id: dict[str, dict[str, Any]] = {}
    sources_by_id: dict[str, dict[str, Any]] = {}
    covered_branch_count = 0
    for row in rows:
        wallet_label = str(row["wallet_label"] or "Wallet")
        wallet = wallets_by_id.setdefault(
            str(row["wallet_id"]),
            {"wallet_label": wallet_label, "epochs": []},
        )
        epoch_id = str(row["epoch_id"])
        epoch = epochs_by_id.get(epoch_id)
        if epoch is None:
            epoch = {
                "epoch_id": epoch_id,
                "status": str(row["status"]),
                "chain": str(row["chain"]),
                "network": str(row["network"]),
                "created_at": row["epoch_created_at"],
                "retired_at": row["retired_at"],
                "sources": [],
            }
            epochs_by_id[epoch_id] = epoch
            wallet["epochs"].append(epoch)

        if row["source_id"] is None:
            continue
        source_id = str(row["source_id"])
        source = sources_by_id.get(source_id)
        if source is None:
            source = {
                "source": _public_policy_source_name(row["source_key"]),
                "observer_kind": _public_observer_name(row["observer_kind"]),
                "branches": _declared_coverage_branches(row["branch_keys_json"]),
            }
            sources_by_id[source_id] = source
            epoch["sources"].append(source)

        branch_key = row["branch_key"]
        if branch_key not in _PUBLIC_BRANCHES:
            continue
        branch = next(
            (item for item in source["branches"] if item["branch"] == branch_key),
            None,
        )
        if branch is None:
            branch = _empty_coverage_branch(str(branch_key))
            source["branches"].append(branch)
        if row["scanned_to_exclusive"] is not None:
            branch.update(
                {
                    "scanned_to_exclusive": int(row["scanned_to_exclusive"]),
                    "highest_used": (
                        int(row["highest_used"])
                        if row["highest_used"] is not None
                        else None
                    ),
                    "observed_at": row["observed_at"],
                }
            )
            covered_branch_count += 1

    wallets = list(wallets_by_id.values())
    epoch_count = len(epochs_by_id)
    active_epoch_count = sum(
        epoch["status"] == "active" for epoch in epochs_by_id.values()
    )
    return {
        "schema_version": 1,
        "scope": "imported_policy_technical_coverage",
        "ownership_universe_known": False,
        "coverage_can_clear_custody_gaps": False,
        "summary": {
            "wallet_count": len(wallets),
            "epoch_count": epoch_count,
            "active_epoch_count": active_epoch_count,
            "retired_epoch_count": epoch_count - active_epoch_count,
            "source_count": len(sources_by_id),
            "covered_branch_count": covered_branch_count,
        },
        "wallets": wallets,
    }


__all__ = [
    "ensure_active_wallet_epoch",
    "private_policy_material",
    "record_observer_policy_coverage",
    "retired_policy_materials",
    "roll_wallet_policy_epoch",
    "technical_coverage_snapshot",
]
