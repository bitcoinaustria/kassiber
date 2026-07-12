"""Profile-wide watch-only ownership coverage assessment.

Coverage is derived from authored wallet-policy declarations plus disposable
sync checkpoints.  It is not a second transaction ledger.  The accounting
boundary cares about two distinct questions:

* policy coverage: can every script belonging to this real-world wallet be
  recognized by the profile-wide ownership index?
* history coverage: did a backend actually scan that policy over the declared
  historical interval?

Only policy coverage is needed to call an unmatched output external in a
transaction graph that is already present.  History coverage remains visible
for diagnostics/backfill and must never be inferred from a gap limit alone.
"""

from __future__ import annotations

import json
import re
import sqlite3
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ..wallet_descriptors import normalize_chain, normalize_network
from ..db import get_setting, set_setting
from .freshness import SOURCE_ONCHAIN, source_key
from .repo import invalidate_journals
from .wallets import OWNERSHIP_HISTORY_CONFIG_KEY, wallet_is_deprecated

POLICY_CONFIG_KEY = "ownership_policy"
PROFILE_UNIVERSE_SETTING_PREFIX = "ownership.policy_universe.profile."
TIER_UNKNOWN = "unknown"
TIER_ASSUMED = "assumed"
TIER_PROVEN = "proven"
TIERS = (TIER_UNKNOWN, TIER_ASSUMED, TIER_PROVEN)

EVIDENCE_USER_ATTESTED = "user_attested"
EVIDENCE_WALLET_EXPORT = "wallet_export"
EVIDENCE_BACKEND_REPORTED = "backend_reported"
EVIDENCE_KINDS = {
    EVIDENCE_USER_ATTESTED,
    EVIDENCE_WALLET_EXPORT,
    EVIDENCE_BACKEND_REPORTED,
}


@dataclass(frozen=True)
class WalletOwnershipCoverage:
    wallet_id: str
    wallet_label: str
    chain: str
    network: str
    policy_tier: str
    history_tier: str
    complete: bool
    evidence: str
    policy_set_id: str
    policy_shape: str
    branch_last_issued: Mapping[str, int]
    derived_through: Mapping[str, int]
    limitations: tuple[str, ...]
    repair_actions: tuple[str, ...]

    def to_safe_dict(self) -> dict[str, Any]:
        return {
            "wallet_id": self.wallet_id,
            "wallet_label": self.wallet_label,
            "chain": self.chain,
            "network": self.network,
            "policy_tier": self.policy_tier,
            "history_tier": self.history_tier,
            "complete": self.complete,
            "evidence": self.evidence,
            "policy_set_id": self.policy_set_id,
            "policy_shape": self.policy_shape,
            "branch_last_issued": dict(sorted(self.branch_last_issued.items())),
            "derived_through": dict(sorted(self.derived_through.items())),
            "limitations": list(self.limitations),
            "repair_actions": list(self.repair_actions),
        }


@dataclass(frozen=True)
class ProfileOwnershipCoverage:
    profile_id: str
    wallets: tuple[WalletOwnershipCoverage, ...]
    universe_complete: bool = False
    universe_evidence: str = ""

    def tier_for(self, chain: str, network: str) -> str:
        if not self.universe_complete:
            return TIER_UNKNOWN
        relevant = [
            wallet.policy_tier
            for wallet in self.wallets
            if wallet.chain == chain and wallet.network == network
        ]
        if not relevant:
            return TIER_UNKNOWN
        return min(relevant, key=TIERS.index)

    def is_policy_proven(self, chain: str, network: str) -> bool:
        return self.tier_for(chain, network) == TIER_PROVEN

    def to_safe_dict(self) -> dict[str, Any]:
        scopes = sorted({(wallet.chain, wallet.network) for wallet in self.wallets})
        return {
            "profile_id": self.profile_id,
            "universe_complete": self.universe_complete,
            "universe_evidence": self.universe_evidence,
            "scopes": [
                {"chain": chain, "network": network, "policy_tier": self.tier_for(chain, network)}
                for chain, network in scopes
            ],
            "wallets": [wallet.to_safe_dict() for wallet in self.wallets],
        }


def normalize_policy_declaration(value: Any) -> dict[str, Any]:
    if value in (None, ""):
        return {}
    if not isinstance(value, Mapping):
        raise ValueError("ownership_policy must be an object")
    complete = value.get("complete")
    if type(complete) is not bool:
        raise ValueError("ownership_policy.complete must be a boolean")
    evidence = str(value.get("evidence") or EVIDENCE_USER_ATTESTED).strip().lower()
    if evidence not in EVIDENCE_KINDS:
        raise ValueError("ownership_policy.evidence is not supported")
    raw_bounds = value.get("branch_last_issued") or {}
    if not isinstance(raw_bounds, Mapping):
        raise ValueError("ownership_policy.branch_last_issued must be an object")
    bounds: dict[str, int] = {}
    for branch, raw_index in raw_bounds.items():
        try:
            index = int(raw_index)
        except (TypeError, ValueError) as exc:
            raise ValueError("ownership policy branch bounds must be integers") from exc
        if index < 0 or index > 20_000:
            raise ValueError("ownership policy branch bounds must be between 0 and 20000")
        bounds[str(int(branch))] = index
    policy_set_id = str(value.get("policy_set_id") or "").strip().lower()
    if policy_set_id and not re.fullmatch(r"[a-z0-9][a-z0-9._-]{0,63}", policy_set_id):
        raise ValueError("ownership_policy.policy_set_id must be a stable token")
    return {
        "complete": complete,
        "evidence": evidence,
        "policy_set_id": policy_set_id,
        "branch_last_issued": dict(sorted(bounds.items())),
    }


def assess_profile_ownership_coverage(
    conn: sqlite3.Connection,
    profile_id: str,
    wallets: Sequence[Mapping[str, Any]] | None = None,
    *,
    derived_through_by_wallet: Mapping[str, Mapping[str, int]] | None = None,
    derivation_complete_by_wallet: Mapping[str, bool] | None = None,
) -> ProfileOwnershipCoverage:
    if wallets is None:
        wallets = conn.execute(
            "SELECT id, label, kind, config_json FROM wallets WHERE profile_id = ? ORDER BY label",
            (profile_id,),
        ).fetchall()
    assessed: list[WalletOwnershipCoverage] = []
    for wallet in wallets:
        config = _json_object(wallet["config_json"])
        if wallet_is_deprecated(config) or not _has_onchain_ownership_material(config):
            continue
        assessed.append(
            _assess_wallet(
                conn,
                profile_id,
                wallet,
                config,
                (derived_through_by_wallet or {}).get(str(wallet["id"])),
                (derivation_complete_by_wallet or {}).get(str(wallet["id"]), True),
            )
        )
    universe = _profile_universe_declaration(conn, profile_id)
    return ProfileOwnershipCoverage(
        profile_id=profile_id,
        wallets=tuple(assessed),
        universe_complete=bool(universe.get("complete")),
        universe_evidence=str(universe.get("evidence") or ""),
    )


def build_ownership_coverage_snapshot(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    wallet_id: str | None = None,
) -> dict[str, Any]:
    coverage = assess_profile_ownership_coverage(conn, profile_id)
    wallets = [
        wallet for wallet in coverage.wallets
        if wallet_id is None or wallet.wallet_id == wallet_id
    ]
    counts = {tier: 0 for tier in TIERS}
    repairs: list[str] = []
    for wallet in wallets:
        counts[wallet.policy_tier] += 1
        repairs.extend(wallet.repair_actions)
    return {
        "profile_id": profile_id,
        "summary": {
            "wallets": len(wallets),
            "policy_unknown": counts[TIER_UNKNOWN],
            "policy_assumed": counts[TIER_ASSUMED],
            "policy_proven": counts[TIER_PROVEN],
            "all_policy_proven": bool(wallets) and counts[TIER_PROVEN] == len(wallets),
            "wallet_universe_complete": coverage.universe_complete,
            "effective_policy_proven": bool(wallets)
            and all(
                coverage.is_policy_proven(wallet.chain, wallet.network)
                for wallet in wallets
            ),
        },
        "wallets": [wallet.to_safe_dict() for wallet in wallets],
        "repair_actions": list(
            dict.fromkeys(
                (["attest_all_relevant_wallets_added"] if not coverage.universe_complete else [])
                + repairs
            )
        ),
    }


def attest_profile_wallet_universe(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    complete: bool,
    evidence: str = EVIDENCE_USER_ATTESTED,
    commit: bool = True,
) -> dict[str, Any]:
    if type(complete) is not bool:
        raise ValueError("wallet universe completeness must be a boolean")
    evidence = str(evidence or "").strip().lower()
    if evidence not in EVIDENCE_KINDS:
        raise ValueError("wallet universe evidence is not supported")
    value = {"complete": complete, "evidence": evidence}
    set_setting(
        conn,
        f"{PROFILE_UNIVERSE_SETTING_PREFIX}{profile_id}",
        json.dumps(value, sort_keys=True),
    )
    invalidate_journals(conn, profile_id)
    if commit:
        conn.commit()
    return value


def clear_profile_wallet_universe_attestation(
    conn: sqlite3.Connection,
    profile_id: str,
    *,
    commit: bool = True,
) -> bool:
    """Revoke the universe claim after an ownership-topology change."""

    cursor = conn.execute(
        "DELETE FROM settings WHERE key = ?",
        (f"{PROFILE_UNIVERSE_SETTING_PREFIX}{profile_id}",),
    )
    changed = bool(cursor.rowcount)
    if changed:
        invalidate_journals(conn, profile_id)
    if commit:
        conn.commit()
    return changed


def _assess_wallet(
    conn, profile_id, wallet, config, derived_override, derivation_complete
) -> WalletOwnershipCoverage:
    chain = normalize_chain(config.get("chain"))
    network = normalize_network(chain, config.get("network"))
    limitations: list[str] = []
    repairs: list[str] = []
    policy_configs = [config, *[
        item for item in config.get(OWNERSHIP_HISTORY_CONFIG_KEY, [])
        if isinstance(item, Mapping)
    ]]
    declarations: list[dict[str, Any]] = []
    bounds: dict[str, int] = {}
    historic_silent_policy_unproven = False
    for ordinal, policy_config in enumerate(policy_configs):
        try:
            declaration = normalize_policy_declaration(
                policy_config.get(POLICY_CONFIG_KEY)
            )
        except ValueError:
            declaration = {}
        declarations.append(declaration)
        if not declaration.get("complete"):
            limitation = (
                "wallet_policy_set_not_declared_complete"
                if ordinal == 0
                else "historic_policy_coverage_missing"
            )
            limitations.append(limitation)
            repairs.append(
                "declare_complete_wallet_policy_set"
                if ordinal == 0
                else "attest_retired_wallet_policies"
            )
        policy_bounds = dict(declaration.get("branch_last_issued") or {})
        for branch, index in policy_bounds.items():
            bounds[branch] = max(bounds.get(branch, -1), int(index))
        if _has_wildcard_policy(policy_config) and not policy_bounds:
            limitations.append(
                "wildcard_branch_bounds_missing"
                if ordinal == 0
                else "historic_policy_coverage_missing"
            )
            repairs.append(
                "import_last_issued_branch_bounds"
                if ordinal == 0
                else "attest_retired_wallet_policies"
            )
        if ordinal > 0 and policy_config.get("sp_descriptor"):
            # A later wallet checkpoint cannot prove an earlier Silent
            # Payments scan. Until scan evidence is archived per policy, keep
            # the whole real-world wallet policy set fail-closed.
            historic_silent_policy_unproven = True
            limitations.append("historic_silent_payment_scan_not_proven")
            repairs.append("author_manual_custody_component")

    current_declaration = declarations[0] if declarations else {}
    complete = bool(declarations) and all(
        declaration.get("complete") for declaration in declarations
    )
    evidence_tiers = [
        str(declaration.get("evidence") or "") for declaration in declarations
    ]
    evidence = str(current_declaration.get("evidence") or "")
    policy_set_id = str(
        current_declaration.get("policy_set_id") or str(wallet["id"])
    )

    checkpoint = _checkpoint(conn, profile_id, str(wallet["id"]))
    derived = _int_map(derived_override)
    if not derived:
        derived = _int_map(checkpoint.get("ownership_derived_through"))
    if not derived:
        derived = _int_map(checkpoint.get("bitcoinrpc_descriptor_range_ends"))
    if not derived:
        highest = _int_map(checkpoint.get("highest_used"))
        gap = int(config.get("gap_limit") or 0)
        derived = {branch: index + gap for branch, index in highest.items()}

    missing_bounds = [
        branch for branch, last_issued in bounds.items()
        if int(derived.get(branch, -1)) < int(last_issued)
    ]
    if missing_bounds:
        limitations.append("declared_policy_not_fully_derived")
        repairs.append("extend_ownership_derivation")
    if not derivation_complete:
        limitations.append("wallet_policy_derivation_incomplete")
        repairs.append("repair_wallet_policy_derivation")

    silent_policy_proven = True
    if config.get("sp_descriptor"):
        silent_checkpoint = (
            checkpoint.get("silent_payment")
            if isinstance(checkpoint.get("silent_payment"), Mapping)
            else {}
        )
        silent_policy_proven = bool(
            config.get("sp_full_history")
            and silent_checkpoint.get("full_history")
            and silent_checkpoint.get("scan_complete")
            and not silent_checkpoint.get("degraded")
        )
        if not silent_policy_proven:
            limitations.append("silent_payment_full_history_not_proven")
            repairs.append("run_private_full_history_scan")

    policy_tier = TIER_UNKNOWN
    if (
        complete
        and not missing_bounds
        and derivation_complete
        and silent_policy_proven
        and not historic_silent_policy_unproven
        and not any(
            item in limitations
            for item in (
                "wildcard_branch_bounds_missing",
                "historic_policy_coverage_missing",
                "wallet_policy_set_not_declared_complete",
            )
        )
    ):
        policy_tier = (
            TIER_ASSUMED
            if EVIDENCE_USER_ATTESTED in evidence_tiers
            else TIER_PROVEN
        )

    history_tier = _history_tier(config, checkpoint, policy_tier)
    if history_tier != TIER_PROVEN:
        limitations.append("backend_history_not_proven")
        repairs.append("run_private_full_history_scan")

    return WalletOwnershipCoverage(
        wallet_id=str(wallet["id"]),
        wallet_label=str(wallet["label"]),
        chain=chain,
        network=network,
        policy_tier=policy_tier,
        history_tier=history_tier,
        complete=complete,
        evidence=evidence,
        policy_set_id=policy_set_id,
        policy_shape=_policy_shape(config),
        branch_last_issued=bounds,
        derived_through=derived,
        limitations=tuple(dict.fromkeys(limitations)),
        repair_actions=tuple(dict.fromkeys(repairs)),
    )


def _history_tier(config, checkpoint, policy_tier):
    if policy_tier == TIER_UNKNOWN or not checkpoint:
        return TIER_UNKNOWN
    backend = checkpoint.get("backend") if isinstance(checkpoint.get("backend"), Mapping) else {}
    kind = str(backend.get("kind") or "")
    if config.get("sp_descriptor"):
        silent = (
            checkpoint.get("silent_payment")
            if isinstance(checkpoint.get("silent_payment"), Mapping)
            else {}
        )
        if silent.get("scan_complete") and not silent.get("degraded"):
            return TIER_PROVEN if silent.get("full_history") else TIER_ASSUMED
        return TIER_UNKNOWN
    if kind == "bitcoinrpc" and config.get("birthday") and checkpoint.get("bitcoinrpc_last_block"):
        return TIER_PROVEN if policy_tier == TIER_PROVEN else TIER_ASSUMED
    if kind in {"esplora", "electrum"} and checkpoint.get("highest_used"):
        return TIER_ASSUMED
    return TIER_UNKNOWN


def _checkpoint(conn, profile_id, wallet_id):
    try:
        row = conn.execute(
            "SELECT checkpoint_json FROM freshness_source_states WHERE profile_id = ? AND source_key = ?",
            (profile_id, source_key(SOURCE_ONCHAIN, wallet_id)),
        ).fetchone()
    except sqlite3.OperationalError:
        # Pure-engine callers intentionally use a minimal schema.
        return {}
    return _json_object(row["checkpoint_json"] if row else None)


def _profile_universe_declaration(conn, profile_id):
    try:
        raw = get_setting(conn, f"{PROFILE_UNIVERSE_SETTING_PREFIX}{profile_id}")
    except sqlite3.OperationalError:
        return {}
    return _json_object(raw)


def _has_onchain_ownership_material(config):
    return bool(
        config.get("descriptor")
        or config.get("xpub")
        or config.get("addresses")
        or config.get("sp_descriptor")
        or config.get(OWNERSHIP_HISTORY_CONFIG_KEY)
    )


def _has_wildcard_policy(config):
    return bool(config.get("xpub") or "*" in str(config.get("descriptor") or "") or "*" in str(config.get("change_descriptor") or ""))


def _policy_shape(config):
    descriptor = str(config.get("descriptor") or "").lower()
    if "sortedmulti(" in descriptor or "multi(" in descriptor:
        return "multisig_descriptor"
    if config.get("xpub"):
        return "multi_script_xpub" if len(config.get("script_types") or []) > 1 else "single_key_xpub"
    if config.get("sp_descriptor"):
        return "silent_payment"
    if descriptor:
        return "descriptor"
    if config.get("addresses"):
        return "finite_addresses"
    return "unknown"


def _json_object(value):
    if isinstance(value, Mapping):
        return dict(value)
    try:
        parsed = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return dict(parsed) if isinstance(parsed, Mapping) else {}


def _int_map(value):
    if not isinstance(value, Mapping):
        return {}
    output = {}
    for key, raw in value.items():
        try:
            parsed = int(raw)
        except (TypeError, ValueError):
            continue
        if parsed >= 0:
            output[str(int(key))] = parsed
    return output


__all__ = [
    "POLICY_CONFIG_KEY",
    "ProfileOwnershipCoverage",
    "WalletOwnershipCoverage",
    "assess_profile_ownership_coverage",
    "attest_profile_wallet_universe",
    "build_ownership_coverage_snapshot",
    "clear_profile_wallet_universe_attestation",
    "normalize_policy_declaration",
]
