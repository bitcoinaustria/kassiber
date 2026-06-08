from __future__ import annotations

import json
import re
import sqlite3
import uuid
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from ..errors import AppError
from ..time_utils import now_iso
from ..util import normalize_network_value, str_or_none
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    MAX_DESCRIPTOR_GAP_LIMIT,
    derive_descriptor_target,
    load_descriptor_plan,
)
from . import wallets as core_wallets
from .repo import fetch_wallet_with_account, resolve_account, resolve_scope


SAMOURAI_CONFIG_KEY = "samourai"
SAMOURAI_PARENT_KIND = "samourai"
SAMOURAI_CHILD_KIND = "descriptor"
SAMOURAI_POSTMIX_MIN_GAP_LIMIT = DEFAULT_DESCRIPTOR_GAP_LIMIT
SAMOURAI_GROUP_SECTIONS = (
    "deposit",
    "badbank",
    "premix",
    "postmix",
    "ricochet",
)
SAMOURAI_PRIVACY_SECTIONS = {"badbank", "premix", "postmix"}
SAMOURAI_SAFE_METADATA_FIELDS = {
    "role",
    "group_id",
    "group_label",
    "parent_wallet_id",
    "source",
    "section",
    "script_type",
    "root_path",
    "gap_limit",
    "privacy_boundary",
    "whirlpool",
    "toxic_change",
    "minimum_mix_count",
    "mix_count",
    "mix_count_confidence",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "round_txid",
    "round_txids",
    "tx0_role",
    "whirlpool_event",
    "privacy_event",
    "exit_kind",
    "ricochet_hops",
    "watch_only",
    "bip47",
    "paynym",
    "scanned_without_explicit_descriptor",
    "sections",
}
SAMOURAI_EXPLICIT_PROVENANCE_FIELDS = {
    "minimum_mix_count",
    "mix_count",
    "mix_count_confidence",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "round_txid",
    "round_txids",
    "tx0_role",
    "whirlpool_event",
    "privacy_event",
    "exit_kind",
    "ricochet_hops",
}
SAMOURAI_ENUM_VALUES = {
    "mix_count_confidence": {"minimum", "exact", "estimated", "unknown"},
    "tx0_role": {"deposit", "premix", "badbank", "fee"},
    "whirlpool_event": {
        "tx0",
        "premix_pending",
        "first_mix",
        "remix",
        "mix_to_wallet",
        "external_spend",
    },
    "privacy_event": {
        "coinjoin",
        "payjoin",
        "tx0",
        "first_mix",
        "remix",
        "ricochet",
        "exit",
    },
    "exit_kind": {"cold_storage", "external_spend", "ricochet", "toxic_change_spend"},
}
SAMOURAI_NON_NEGATIVE_INT_FIELDS = {
    "gap_limit",
    "minimum_mix_count",
    "mix_count",
    "target_mix_count",
    "pool_denomination_sat",
    "coordinator_fee_sat",
    "miner_fee_sat",
    "ricochet_hops",
}


@dataclass(frozen=True)
class SamouraiAccountTemplate:
    section: str
    label: str
    purpose: int
    account: int
    script_type: str
    receive_change: bool = True
    minimum_gap_limit: int = DEFAULT_DESCRIPTOR_GAP_LIMIT
    whirlpool: bool = False
    toxic_change: bool = False
    paynym: bool = False
    minimum_mix_count: int | None = None
    mix_count_confidence: str | None = None

    @property
    def path(self) -> str:
        return f"m/{self.purpose}'/{{coin_type}}'/{self.account}'"


SAMOURAI_ACCOUNT_TEMPLATES: tuple[SamouraiAccountTemplate, ...] = (
    SamouraiAccountTemplate("deposit", "Deposit Legacy", 44, 0, "p2pkh"),
    SamouraiAccountTemplate("deposit", "Deposit Nested SegWit", 49, 0, "p2sh-p2wpkh"),
    SamouraiAccountTemplate("deposit", "Deposit Native SegWit", 84, 0, "p2wpkh"),
    SamouraiAccountTemplate(
        "deposit",
        "Deposit PayNym",
        47,
        0,
        "p2pkh",
        paynym=True,
    ),
    SamouraiAccountTemplate(
        "badbank",
        "Badbank / Toxic Change",
        84,
        2_147_483_644,
        "p2wpkh",
        whirlpool=True,
        toxic_change=True,
    ),
    SamouraiAccountTemplate(
        "premix",
        "Premix",
        84,
        2_147_483_645,
        "p2wpkh",
        whirlpool=True,
    ),
    SamouraiAccountTemplate(
        "postmix",
        "Postmix",
        84,
        2_147_483_646,
        "p2wpkh",
        minimum_gap_limit=SAMOURAI_POSTMIX_MIN_GAP_LIMIT,
        whirlpool=True,
        minimum_mix_count=1,
        mix_count_confidence="minimum",
    ),
    SamouraiAccountTemplate("ricochet", "Ricochet Legacy", 44, 2_147_483_647, "p2pkh"),
    SamouraiAccountTemplate(
        "ricochet",
        "Ricochet Nested SegWit",
        49,
        2_147_483_647,
        "p2sh-p2wpkh",
    ),
    SamouraiAccountTemplate(
        "ricochet",
        "Ricochet Native SegWit",
        84,
        2_147_483_647,
        "p2wpkh",
    ),
)


def _normalize_import_network(network: str | None, declared_network: str | None = None) -> str:
    try:
        normalized_declared = (
            normalize_network_value("bitcoin", declared_network)
            if declared_network
            else None
        )
        normalized = normalize_network_value(
            "bitcoin",
            network or normalized_declared or "main",
        )
    except AppError:
        raise
    if normalized_declared and normalized != normalized_declared:
        raise AppError(
            "Samourai source-set network does not match --network",
            code="validation",
            hint=f"Use --network {normalized_declared} or choose a matching descriptor/xpub source set.",
            details={"source_set_network": normalized_declared, "requested_network": normalized},
            retryable=False,
        )
    return normalized


def _coin_type_for_network(network: str) -> int:
    return 0 if network == "main" else 1


def _template_path(template: SamouraiAccountTemplate, network: str) -> str:
    return template.path.format(coin_type=_coin_type_for_network(network))


def _descriptor_for_xpub(
    script_type: str,
    fingerprint: str,
    path: str,
    xpub: str,
    branch: int,
) -> str:
    origin = f"[{fingerprint}/{path[2:]}]{xpub}/{branch}/*"
    if script_type == "p2pkh":
        return f"pkh({origin})"
    if script_type == "p2sh-p2wpkh":
        return f"sh(wpkh({origin}))"
    if script_type == "p2wpkh":
        return f"wpkh({origin})"
    raise AppError(
        f"Unsupported Samourai script type '{script_type}'",
        code="validation",
        hint="Supported script types are p2pkh, p2sh-p2wpkh, and p2wpkh.",
        retryable=False,
    )


def _gap_limit_for(template: SamouraiAccountTemplate, requested_gap_limit: int | None) -> int:
    gap_limit = requested_gap_limit or DEFAULT_DESCRIPTOR_GAP_LIMIT
    if gap_limit <= 0:
        raise AppError("Descriptor gap limit must be positive", code="validation")
    if gap_limit > MAX_DESCRIPTOR_GAP_LIMIT:
        raise AppError(
            f"Descriptor gap limit must be {MAX_DESCRIPTOR_GAP_LIMIT} or lower",
            code="validation",
        )
    return max(gap_limit, template.minimum_gap_limit)


def _source_from_account_xpub(
    template: SamouraiAccountTemplate,
    *,
    network: str,
    account_xpub: str,
    fingerprint: str,
    root_path: str,
    gap_limit: int,
) -> dict[str, Any]:
    descriptor = _descriptor_for_xpub(template.script_type, fingerprint, root_path, account_xpub, 0)
    change_descriptor = (
        _descriptor_for_xpub(template.script_type, fingerprint, root_path, account_xpub, 1)
        if template.receive_change
        else None
    )
    config: dict[str, Any] = {
        "chain": "bitcoin",
        "network": network,
        "descriptor": descriptor,
        "gap_limit": gap_limit,
        SAMOURAI_CONFIG_KEY: _safe_template_metadata(template, root_path, gap_limit),
    }
    if change_descriptor:
        config["change_descriptor"] = change_descriptor
    _validate_descriptor_config(config)
    return {
        "section": template.section,
        "label": template.label,
        "config": config,
    }


def _safe_template_metadata(
    template: SamouraiAccountTemplate,
    root_path: str,
    gap_limit: int,
) -> dict[str, Any]:
    metadata: dict[str, Any] = {
        "role": "child",
        "section": template.section,
        "script_type": template.script_type,
        "root_path": root_path,
        "gap_limit": gap_limit,
        "privacy_boundary": template.section in SAMOURAI_PRIVACY_SECTIONS,
    }
    if template.whirlpool:
        metadata["whirlpool"] = True
    if template.toxic_change:
        metadata["toxic_change"] = True
    if template.paynym:
        metadata["paynym"] = True
        metadata["scanned_without_explicit_descriptor"] = False
    if template.minimum_mix_count is not None:
        metadata["minimum_mix_count"] = template.minimum_mix_count
        metadata["mix_count_confidence"] = template.mix_count_confidence or "minimum"
    return metadata


def _safe_samourai_metadata(raw_metadata: Any, *, fields: set[str]) -> dict[str, Any]:
    if not isinstance(raw_metadata, dict):
        return {}
    safe: dict[str, Any] = {}
    for key, value in raw_metadata.items():
        if key not in fields or key not in SAMOURAI_SAFE_METADATA_FIELDS:
            continue
        normalized = _safe_samourai_metadata_value(key, value)
        if normalized is not None:
            safe[key] = normalized
    return safe


def _safe_samourai_metadata_value(key: str, value: Any) -> Any:
    if key in SAMOURAI_NON_NEGATIVE_INT_FIELDS:
        try:
            normalized = int(value)
        except (TypeError, ValueError):
            return None
        return normalized if normalized >= 0 else None
    if key in {"privacy_boundary", "whirlpool", "toxic_change", "watch_only", "paynym"}:
        return bool(value)
    if key == "round_txids":
        if not isinstance(value, list):
            return None
        txids = [_normalize_txid_or_none(item) for item in value]
        return [txid for txid in txids if txid is not None] or None
    if key == "round_txid":
        return _normalize_txid_or_none(value)
    if key == "sections":
        if not isinstance(value, list):
            return None
        sections = [
            str(item).strip().lower()
            for item in value
            if str(item).strip().lower() in SAMOURAI_GROUP_SECTIONS
        ]
        return sections or None
    if key in SAMOURAI_ENUM_VALUES:
        normalized = str(value or "").strip().lower()
        return normalized if normalized in SAMOURAI_ENUM_VALUES[key] else None
    if isinstance(value, str):
        normalized = value.strip()
        return normalized[:128] if normalized else None
    return value if value is None or isinstance(value, (int, bool)) else None


def _normalize_txid_or_none(value: Any) -> str | None:
    normalized = str(value or "").strip().lower()
    if len(normalized) != 64:
        return None
    try:
        bytes.fromhex(normalized)
    except ValueError:
        return None
    return normalized


def _safe_explicit_samourai_metadata(raw_source: dict[str, Any]) -> dict[str, Any]:
    raw_metadata: dict[str, Any] = {}
    inline = raw_source.get(SAMOURAI_CONFIG_KEY)
    if isinstance(inline, dict):
        raw_metadata.update(inline)
    for key in SAMOURAI_EXPLICIT_PROVENANCE_FIELDS:
        if key in raw_source:
            raw_metadata[key] = raw_source[key]
    return _safe_samourai_metadata(
        raw_metadata,
        fields=SAMOURAI_EXPLICIT_PROVENANCE_FIELDS,
    )


def _validate_descriptor_config(config: dict[str, Any]) -> None:
    try:
        load_descriptor_plan(config)
    except ValueError as exc:
        raise AppError(
            "Samourai descriptor material is malformed",
            code="validation",
            hint="Check the descriptor or xpub set before importing.",
            retryable=False,
        ) from exc


def _validate_descriptor_origin_matches_template(
    config: dict[str, Any],
    *,
    template: SamouraiAccountTemplate,
    root_path: str,
) -> None:
    try:
        plan = load_descriptor_plan(config)
        if plan is None:
            raise ValueError("missing descriptor")
        expected_branches = {0, 1} if template.receive_change else {0}
        actual_branches = {branch.branch_index for branch in plan.branches}
        missing_branches = sorted(expected_branches - actual_branches)
        if missing_branches:
            raise AppError(
                "Samourai descriptor source is missing a required receive/change branch",
                code="validation",
                hint=(
                    "Include both descriptor and change_descriptor, or a ranged descriptor "
                    "covering branches 0 and 1."
                ),
                details={
                    "section": template.section,
                    "root_path": root_path,
                    "missing_branches": missing_branches,
                },
                retryable=False,
            )
        for branch in plan.branches:
            target = derive_descriptor_target(plan, branch.branch_index, 0)
            expected = f"{root_path}/{branch.branch_index}/0"
            derivation_paths = set(target.derivation_paths)
            if derivation_paths != {expected}:
                raise ValueError(
                    f"expected descriptor origin {expected}, got {sorted(derivation_paths)}"
                )
    except AppError:
        raise
    except ValueError as exc:
        raise AppError(
            "Samourai descriptor origin does not match the declared account path",
            code="validation",
            hint="Use descriptors with key origins that match the declared Samourai section/root_path.",
            details={"expected_root_path": root_path},
            retryable=False,
        ) from exc


def _template_for(
    section: str,
    script_type: str,
    root_path: str | None = None,
) -> SamouraiAccountTemplate:
    normalized_section = str(section or "").strip().lower().replace("_", "-")
    normalized_script = str(script_type or "").strip().lower()
    root = str(root_path or "")
    candidates = [
        template
        for template in SAMOURAI_ACCOUNT_TEMPLATES
        if template.section == normalized_section and template.script_type == normalized_script
    ]
    if root:
        for template in candidates:
            if _root_matches_template(root, template):
                return template
    if candidates:
        return candidates[0]
    raise AppError(
        "Explicit Samourai source has an unsupported section/script type",
        code="validation",
        hint="Use Deposit, Badbank, Premix, Postmix, or Ricochet with p2pkh, p2sh-p2wpkh, or p2wpkh.",
        retryable=False,
    )


def _root_matches_template(root_path: str, template: SamouraiAccountTemplate) -> bool:
    pattern = rf"^m/{template.purpose}'/[01]'/{template.account}'$"
    return re.fullmatch(pattern, str(root_path or "").strip()) is not None


def _validate_template_root_path(
    template: SamouraiAccountTemplate,
    root_path: str | None,
    network: str,
) -> str:
    expected = _template_path(template, network)
    normalized = str(root_path or expected).strip()
    if normalized != expected:
        raise AppError(
            "Explicit Samourai source uses a path outside the supported account map",
            code="validation",
            hint=f"Use {expected} for {template.label} on {network}.",
            details={"expected_root_path": expected, "provided_root_path": normalized},
            retryable=False,
        )
    return normalized


def load_samourai_source_set(
    path: str,
    *,
    network: str,
    gap_limit: int | None = None,
) -> list[dict[str, Any]]:
    try:
        raw_text = Path(path).expanduser().read_text(encoding="utf-8")
    except OSError as exc:
        raise AppError(
            "Could not read Samourai descriptor/xpub set",
            code="not_found",
            hint="Choose a readable local JSON file.",
            details={"path": path},
            retryable=False,
        ) from exc
    try:
        data = json.loads(raw_text)
    except json.JSONDecodeError as exc:
        raise AppError(
            "Samourai descriptor/xpub set is not valid JSON",
            code="validation",
            hint="Choose a JSON file with children, sources, or xpubs entries.",
            retryable=False,
        ) from exc
    return load_samourai_source_set_payload(data, network=network, gap_limit=gap_limit)


def load_samourai_source_set_payload(
    data: Any,
    *,
    network: str,
    gap_limit: int | None = None,
) -> list[dict[str, Any]]:
    """Normalize an inline Samourai descriptor/xpub source-set payload."""
    if not isinstance(data, dict):
        raise AppError(
            "Samourai descriptor/xpub set must be a JSON object",
            code="validation",
            retryable=False,
        )
    declared_network = str_or_none(data.get("network"))
    source_network = (
        _normalize_import_network(network, declared_network)
        if declared_network
        else _normalize_import_network(network, None)
    )
    sources = data.get("children") or data.get("sources") or []
    xpub_sources = data.get("xpubs") or []
    if not isinstance(sources, list) or not isinstance(xpub_sources, list):
        raise AppError(
            "Samourai source set children/xpubs must be lists",
            code="validation",
            retryable=False,
        )
    output: list[dict[str, Any]] = []
    for raw_source in sources:
        output.append(
            _explicit_descriptor_source(
                raw_source,
                network=source_network,
                default_gap_limit=gap_limit,
            )
        )
    for raw_source in xpub_sources:
        output.append(
            _explicit_xpub_source(
                raw_source,
                network=source_network,
                default_gap_limit=gap_limit,
            )
        )
    if not output:
        raise AppError(
            "Samourai source set does not contain any descriptor or xpub sources",
            code="validation",
            retryable=False,
        )
    return output


def _explicit_descriptor_source(
    raw_source: Any,
    *,
    network: str,
    default_gap_limit: int | None,
) -> dict[str, Any]:
    if not isinstance(raw_source, dict):
        raise AppError("Samourai source entries must be objects", code="validation")
    descriptor = str_or_none(raw_source.get("descriptor"))
    if descriptor is None:
        raise AppError("Samourai descriptor source is missing descriptor", code="validation")
    section = str_or_none(raw_source.get("section")) or "deposit"
    script_type = str_or_none(raw_source.get("script_type")) or _script_type_from_descriptor(descriptor)
    root_path = str_or_none(raw_source.get("root_path"))
    template = _template_for(section, script_type, root_path)
    root_path = _validate_template_root_path(template, root_path, network)
    gap_limit = _coerce_gap_limit(raw_source.get("gap_limit"), default_gap_limit, template)
    metadata = _safe_template_metadata(template, root_path, gap_limit)
    metadata.update(_safe_explicit_samourai_metadata(raw_source))
    config: dict[str, Any] = {
        "chain": "bitcoin",
        "network": network,
        "descriptor": descriptor,
        "gap_limit": gap_limit,
        SAMOURAI_CONFIG_KEY: metadata,
    }
    change_descriptor = str_or_none(raw_source.get("change_descriptor"))
    if change_descriptor:
        config["change_descriptor"] = change_descriptor
    _validate_descriptor_config(config)
    _validate_descriptor_origin_matches_template(config, template=template, root_path=root_path)
    return {
        "section": template.section,
        "label": str_or_none(raw_source.get("label")) or template.label,
        "config": config,
    }


def _explicit_xpub_source(
    raw_source: Any,
    *,
    network: str,
    default_gap_limit: int | None,
) -> dict[str, Any]:
    if not isinstance(raw_source, dict):
        raise AppError("Samourai xpub entries must be objects", code="validation")
    section = str_or_none(raw_source.get("section")) or "deposit"
    script_type = str_or_none(raw_source.get("script_type")) or "p2wpkh"
    root_path = str_or_none(raw_source.get("root_path"))
    if root_path is None:
        raise AppError(
            "Samourai xpub source is missing root_path",
            code="validation",
            hint="Include the BIP32 account path so provenance is explicit.",
            retryable=False,
        )
    account_xpub = str_or_none(raw_source.get("xpub"))
    fingerprint = str_or_none(raw_source.get("fingerprint")) or "00000000"
    if account_xpub is None:
        raise AppError("Samourai xpub source is missing xpub", code="validation")
    template = _template_for(section, script_type, root_path)
    root_path = _validate_template_root_path(template, root_path, network)
    source = _source_from_account_xpub(
        template,
        network=network,
        account_xpub=account_xpub,
        fingerprint=fingerprint.lower(),
        root_path=root_path,
        gap_limit=_coerce_gap_limit(raw_source.get("gap_limit"), default_gap_limit, template),
    )
    source["config"][SAMOURAI_CONFIG_KEY].update(
        _safe_explicit_samourai_metadata(raw_source)
    )
    return source


def _coerce_gap_limit(
    value: Any,
    default_gap_limit: int | None,
    template: SamouraiAccountTemplate,
) -> int:
    raw = value if value not in (None, "") else default_gap_limit
    try:
        coerced = int(raw) if raw not in (None, "") else None
    except (TypeError, ValueError) as exc:
        raise AppError("Descriptor gap limit must be an integer", code="validation") from exc
    return _gap_limit_for(template, coerced)


def _script_type_from_descriptor(descriptor: str) -> str:
    normalized = re.sub(r"\s+", "", descriptor or "").lower()
    if normalized.startswith("pkh("):
        return "p2pkh"
    if normalized.startswith("sh(wpkh("):
        return "p2sh-p2wpkh"
    if normalized.startswith("wpkh("):
        return "p2wpkh"
    raise AppError(
        "Samourai descriptor source has an unsupported descriptor type",
        code="validation",
        hint="Use pkh(), sh(wpkh()), or wpkh() account descriptors.",
        retryable=False,
    )


def _existing_wallet_labels(conn: sqlite3.Connection, profile_id: str, labels: list[str]) -> set[str]:
    if not labels:
        return set()
    placeholders = ",".join("?" for _ in labels)
    rows = conn.execute(
        f"SELECT label FROM wallets WHERE profile_id = ? AND label IN ({placeholders})",
        (profile_id, *labels),
    ).fetchall()
    return {row["label"] for row in rows}


def _child_label(group_label: str, source: dict[str, Any]) -> str:
    return f"{group_label} - {source['label']}"


def import_samourai_wallet_group(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    *,
    label: str,
    account_ref: str | None = None,
    backend: str | None = None,
    network: str | None = None,
    gap_limit: int | None = None,
    source_set_file: str | None = None,
    source_set: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    sources, normalized_network, import_source = _resolve_import_sources(
        source_set_file=source_set_file,
        source_set=source_set,
        network=network,
        gap_limit=gap_limit,
    )
    group_label = str(label or "").strip()
    if not group_label:
        raise AppError("Samourai group label is required", code="validation")
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    account = resolve_account(conn, profile["id"], account_ref or "treasury")
    group_id = str(uuid.uuid4())
    labels = [group_label] + [_child_label(group_label, source) for source in sources]
    duplicate_labels = sorted(
        label for label, count in Counter(labels).items() if count > 1
    )
    if duplicate_labels:
        raise AppError(
            "Samourai import contains duplicate wallet labels",
            code="validation",
            hint="Give duplicate source entries distinct labels or remove the duplicate entry.",
            details={"duplicate_labels": duplicate_labels},
            retryable=False,
        )
    conflicts = sorted(_existing_wallet_labels(conn, profile["id"], labels))
    if conflicts:
        raise AppError(
            "Samourai import would overwrite existing wallet labels",
            code="conflict",
            hint="Choose a different group label or remove the existing wallet first.",
            details={"conflicting_labels": conflicts},
            retryable=False,
        )
    if backend:
        normalized_backend = backend.strip().lower()
        for source in sources:
            source["config"]["backend"] = normalized_backend
    parent_config = {
        "chain": "bitcoin",
        "network": normalized_network,
        "gap_limit": gap_limit or DEFAULT_DESCRIPTOR_GAP_LIMIT,
        SAMOURAI_CONFIG_KEY: {
            "role": "parent",
            "group_id": group_id,
            "source": import_source,
            "sections": list(SAMOURAI_GROUP_SECTIONS),
            "bip47": "recognised_not_scanned_without_explicit_descriptors",
            "watch_only": True,
        },
    }
    if backend:
        parent_config["backend"] = backend.strip().lower()
    created_children = []
    with conn:
        parent_id = _insert_wallet(
            conn,
            workspace["id"],
            profile["id"],
            account["id"],
            group_label,
            SAMOURAI_PARENT_KIND,
            parent_config,
        )
        for source in sources:
            config = dict(source["config"])
            safe_meta = dict(config.get(SAMOURAI_CONFIG_KEY) or {})
            safe_meta.update(
                {
                    "role": "child",
                    "group_id": group_id,
                    "group_label": group_label,
                    "parent_wallet_id": parent_id,
                    "source": import_source,
                }
            )
            config[SAMOURAI_CONFIG_KEY] = safe_meta
            child_id = _insert_wallet(
                conn,
                workspace["id"],
                profile["id"],
                account["id"],
                _child_label(group_label, source),
                SAMOURAI_CHILD_KIND,
                config,
            )
            created_children.append(child_id)
    parent = core_wallets.wallet_row_to_dict(fetch_wallet_with_account(conn, parent_id))
    children = [
        core_wallets.wallet_row_to_dict(fetch_wallet_with_account(conn, child_id))
        for child_id in created_children
    ]
    return {
        "group": parent,
        "children": children,
        "warnings": _samourai_import_warnings(children),
    }


def _resolve_import_sources(
    *,
    source_set_file: str | None,
    source_set: Mapping[str, Any] | None,
    network: str | None,
    gap_limit: int | None,
) -> tuple[list[dict[str, Any]], str, str]:
    selected = [
        bool(source_set_file),
        source_set is not None,
    ]
    if sum(1 for value in selected if value) != 1:
        raise AppError(
            "Choose exactly one Samourai import source",
            code="validation",
            hint="Use a descriptor/xpub source-set file or inline source_set payload.",
            retryable=False,
        )
    normalized_network = _normalize_import_network(network, None)
    if source_set is not None:
        return (
            load_samourai_source_set_payload(
                source_set,
                network=normalized_network,
                gap_limit=gap_limit,
            ),
            normalized_network,
            "source_set",
        )
    return (
        load_samourai_source_set(
            source_set_file or "",
            network=normalized_network,
            gap_limit=gap_limit,
        ),
        normalized_network,
        "source_set",
    )


def _insert_wallet(
    conn: sqlite3.Connection,
    workspace_id: str,
    profile_id: str,
    account_id: str,
    label: str,
    kind: str,
    config: dict[str, Any],
) -> str:
    normalized_kind = core_wallets.normalize_wallet_kind(kind)
    validated_config = core_wallets._validated_wallet_config(normalized_kind, config)
    wallet_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
        VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            wallet_id,
            workspace_id,
            profile_id,
            account_id,
            label,
            normalized_kind,
            json.dumps(validated_config, sort_keys=True),
            now_iso(),
        ),
    )
    return wallet_id


def _samourai_import_warnings(children: list[dict[str, Any]]) -> list[dict[str, str]]:
    warnings = [
        {
            "code": "bip47_not_auto_scanned",
            "message": "Samourai payment-code paths are recognised but are not scanned unless supplied as explicit descriptors.",
        }
    ]
    if any((child.get("config") or {}).get(SAMOURAI_CONFIG_KEY, {}).get("section") == "postmix" for child in children):
        warnings.append(
            {
                "code": "postmix_gap_watch",
                "message": "Postmix discovery uses a widened gap limit; raise it for old wallets with long unused-address runs.",
            }
        )
    return warnings


def samourai_metadata_from_wallet_config(config: dict[str, Any] | None) -> dict[str, Any] | None:
    if not isinstance(config, dict):
        return None
    metadata = config.get(SAMOURAI_CONFIG_KEY)
    if not isinstance(metadata, dict):
        return None
    safe = _safe_samourai_metadata(metadata, fields=SAMOURAI_SAFE_METADATA_FIELDS)
    return safe or None
