"""Stable, non-secret identities for request-scoped chain observers."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ...errors import AppError
from ...util import normalize_chain_value, normalize_network_value, str_or_none


IDENTITY_VERSION = 1


@dataclass(frozen=True, slots=True)
class ObserverIdentity:
    """Persistent identity for one observer below a logical wallet.

    ``source_key`` is deliberately structural (script family or Samourai
    source), never descriptor/xpub material. The hashed id is therefore stable
    across process restarts without creating another secret-derived handle.
    """

    id: str
    workspace_id: str
    profile_id: str
    logical_wallet_id: str
    source_wallet_id: str
    source_key: str
    observer_kind: str
    chain: str
    network: str
    branch_keys: tuple[str, ...]


def observer_instance_id(
    logical_wallet_id: str,
    source_wallet_id: str,
    source_key: str,
) -> str:
    material = "\0".join(
        (
            f"chain-observer-identity-v{IDENTITY_VERSION}",
            logical_wallet_id,
            source_wallet_id,
            source_key,
        )
    )
    return hashlib.sha256(material.encode("utf-8")).hexdigest()


def _value(row: Mapping[str, Any], key: str, default: Any = None) -> Any:
    try:
        if hasattr(row, "keys") and key not in row.keys():
            return default
        return row[key]
    except (KeyError, IndexError, TypeError):
        return default


def _wallet_config(wallet: Mapping[str, Any]) -> dict[str, Any]:
    config = _value(wallet, "config")
    if isinstance(config, Mapping):
        return dict(config)
    raw = _value(wallet, "config_json", "{}")
    if not isinstance(raw, str):
        return {}
    try:
        parsed = json.loads(raw or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _normal_chain_network(config: Mapping[str, Any]) -> tuple[str, str]:
    chain = normalize_chain_value(config.get("chain") or "bitcoin")
    network = normalize_network_value(chain, config.get("network"))
    return chain, network


def _identity(
    wallet: Mapping[str, Any],
    *,
    logical_wallet_id: str,
    source_key: str,
    observer_kind: str,
    chain: str,
    network: str,
    branch_keys: Sequence[str],
) -> ObserverIdentity:
    source_wallet_id = str(_value(wallet, "id") or "").strip()
    workspace_id = str(_value(wallet, "workspace_id") or "").strip()
    profile_id = str(_value(wallet, "profile_id") or "").strip()
    if not source_wallet_id or not workspace_id or not profile_id:
        raise AppError(
            "Observer identity requires a persisted wallet scope",
            code="observer_identity_invalid",
            retryable=False,
        )
    return ObserverIdentity(
        id=observer_instance_id(logical_wallet_id, source_wallet_id, source_key),
        workspace_id=workspace_id,
        profile_id=profile_id,
        logical_wallet_id=logical_wallet_id,
        source_wallet_id=source_wallet_id,
        source_key=source_key,
        observer_kind=observer_kind,
        chain=chain,
        network=network,
        branch_keys=tuple(dict.fromkeys(str(value) for value in branch_keys)),
    )


def identities_for_wallet(
    wallet: Mapping[str, Any],
    *,
    observer_kind: str | None = None,
) -> tuple[ObserverIdentity, ...]:
    """Return deterministic observer instances for one stored wallet row."""

    config = _wallet_config(wallet)
    wallet_id = str(_value(wallet, "id") or "").strip()
    if not wallet_id:
        raise AppError(
            "Observer identity requires a persisted wallet id",
            code="observer_identity_invalid",
            retryable=False,
        )
    chain, network = _normal_chain_network(config)
    resolved_observer_kind = (
        str_or_none(observer_kind)
        or str_or_none(config.get("observer_kind"))
        or "compatibility"
    )
    samourai = config.get("samourai")
    if isinstance(samourai, Mapping):
        role = str_or_none(samourai.get("role"))
        if role == "parent":
            return ()
        if role == "child":
            logical_wallet_id = str_or_none(samourai.get("parent_wallet_id")) or wallet_id
            section = str_or_none(samourai.get("section")) or "source"
            script_type = str_or_none(samourai.get("script_type")) or "descriptor"
            root_path = str_or_none(samourai.get("root_path")) or "account"
            source_key = f"samourai:{section}:{script_type}:{root_path}"
            branches = ("receive", "change") if config.get("change_descriptor") else ("receive",)
            return (
                _identity(
                    wallet,
                    logical_wallet_id=logical_wallet_id,
                    source_key=source_key,
                    observer_kind=resolved_observer_kind,
                    chain=chain,
                    network=network,
                    branch_keys=branches,
                ),
            )

    script_types = sorted(
        {
            str(value).strip().lower()
            for value in (config.get("script_types") or ())
            if str(value).strip()
        }
    )
    if str_or_none(config.get("xpub")) and script_types:
        return tuple(
            _identity(
                wallet,
                logical_wallet_id=wallet_id,
                source_key=f"xpub:{script_type}",
                observer_kind=resolved_observer_kind,
                chain=chain,
                network=network,
                branch_keys=("receive", "change"),
            )
            for script_type in script_types
        )
    if str_or_none(config.get("descriptor")):
        # Use the executable plan so a receive-only ranged descriptor promoted
        # to canonical <0;1> multipath receives stable change coverage too.
        from ...wallet_descriptors import load_descriptor_plan

        plan = load_descriptor_plan(config)
        branches = tuple(
            "change" if "change" in str(branch.branch_label).lower() else "receive"
            for branch in (plan.branches if plan is not None else ())
        ) or ("receive",)
        return (
            _identity(
                wallet,
                logical_wallet_id=wallet_id,
                source_key="descriptor:default",
                observer_kind=resolved_observer_kind,
                chain=chain,
                network=network,
                branch_keys=branches,
            ),
        )
    return ()


def identities_for_wallets(
    wallets: Sequence[Mapping[str, Any]],
    *,
    observer_kind: str | None = None,
) -> tuple[ObserverIdentity, ...]:
    identities = [
        identity
        for wallet in wallets
        for identity in identities_for_wallet(wallet, observer_kind=observer_kind)
    ]
    return tuple(sorted(identities, key=lambda identity: identity.id))


__all__ = [
    "IDENTITY_VERSION",
    "ObserverIdentity",
    "identities_for_wallet",
    "identities_for_wallets",
    "observer_instance_id",
]
