#!/usr/bin/env python3
"""Build the Xcode String Catalog from the SwiftPM `.lproj` sidecars."""

from __future__ import annotations

import json
import re
import argparse
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / "Sources" / "KassiberApp" / "Resources"
SWIFT_SOURCES = ROOT / "Sources" / "KassiberApp"

LOCALIZATION_CALL_PATTERN = re.compile(
    r'(?:AppLocalization[.]string|analysisLocalized|chatLocalized|parityString|'
    r'reportsImportsLocalized|logsLocalized|notificationLocalized|reviewLocalized|'
    r'exportLocalized|localized)\(\s*"([^"]+)"'
)

RAW_ERROR_PRESENTATION_PATTERNS = (
    re.compile(r"Text\(\s*error\s*\)"),
    re.compile(r"Label\(\s*error\s*,"),
    re.compile(r"[.]help\(model[.]errorMessage\s*[?][?]"),
)

# These stable daemon values are rendered as user-facing labels by the native
# review, sync, chat, and log surfaces. Keep the list explicit so a catalog
# cleanup cannot silently fall back to English-style code humanization in de-AT.
REQUIRED_PRESENTATION_CODE_KEYS = {
    "code.error", "code.cancelled", "code.awaiting_consent",
    "code.trace", "code.debug", "code.info", "code.warning", "code.critical",
    "code.acquisition", "code.disposal", "code.fee", "code.transfer_fee",
    "code.transfer_in", "code.transfer_out", "code.income", "code.neutral_swap",
    "code.missing_spot_price", "code.pricing_review_required",
    "code.transfer_fee_implausible", "code.ownership_transfer_amount_mismatch",
    "code.ownership_transfer_unresolved", "code.owned_fanout_unresolved",
    "code.conflicting_spend", "code.pending_onchain_confirmation",
    "code.derived_transfer_group_blocked", "code.insufficient_lots",
    "code.missing_cost_basis", "code.manual_multi_pair_ambiguous",
    "code.privacy_hop_unresolved", "code.transfer_mismatch",
    "code.unsupported_tax_direction", "code.basis_provenance_incomplete",
    "code.non_sale_disposal_kind", "code.unclassified_income_kind",
    "code.bitcoin_rail_carry_basis_unresolved", "code.at_swap_basis_carry_unresolved",
    "code.channel_open_unresolved", "code.channel_close_unresolved",
    "code.exact", "code.strong", "code.payment_hash", "code.provider_swap_id",
    "code.heuristic", "code.htlc_refund", "code.ownership_graph",
    "code.manual", "code.coinjoin", "code.whirlpool", "code.chain_swap",
    "code.peg_in", "code.peg_out", "code.reverse_submarine_swap",
    "code.submarine_swap", "code.swap_refund", "code.carrying_value",
    "code.taxable", "code.bulk_exact", "code.bulk_selected", "code.rule_auto",
    "code.owned", "code.external", "code.unknown", "code.invalid",
    "code.owned_address", "code.external_address", "code.self_transfer",
    "code.outbound_payment", "code.inbound_receipt", "code.touches_wallet",
    "code.undetermined", "code.receive", "code.change",
    "code.discovery", "code.backend_fetch", "code.decode_enrich",
    "code.rate_coverage", "code.journal_refresh", "code.auto_pair",
    "code.reading_local_context", "code.waiting_for_model",
}

# These keys are assembled from bounded daemon values at runtime, so the
# literal-call audit below cannot discover them from Swift source alone.
REQUIRED_DYNAMIC_LOCALIZATION_KEYS = {
    "connections.scriptType.p2wpkh",
    "connections.scriptType.p2sh-p2wpkh",
    "connections.scriptType.p2pkh",
    "connections.scriptType.p2tr",
    "error.operationFailed",
    "error.transactionNotFound",
    "error.unexpectedOverview",
    "error.unexpectedTransactions",
    "wallet.inventoryUnsupported",
    "transactionFlow.role.input",
    "transactionFlow.role.output",
    "transactionFlow.role.change",
    "transactionFlow.role.external_recipient",
    "transactionFlow.role.incoming_payment",
    "transactionFlow.role.owned_destination",
    "transactionFlow.role.op_return",
    "transactionFlow.role.fee",
    "transactionFlow.role.overflow",
    "transactionFlow.role.ambiguous_owned_output",
    "transactionFlow.role.leg",
    "transactionFlow.role.spend",
    "transactionFlow.role.receive",
    "transactionFlow.role.consolidation",
    "transactionFlow.ownership.network_fee",
    "transactionFlow.ownership.owned",
    "transactionFlow.ownership.external",
    "transactionFlow.ownership.ambiguous",
    "transactionFlow.ownership.unspendable",
    "transactionFlow.ownership.overflow",
    "transactionFlow.ownership.unknown",
    "transactionDraft.network.bitcoin",
    "transactionDraft.network.lightning",
    "transactionDraft.network.liquid",
    "transactionDraft.network.ecash",
    "transactionDraft.network.exchange",
    "transactionDraft.network.other",
    "transactionDraft.flow.incoming",
    "transactionDraft.flow.outgoing",
    "transactionDraft.flow.transfer",
    "transactionDraft.flow.swap",
    "transactionDraft.flow.layer-transition",
    "transactionDraft.pricing.manual_override",
    "transactionDraft.pricing.generic_import",
    "transactionDraft.pricing.fmv_provider",
    "transactionDraft.pricing.missing",
    "transactionFlow.tell.sender_common_input",
    "transactionFlow.tell.common_input",
    "transactionFlow.tell.fee_fingerprint",
    "transactionFlow.tell.sender_rbf",
    "transactionFlow.tell.op_return_output",
    "transactionFlow.tell.change_output",
    "transactionFlow.tell.unknown_provenance",
    "transactionFlow.tell.known_source_proximity",
    "transactionFlow.tell.source_proximity_coverage_gaps",
    "transactionFlow.tell.coverage_degraded",
    "transactionFlow.warning.graphless_import",
    "transactionFlow.warning.liquid_reference_graph_not_local",
    "transactionFlow.warning.graph_lookup_failed",
    "transactionFlow.warning.graph_lookup_warning",
    "transactionFlow.warning.bitcoin_reference_lookup_mismatch",
    "transactionFlow.warning.bitcoin_reference_lookup_incomplete",
    "transactionFlow.warning.ownership_index",
    "transactionFlow.warning.no_active_profile",
    "transactionFlow.warning.not_found",
    "transactionFlow.warning.missing_cost_basis",
    "code.full",
    "code.partial",
    "code.graphless",
    "code.unsupported",
    "sourceFundsParity.status.suggestionsAdded %lld",
    "sourceFundsParity.status.assembled %lld %lld",
    "sourceFundsParity.status.bulkReviewed %lld %lld",
    "sourceFundsParity.status.evidenceAttached",
    "sourceFundsParity.status.linkAccepted",
    "sourceFundsParity.status.linkRejected",
    "sourceFundsParity.status.recipientUpdated",
    "sourceFundsParity.status.recipientDeleted",
    "importsParity.preview.wasabiBundle",
    "importsParity.preview.validatedSections %lld",
    "importsParity.preview.problemRow %lld %@",
    "importsParity.preview.mappedErrors %lld %lld",
    "importsParity.preview.columnsRows %lld %lld",
    "privacy.tellKind.sender_rbf",
    "privacy.tellKind.address_reuse",
    "privacy.reco.sender_rbf",
    "privacy.reco.address_reuse",
    "privacy.assumption.no_reputation_lists",
    "privacy.assumption.local_inventory_scope",
    "privacy.assumption.not_global_kyc_knowledge",
    "privacy.assumption.hypothetical_not_identity_claim",
    "privacy.worstKind.address_reuse",
    "privacy.worstKind.change_output",
    "privacy.worstKind.storage",
    "privacy.worstKind.network",
    "privacy.worstKind.ai",
    "privacy.worstKind.wallets",
    "privacy.worstKind.transactions",
    "privacy.worstKind.inventory",
    "privacy.worstKind.journals",
    "privacy.reco.storage",
    "privacy.reco.network",
    "privacy.reco.ai",
    "privacy.reco.wallets",
    "privacy.reco.transactions",
    "privacy.reco.inventory",
    "privacy.reco.journals",
}


def read_strings(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    pattern = re.compile(r'^\s*"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)"\s*;\s*$')
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.lstrip().startswith("/*"):
            continue
        match = pattern.match(line)
        if not match:
            raise ValueError(f"could not parse {path}:{number}")
        key, value = (json.loads(f'"{group}"') for group in match.groups())
        if key in values:
            raise ValueError(f"duplicate localization key {key!r} in {path}:{number}")
        values[key] = value
    return values


def literal_localization_references() -> set[str]:
    """Return non-interpolated catalog keys referenced directly by SwiftUI."""

    references: set[str] = set()
    for path in SWIFT_SOURCES.rglob("*.swift"):
        for key in LOCALIZATION_CALL_PATTERN.findall(path.read_text(encoding="utf-8")):
            if r"\(" not in key:
                references.add(key)
    return references


def raw_error_presentations() -> list[str]:
    """Find error strings rendered without the localization boundary."""

    violations: list[str] = []
    for path in SWIFT_SOURCES.rglob("*.swift"):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if any(pattern.search(line) for pattern in RAW_ERROR_PRESENTATION_PATTERNS):
                violations.append(f"{path.relative_to(ROOT)}:{number}")
    return violations


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    english = read_strings(RESOURCES / "en.lproj" / "Localizable.strings")
    german = read_strings(RESOURCES / "de.lproj" / "Localizable.strings")
    if english.keys() != german.keys():
        missing_de = sorted(english.keys() - german.keys())
        missing_en = sorted(german.keys() - english.keys())
        raise ValueError(f"locale keys differ: missing de={missing_de}, missing en={missing_en}")
    missing_codes = sorted(
        (REQUIRED_PRESENTATION_CODE_KEYS | REQUIRED_DYNAMIC_LOCALIZATION_KEYS)
        - english.keys()
    )
    if missing_codes:
        raise ValueError(f"missing native presentation code labels: {missing_codes}")
    missing_references = sorted(literal_localization_references() - english.keys())
    if missing_references:
        raise ValueError(f"SwiftUI references missing localization keys: {missing_references}")
    raw_errors = raw_error_presentations()
    if raw_errors:
        raise ValueError(f"SwiftUI renders unlocalized error strings: {raw_errors}")
    strings = {}
    for key in sorted(english):
        strings[key] = {
            "localizations": {
                "de": {"stringUnit": {"state": "translated", "value": german[key]}},
                "en": {"stringUnit": {"state": "translated", "value": english[key]}},
            }
        }
    catalog = {"sourceLanguage": "en", "strings": strings, "version": "1.0"}
    path = RESOURCES / "Localizable.xcstrings"
    expected = json.dumps(catalog, ensure_ascii=False, indent=2) + "\n"
    if args.check:
        if not path.exists() or path.read_text(encoding="utf-8") != expected:
            print("Localizable.xcstrings is stale", file=sys.stderr)
            raise SystemExit(1)
    else:
        path.write_text(expected, encoding="utf-8")
    print(f"synced {len(strings)} bilingual strings")


if __name__ == "__main__":
    main()
