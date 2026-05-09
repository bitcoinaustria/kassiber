"""Actionable next-step hints for source-funds findings.

Translates the report's finding codes (e.g. ``missing_history``,
``unreviewed_link``) into UI-renderable next-step objects. The translation
is intentionally pure: it takes a finding dict and returns the same dict with
a ``next_step`` field added, so it can be unit-tested without a database.

The shape is stable so the desktop UI and the CLI can both render the
same hint:

    {
        "headline": "Attest the missing-history gap",
        "action": "open_source_creator",
        "action_args": {"source_type": "missing_history"},
        "doc_anchor": "missing-history",
    }

Action names map to UI intents the desktop app already knows about. Unknown
codes get a generic hint pointing at the docs so users still see something
concrete instead of a raw code.
"""

from __future__ import annotations

from typing import Any, Mapping


_FINDING_HINTS: dict[str, dict[str, Any]] = {
    "missing_history": {
        "headline": "Attach a root source or attest the gap",
        "action": "open_source_creator",
        "action_args": {"source_type": "missing_history"},
        "doc_anchor": "missing-history",
    },
    "missing_pricing": {
        "headline": "Add a fiat price for this transaction",
        "action": "open_transaction",
        "action_args": {"focus": "pricing"},
        "doc_anchor": "fiat-pricing",
    },
    "asset_mismatch": {
        "headline": "Re-review the link with matching assets",
        "action": "open_link_review",
        "action_args": {"focus": "asset"},
        "doc_anchor": "asset-mismatch",
    },
    "source_asset_mismatch": {
        "headline": "Pick a source whose asset matches the link",
        "action": "open_link_review",
        "action_args": {"focus": "source-asset"},
        "doc_anchor": "asset-mismatch",
    },
    "transaction_overallocation": {
        "headline": "Reduce the link allocation to the parent transaction amount",
        "action": "open_link_review",
        "action_args": {"focus": "allocation"},
        "doc_anchor": "over-allocation",
    },
    "source_overallocation": {
        "headline": "Reduce link allocations or split across multiple sources",
        "action": "open_link_review",
        "action_args": {"focus": "allocation"},
        "doc_anchor": "over-allocation",
    },
    "source_amount_missing": {
        "headline": "Set the source's amount before exporting",
        "action": "open_source",
        "action_args": {"focus": "amount"},
        "doc_anchor": "source-amount",
    },
    "path_truncated": {
        "headline": "Resolve the upstream path or attest a missing-history stop",
        "action": "open_source_creator",
        "action_args": {"source_type": "missing_history"},
        "doc_anchor": "max-depth",
    },
    "path_cycle": {
        "headline": "Break the cycle by removing one of the reviewed links",
        "action": "open_link_review",
        "action_args": {"focus": "cycle"},
        "doc_anchor": "path-cycle",
    },
    "unreviewed_link": {
        "headline": "Review the suggested link or reject it",
        "action": "open_link_review",
        "action_args": {"focus": "review"},
        "doc_anchor": "review-queue",
    },
    "ambiguous_allocation": {
        "headline": "Set an explicit allocation amount and policy on the link",
        "action": "open_link_review",
        "action_args": {"focus": "allocation"},
        "doc_anchor": "explicit-allocation",
    },
    "unconfirmed_chain_data": {
        "headline": "Confirm or remove the chain observation backing this link",
        "action": "open_link_review",
        "action_args": {"focus": "chain-observation"},
        "doc_anchor": "chain-observations",
    },
    "chain_observation_privacy": {
        "headline": "Chain-only evidence is context, not proof: add primary evidence",
        "action": "open_link_review",
        "action_args": {"focus": "evidence"},
        "doc_anchor": "evidence-tiers",
    },
    "privacy_hop_unresolved": {
        "headline": "Attach explicit evidence covering the privacy hop",
        "action": "open_link_review",
        "action_args": {"focus": "evidence"},
        "doc_anchor": "privacy-hops",
    },
    "chronology_violation": {
        "headline": "Pick a parent dated on or before the child transaction",
        "action": "open_link_review",
        "action_args": {"focus": "chronology"},
        "doc_anchor": "chronology",
    },
    "opening_balance_attestation": {
        "headline": "Opening-balance attestations are weaker than primary evidence",
        "action": "open_source",
        "action_args": {"focus": "evidence"},
        "doc_anchor": "opening-balance",
    },
}


_GENERIC_HINT = {
    "headline": "Open the relevant link or source to resolve this finding",
    "action": "open_review_queue",
    "action_args": {},
    "doc_anchor": "findings",
}


def hint_for_code(code: str) -> dict[str, Any]:
    """Return the next-step hint template for a finding code.

    Always returns a dict; unknown codes fall back to a generic hint so
    downstream renderers don't need to defend against ``None``.
    """
    return dict(_FINDING_HINTS.get(code, _GENERIC_HINT))


def enrich_findings_with_next_steps(findings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Attach a ``next_step`` field to every finding.

    Mutates and returns the list. The hint is keyed by ``code`` only; the
    finding's ``ref`` is left where it is so renderers can deep-link without
    an extra round trip.
    """
    for finding in findings:
        finding.setdefault("next_step", hint_for_code(str(finding.get("code", ""))))
    return findings


def known_finding_codes() -> tuple[str, ...]:
    """Stable, sorted tuple of finding codes that have a hint."""
    return tuple(sorted(_FINDING_HINTS.keys()))


def hint_action_names() -> tuple[str, ...]:
    """All distinct action names hints can request from the UI."""
    actions: set[str] = {_GENERIC_HINT["action"]}
    for hint in _FINDING_HINTS.values():
        action = hint.get("action")
        if isinstance(action, str):
            actions.add(action)
    return tuple(sorted(actions))


def coverage_for_known_codes(codes: Mapping[str, Any]) -> dict[str, bool]:
    """Map of code -> True if a hint exists.

    Useful for tests that assert every finding code emitted by the report
    builder has a matching hint.
    """
    return {code: code in _FINDING_HINTS for code in codes}
