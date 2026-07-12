"""Curated AI tool catalog for Kassiber's in-app assistant."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
import sys
from typing import Any, Literal

from ..errors import AppError
from ..redaction import is_sensitive_key, redact_secret_text


ToolKindClass = Literal["read_only", "mutating"]

SENSITIVE_ARGUMENT_KEY_PARTS = (
    "api_key",
    "auth",
    "blinding",
    "config_json",
    "cookie",
    "descriptor",
    "header",
    "mnemonic",
    "password",
    "passphrase",
    "private",
    "recovery",
    "secret",
    "seed",
    "token",
    "wif",
    "xprv",
)

_AI_TOOL_RESULT_URL_RE = re.compile(
    r"\b[a-z][a-z0-9+.-]*://[^\s'\"<>]+",
    re.IGNORECASE,
)
# Require at least two path segments so typed UI routes such as
# ``/transactions`` are not mistaken for local filesystem paths. URLs are
# removed first, before their slash-delimited paths reach this pattern.
_AI_TOOL_RESULT_ABSOLUTE_PATH_RE = re.compile(
    r"(?:[A-Za-z]:[\\/]|[\\/]{2}|~[\\/]|(?<![A-Za-z0-9_])[\\/])"
    r"[\w@.+ -]+(?:[\\/][\w@.+ -]+)+"
)


@dataclass(frozen=True)
class ToolEntry:
    name: str
    description: str
    parameters: dict[str, Any]
    kind_class: ToolKindClass
    wire_name: str | None = None
    daemon_kind: str | None = None
    summary_template: str | None = None

    @property
    def provider_name(self) -> str:
        return self.wire_name or self.name

    def to_openai_tool(self) -> dict[str, Any]:
        return {
            "type": "function",
            "function": {
                "name": self.provider_name,
                "description": self.description,
                "parameters": self.parameters,
            },
        }


SKILL_REFERENCE_NAMES = (
    "index",
    "command-templates",
    "journal-processing",
    "metadata",
    "onboarding",
    "reports",
    "secrets-and-backup",
    "swap-matching",
    "troubleshooting",
    "verification",
    "wallets-backends",
)

TOOL_PROFILE_NAMES = ("core", "full")

CORE_TOOL_NAMES = frozenset(
    {
        "status", "ui.overview.snapshot", "ui.transactions.list",
        "ui.transactions.extremes", "ui.transactions.search", "ui.wallets.list",
        "ui.wallets.identify", "ui.backends.list", "ui.profiles.snapshot",
        "ui.reports.capital_gains", "ui.reports.summary",
        "ui.reports.balance_sheet", "ui.reports.portfolio_summary",
        "ui.reports.tax_summary", "ui.reports.balance_history",
        "ui.journals.snapshot", "ui.journals.quarantine",
        "ui.journals.transfers.list", "ui.rates.summary", "ui.rates.coverage",
        "ui.rates.rebuild", "ui.report.blockers",
        "ui.audit.changes_since_last_answer", "ui.maintenance.settings",
        "ui.workspace.health", "ui.next_actions", "read_skill_reference",
        "ui.wallets.sync", "ui.journals.process", "ui.maintenance.run",
        "ui.transfers.suggest", "ui.transfers.review_context",
        "ui.transfers.list", "ui.transfers.rules.list",
    }
)


_EMPTY_OBJECT_SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "properties": {},
}

_SOURCE_FUNDS_SOURCE_TYPES = (
    "fiat_purchase",
    "exchange_withdrawal",
    "mining",
    "income",
    "gift",
    "opening_balance_attestation",
    "missing_history",
    "unknown",
)
_SOURCE_FUNDS_LINK_TYPES = (
    "self_transfer",
    "exchange_transfer",
    "trade",
    "swap",
    "peg_in",
    "peg_out",
    "lightning_funding",
    "lightning_close",
    "lightning_routed",
    "lightning_swap",
    "coinjoin",
    "payjoin",
    "manual_source",
    "missing_history",
)
_SOURCE_FUNDS_LINK_STATES = ("suggested", "reviewed", "rejected")
_SOURCE_FUNDS_CONFIDENCE_LEVELS = ("exact", "strong", "weak", "unknown")
_SOURCE_FUNDS_ALLOCATION_POLICIES = ("explicit", "heuristic", "unknown")
_SOURCE_FUNDS_REVEAL_MODES = ("labels_only", "minimal", "standard", "full")
_SOURCE_FUNDS_REPORT_PURPOSES = ("existing_transaction", "planned_exchange_sale")
_TRANSFER_MATCH_METHODS = (
    "payment_hash",
    "provider_swap_id",
    "heuristic",
    "htlc_refund",
)
_TRANSFER_PAIR_KINDS = (
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
)
_LOAN_MARK_TYPES = (
    "collateral",
    "returned",
    "principal-received",
    "principal-repaid",
)
_DIRECT_PAYOUT_ASSETS = ("BTC", "LBTC", "LNBTC")
_CUSTODY_COMPONENT_TYPES = (
    "native_transfer",
    "channel_lifecycle",
    "peg",
    "swap",
    "refund",
    "manual_bridge",
)
_CUSTODY_LEG_ROLES = (
    "source",
    "destination",
    "fee",
    "external",
    "retained",
    "unresolved",
)
_EXACT_NONNEGATIVE_INTEGER_SCHEMA: dict[str, Any] = {
    "type": ["integer", "string"],
    "description": "Exact non-negative integer, using a decimal string outside JSON's safe integer range.",
}
_CUSTODY_COMPONENT_SPEC_SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "required": ["component_type", "legs"],
    "properties": {
        "component_type": {"type": "string", "enum": list(_CUSTODY_COMPONENT_TYPES)},
        "conservation_mode": {
            "type": "string",
            "enum": ["quantity", "conversion"],
            "description": "Defaults to quantity. Conversion requires separately reviewed conversion evidence.",
        },
        "evidence_kind": {"type": "string"},
        "evidence_grade": {"type": "string"},
        "notes": {"type": "string"},
        "change_reason": {"type": "string"},
        "conversion_policy": {"type": "string"},
        "conversion_metadata": {"type": "object"},
        "legs": {
            "type": "array",
            "minItems": 2,
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": ["id", "role", "amount_msat"],
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Stable component-local leg id used by explicit allocations.",
                    },
                    "role": {"type": "string", "enum": list(_CUSTODY_LEG_ROLES)},
                    "transaction": {
                        "type": "string",
                        "description": "Imported transaction id, txid, or external id anchoring this leg.",
                    },
                    "wallet": {"type": "string"},
                    "untracked_wallet": {
                        "type": "string",
                        "description": "Explicit label for a missing owned-wallet hop; never use for an external party.",
                    },
                    "amount_msat": dict(_EXACT_NONNEGATIVE_INTEGER_SCHEMA),
                    "valuation_unit": {
                        "type": "string",
                        "description": "Reviewed conversion valuation unit; supply together with valuation_amount.",
                    },
                    "valuation_amount": dict(_EXACT_NONNEGATIVE_INTEGER_SCHEMA),
                    "asset": {"type": "string"},
                    "rail": {"type": "string"},
                    "chain": {"type": "string"},
                    "network": {"type": "string"},
                    "exposure": {"type": "string"},
                    "conservation_unit": {"type": "string"},
                    "occurred_at": {"type": "string"},
                    "notes": {"type": "string"},
                },
            },
        },
        "allocations": {
            "type": "array",
            "description": "Required for genuine N:M; edges must cover each source and sink exactly once in aggregate.",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "source_leg_id",
                    "sink_leg_id",
                    "source_amount_msat",
                    "sink_amount_msat",
                ],
                "properties": {
                    "source_leg_id": {"type": "string"},
                    "sink_leg_id": {"type": "string"},
                    "source_amount_msat": dict(_EXACT_NONNEGATIVE_INTEGER_SCHEMA),
                    "sink_amount_msat": dict(_EXACT_NONNEGATIVE_INTEGER_SCHEMA),
                },
            },
        },
    },
}
_REVIEW_WORKLIST_CATEGORIES = (
    "readiness",
    "quarantine",
    "stale_edits",
    "transfers",
    "loans",
    "commercial",
    "source_funds",
)


_BASE_TOOL_CATALOG: tuple[ToolEntry, ...] = (
    ToolEntry(
        name="status",
        description="Read Kassiber runtime status, active data root, backend, and local auth mode.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        daemon_kind="status",
        summary_template="Read Kassiber status",
    ),
    ToolEntry(
        name="ui.overview.snapshot",
        description="Read the current dashboard overview snapshot for the active workspace/profile.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_overview_snapshot",
        daemon_kind="ui.overview.snapshot",
        summary_template="Read overview snapshot",
    ),
    ToolEntry(
        name="ui.transactions.list",
        description="Read a bounded list of recent transactions for the active profile.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Maximum number of transactions to return.",
                },
                "direction": {
                    "type": "string",
                    "enum": ["inbound", "outbound"],
                    "description": "Optional transaction direction filter.",
                },
                "asset": {
                    "type": "string",
                    "description": "Optional asset code filter, for example BTC or LBTC.",
                },
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label filter.",
                },
                "since": {
                    "type": "string",
                    "description": "Optional RFC3339 lower bound on occurred_at.",
                },
                "until": {
                    "type": "string",
                    "description": "Optional RFC3339 upper bound on occurred_at.",
                },
                "period": {
                    "type": "string",
                    "enum": [
                        "30days",
                        "3months",
                        "ytd",
                        "1year",
                        "5years",
                        "10years",
                        "15years",
                        "all",
                    ],
                    "description": "Optional relative period filter.",
                },
                "txids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Exact internal, external, or public explorer transaction ids.",
                },
                "status": {
                    "type": "string",
                    "enum": ["completed", "pending", "failed", "review"],
                    "description": "Optional review/status filter.",
                },
                "flow": {
                    "type": "string",
                    "enum": ["incoming", "outgoing", "transfer", "swap", "layer-transition"],
                    "description": "Optional UI flow filter.",
                },
                "payment_method": {
                    "type": "string",
                    "enum": ["On-chain", "Exchange", "Lightning", "Liquid"],
                    "description": "Optional payment method / network family filter.",
                },
                "network": {
                    "type": "string",
                    "description": "Optional wallet network/chain/payment method filter.",
                },
                "withFees": {
                    "type": "boolean",
                    "description": "Only return transactions with a non-zero fee.",
                },
                "quick": {
                    "type": "string",
                    "enum": [
                        "external_flow",
                        "review_queue",
                        "no_explorer_id",
                        "missing_price",
                        "failed_import",
                    ],
                    "description": "Optional desktop quick filter.",
                },
                "sort": {
                    "type": "string",
                    "enum": ["occurred-at", "amount", "fiat-value", "fee"],
                    "description": "Sort column applied before the limit.",
                },
                "order": {
                    "type": "string",
                    "enum": ["asc", "desc"],
                    "description": "Sort direction.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_list",
        daemon_kind="ui.transactions.list",
        summary_template="Read recent transactions",
    ),
    ToolEntry(
        name="ui.transactions.extremes",
        description=(
            "Read exact largest and smallest transactions by amount for the active "
            "profile, with sorting applied before the limit."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 20,
                    "description": "Maximum rows to return for each of largest and smallest.",
                },
                "direction": {
                    "type": "string",
                    "enum": ["inbound", "outbound"],
                    "description": "Optional transaction direction filter.",
                },
                "asset": {
                    "type": "string",
                    "description": "Optional asset code filter, for example BTC or LBTC.",
                },
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label filter.",
                },
                "since": {
                    "type": "string",
                    "description": "Optional RFC3339 lower bound on occurred_at.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_extremes",
        daemon_kind="ui.transactions.extremes",
        summary_template="Read transaction extremes",
    ),
    ToolEntry(
        name="ui.transactions.search",
        description=(
            "Search transactions by id, txid, wallet, note, description, counterparty, "
            "kind, or tag with safe bounded filters."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search text to match against transaction metadata.",
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum transactions to return.",
                },
                "direction": {
                    "type": "string",
                    "enum": ["inbound", "outbound"],
                    "description": "Optional transaction direction filter.",
                },
                "asset": {
                    "type": "string",
                    "description": "Optional asset code filter, for example BTC or LBTC.",
                },
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label filter.",
                },
                "since": {
                    "type": "string",
                    "description": "Optional RFC3339 lower bound on occurred_at.",
                },
                "until": {
                    "type": "string",
                    "description": "Optional RFC3339 upper bound on occurred_at.",
                },
                "sort": {
                    "type": "string",
                    "enum": ["occurred-at", "amount", "fiat-value", "fee"],
                    "description": "Sort column applied before the limit.",
                },
                "order": {
                    "type": "string",
                    "enum": ["asc", "desc"],
                    "description": "Sort direction.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_search",
        daemon_kind="ui.transactions.search",
        summary_template="Search transactions",
    ),
    ToolEntry(
        name="ui.wallets.list",
        description=(
            "Read configured wallets, labels, kinds, safe backend names/kinds, "
            "and transaction/sync status without descriptors or wallet config JSON."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_wallets_list",
        daemon_kind="ui.wallets.list",
        summary_template="Read wallets",
    ),
    ToolEntry(
        name="ui.wallets.utxos",
        description=(
            "Read one wallet's watch-only coin/UTXO inventory with outpoints, "
            "amounts, status, coarse branch metadata, and source freshness; "
            "never returns wallet addresses, derivation indices, descriptors, "
            "xpubs, blinding keys, backend URLs/tokens, raw wallet config, or "
            "raw wallet files."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["wallet"],
            "properties": {
                "wallet": {
                    "type": "string",
                    "description": "Wallet id or label to read coin inventory for.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_wallets_utxos",
        daemon_kind="ui.wallets.utxos",
        summary_template="Read wallet coins",
    ),
    ToolEntry(
        name="ui.wallets.ownership_coverage",
        description=(
            "Read safe watch-only ownership policy and backend history coverage "
            "tiers plus guided repair actions. Never returns descriptors, xpubs, "
            "addresses, scripts, derivation paths, backend URLs, or credentials."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label; omit for the profile.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_wallets_ownership_coverage",
        daemon_kind="ui.wallets.ownership_coverage",
        summary_template="Read ownership coverage",
    ),
    ToolEntry(
        name="ui.wallets.identify",
        description=(
            "Reconcile a list of addresses and/or transaction ids against the "
            "active profile's wallets: for each input, report whether it belongs "
            "to a wallet (naming the wallet) or is external/unknown, and classify "
            "each transaction as a self-transfer, outbound payment, or inbound "
            "receipt. Matches local data only (synced inventory, imported "
            "transactions, offline descriptor derivation) and does NOT contact "
            "the network. Receive-vs-change branch and derivation geometry are "
            "intentionally not exposed; never returns descriptors, xpubs, "
            "scriptPubKeys, derivation paths, address indices, blinding keys, "
            "backend URLs/tokens, or raw wallet config."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "addresses": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Addresses to check.",
                },
                "txids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Transaction ids to check.",
                },
                "text": {
                    "type": "string",
                    "description": "Free-form text with one address or txid per line.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_wallets_identify",
        daemon_kind="ui.wallets.identify",
        summary_template="Identify owners",
    ),
    ToolEntry(
        name="ui.backends.list",
        description=(
            "Read sync backends referenced by the active profile with coarse URL "
            "and credential presence flags; never returns exact URLs, tokens, "
            "cookies, auth headers, or config JSON."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_backends_list",
        daemon_kind="ui.backends.list",
        summary_template="Read backends",
    ),
    ToolEntry(
        name="ui.profiles.snapshot",
        description=(
            "After an explicit all-books request, read profiles in the current "
            "workspace and the active profile summary."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_profiles_snapshot",
        daemon_kind="ui.profiles.snapshot",
        summary_template="Read profiles snapshot",
    ),
    ToolEntry(
        name="ui.reports.capital_gains",
        description="Read the current capital gains report snapshot for the active profile.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_reports_capital_gains",
        daemon_kind="ui.reports.capital_gains",
        summary_template="Read capital gains snapshot",
    ),
    ToolEntry(
        name="ui.reports.summary",
        description=(
            "Read exact processed all-time summary totals for the active profile, "
            "including asset_flow and wallet_flow with BTC, sat, and msat amounts, "
            "plus reviewed transfer_pairs so flow answers can identify swaps or pegs."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label to scope the summary.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_reports_summary",
        daemon_kind="ui.reports.summary",
        summary_template="Read summary report",
    ),
    ToolEntry(
        name="ui.reports.balance_sheet",
        description=(
            "Read exact processed current holdings by reporting bucket/account for "
            "the active profile, including BTC, sat, msat, cost basis, and value."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_reports_balance_sheet",
        daemon_kind="ui.reports.balance_sheet",
        summary_template="Read balance sheet",
    ),
    ToolEntry(
        name="ui.reports.portfolio_summary",
        description=(
            "Read exact processed current holdings by wallet, including BTC, sat, "
            "msat, average cost, cost basis, market value, and unrealized PnL."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_reports_portfolio_summary",
        daemon_kind="ui.reports.portfolio_summary",
        summary_template="Read portfolio summary",
    ),
    ToolEntry(
        name="ui.reports.lightning_profitability",
        description=(
            "Read the aggregate routing-profitability summary for a Lightning "
            "connection: routing revenue, payment cost, rebalance cost, on-chain "
            "cost, net profit, and counts plus window label. AI variant is "
            "redacted per docs/reference/lightning-opsec.md Tier 3 — the "
            "connection id and per-channel covers-open-cost rows are omitted "
            "because per-channel peer aliases and short channel ids identify "
            "third parties. Requires a registered Lightning adapter; returns an "
            "error envelope when no LND/CLN sync is installed or the adapter "
            "does not support routing profitability."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["connection"],
            "properties": {
                "connection": {
                    "type": "string",
                    "description": (
                        "Lightning connection identifier (wallet id or label) to"
                        " report on."
                    ),
                },
                "window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 365,
                    "description": "Routing window in days (default 30).",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_reports_lightning_profitability",
        daemon_kind="ui.reports.lightning_profitability",
        summary_template="Read Lightning profitability",
    ),
    ToolEntry(
        name="ui.connections.node.snapshot",
        description=(
            "Read an operational Lightning node snapshot: channels with "
            "local/remote balances, peer count, on-chain balance, fee policies, "
            "and routing window summary. AI variant is redacted per "
            "docs/reference/lightning-opsec.md Tier 3 — operator pubkey, channel "
            "funding outpoints, short channel ids, peer pubkeys and peer aliases "
            "(including on forwards) are omitted; the operator's own connection "
            "label is kept. Returns an error envelope when no Lightning adapter "
            "is registered for the connection's kind or the adapter does not "
            "support node snapshots."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["connection"],
            "properties": {
                "connection": {
                    "type": "string",
                    "description": (
                        "Lightning connection identifier (wallet id or label)."
                    ),
                },
                "window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 365,
                    "description": "Routing/forwards window in days (default 30).",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_connections_node_snapshot",
        daemon_kind="ui.connections.node.snapshot",
        summary_template="Read Lightning node snapshot",
    ),
    ToolEntry(
        name="ui.reports.tax_summary",
        description=(
            "Read exact processed tax-summary rows by year and asset, including "
            "proceeds, cost basis, and gain/loss."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "year": {
                    "type": "integer",
                    "minimum": 1900,
                    "maximum": 9999,
                    "description": "Optional tax year to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_reports_tax_summary",
        daemon_kind="ui.reports.tax_summary",
        summary_template="Read tax summary",
    ),
    ToolEntry(
        name="ui.reports.balance_history",
        description=(
            "Read processed balance history for trends over time, with bounded "
            "interval and scope filters."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "interval": {
                    "type": "string",
                    "enum": ["hour", "day", "week", "month"],
                    "description": "Bucket interval; month is a good default.",
                },
                "start": {
                    "type": "string",
                    "description": "Optional RFC3339 lower bound.",
                },
                "end": {
                    "type": "string",
                    "description": "Optional RFC3339 upper bound.",
                },
                "wallet": {
                    "type": "string",
                    "description": "Optional wallet id or label filter.",
                },
                "account": {
                    "type": "string",
                    "description": "Optional account code or label filter.",
                },
                "asset": {
                    "type": "string",
                    "description": "Optional asset code filter.",
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Maximum latest buckets to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_reports_balance_history",
        daemon_kind="ui.reports.balance_history",
        summary_template="Read balance history",
    ),
    ToolEntry(
        name="ui.reports.privacy_hygiene",
        description=(
            "Read the active profile's redacted privacy-hygiene payload: local-only "
            "counts, endpoint/AI posture, storage and wallet metadata findings, "
            "limitations, and evidence_level on each finding. The payload omits "
            "addresses, scripts, descriptors, xpubs, backend URLs/tokens, wallet "
            "config, raw_json, branch/index values, and derivation paths."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_reports_privacy_hygiene",
        daemon_kind="ui.reports.privacy_hygiene",
        summary_template="Read privacy hygiene",
    ),
    ToolEntry(
        name="ui.reports.privacy_mirror",
        description=(
            "Read the active profile's redacted Privacy Mirror payload and its "
            "precomputed worst-risk answer. Use this for questions such as what "
            "is linkable, who can infer it, what proves it, what is unknown, or "
            "what future PSBT/what-if analysis would worsen. The payload is "
            "local-only, advisory-only, read-only, and omits addresses, scripts, "
            "descriptors, xpubs, backend URLs/tokens, wallet config, raw_json, "
            "branch/index values, derivation paths, and raw PSBT bytes."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_reports_privacy_mirror",
        daemon_kind="ui.reports.privacy_mirror",
        summary_template="Read Privacy Mirror",
    ),
    ToolEntry(
        name="ui.journals.snapshot",
        description=(
            "Read journal processing status, recent journal rows, quarantine "
            "summary, and reviewed pair context for recent swap/peg rows."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_journals_snapshot",
        daemon_kind="ui.journals.snapshot",
        summary_template="Read journals snapshot",
    ),
    ToolEntry(
        name="ui.journals.quarantine",
        description=(
            "Read quarantine counts and a bounded recent list of quarantined transactions. "
            "Before resolving an item, read ui.transactions.review_context and, for ownership "
            "or swap gaps, ui.transfers.review_context."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum quarantined items to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_journals_quarantine",
        daemon_kind="ui.journals.quarantine",
        summary_template="Read journal quarantine",
    ),
    ToolEntry(
        name="ui.journals.quarantine.resolve",
        description=(
            "Resolve one currently quarantined transaction after explicit consent, then "
            "reprocess journals by default and report whether it actually cleared. "
            "Use price_override only with user-reviewed pricing evidence; never invent a "
            "rate or fiat value. Use exclude only when the user explicitly confirms the "
            "transaction is outside the book. Ownership, transfer, swap, missing-wallet, "
            "and N:M quarantines must be repaired with the transfer/custody tools instead."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction", "action", "reason"],
            "properties": {
                "transaction": {
                    "type": "string",
                    "description": "Internal transaction id or public txid currently in quarantine.",
                },
                "action": {
                    "type": "string",
                    "enum": ["price_override", "exclude"],
                },
                "fiat_rate": {
                    "type": ["string", "number"],
                    "description": (
                        "Reviewed fiat price per BTC-style unit for price_override; "
                        "do not also provide fiat_value."
                    ),
                },
                "fiat_value": {
                    "type": ["string", "number"],
                    "description": (
                        "Reviewed total fiat value for price_override; do not also provide fiat_rate."
                    ),
                },
                "reason": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Audit reason stored in transaction edit history.",
                },
                "reprocess": {
                    "type": "boolean",
                    "description": "Rebuild journals and verify the result. Defaults to true.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_journals_quarantine_resolve",
        daemon_kind="ui.journals.quarantine.resolve",
        summary_template="Resolve quarantined transaction",
    ),
    ToolEntry(
        name="ui.journals.events.list",
        description=(
            "Read bounded processed journal events, including transaction ids, "
            "Austrian category fields, and reviewed pair context for swap/peg rows."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "transaction": {
                    "type": "string",
                    "description": "Optional transaction id or txid filter.",
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Maximum journal events to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_journals_events_list",
        daemon_kind="ui.journals.events.list",
        summary_template="Read journal events",
    ),
    ToolEntry(
        name="ui.journals.transfers.list",
        description=(
            "Read bounded transfer-pair audit data and transfer entry counts "
            "without changing journal state."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum transfer pairs to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_journals_transfers_list",
        daemon_kind="ui.journals.transfers.list",
        summary_template="Read transfer audit",
    ),
    ToolEntry(
        name="ui.rates.summary",
        description="Read cached rate pairs and latest local rate metadata; does not sync the network.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_rates_summary",
        daemon_kind="ui.rates.summary",
        summary_template="Read rates summary",
    ),
    ToolEntry(
        name="ui.rates.coverage",
        description=(
            "Read transaction pricing coverage for the active profile, including "
            "missing fiat price rows and whether the local rates cache can cover them."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum missing-price transactions to return.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_rates_coverage",
        daemon_kind="ui.rates.coverage",
        summary_template="Read rate coverage",
    ),
    ToolEntry(
        name="ui.rates.rebuild",
        description=(
            "After explicit consent, fetch missing provider spot-rate windows, "
            "clear provider-derived transaction prices, and reprocess journals."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "source": {
                    "type": "string",
                    "enum": ["coinbase-exchange", "coingecko"],
                    "description": "Provider to fetch from. Defaults to the configured market-rate provider.",
                },
                "pair": {
                    "type": "string",
                    "description": "Optional pair such as BTC-EUR. Omit to cover supported pairs.",
                },
                "days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Fallback continuous range when no transactions need prices.",
                },
                "reprice_transactions": {
                    "type": "boolean",
                    "description": "Clear provider-derived transaction prices and reprocess journals. Defaults to true.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_rates_rebuild",
        daemon_kind="ui.rates.rebuild",
        summary_template="Fetch spot prices",
    ),
    ToolEntry(
        name="ui.report.blockers",
        description=(
            "Read deterministic report-readiness blockers: missing workspace/profile, "
            "wallets, transactions, stale journals, quarantine, and pricing coverage."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_report_blockers",
        daemon_kind="ui.report.blockers",
        summary_template="Read report blockers",
    ),
    ToolEntry(
        name="ui.audit.changes_since_last_answer",
        description=(
            "Read whether transactions, wallets, journals, quarantines, or rates "
            "or metadata edit history changed since an optional prior answer timestamp."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "since": {
                    "type": "string",
                    "description": "Optional RFC3339 timestamp from a previous answer provenance record.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_audit_changes_since_last_answer",
        daemon_kind="ui.audit.changes_since_last_answer",
        summary_template="Read changes since last answer",
    ),
    ToolEntry(
        name="ui.transactions.history",
        description=(
            "Read bounded append-only metadata edit history for one transaction, "
            "including source, reason, changed fields, and redacted before/after values."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction"],
            "properties": {
                "transaction": {"type": "string", "description": "Transaction id or external id."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20},
                "source": {"type": "string", "enum": ["cli", "gui", "ai_tool"]},
                "field_family": {"type": "string", "enum": ["metadata", "tax", "pricing"]},
                "pricing_only": {"type": "boolean"},
                "ai_only": {"type": "boolean"},
                "stale_only": {"type": "boolean"},
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_history",
        daemon_kind="ui.transactions.history",
        summary_template="Read transaction edit history",
    ),
    ToolEntry(
        name="ui.activity.history",
        description=(
            "Read bounded global Activity history for transaction metadata edits "
            "with safe filters for source, field family, wallet, transaction, pricing, and AI changes."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "transaction": {"type": "string"},
                "wallet": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50, "default": 20},
                "source": {"type": "string", "enum": ["cli", "gui", "ai_tool"]},
                "field_family": {"type": "string", "enum": ["metadata", "tax", "pricing"]},
                "pricing_only": {"type": "boolean"},
                "ai_only": {"type": "boolean"},
                "stale_only": {"type": "boolean"},
                "start": {"type": "string"},
                "end": {"type": "string"},
            },
        },
        kind_class="read_only",
        wire_name="ui_activity_history",
        daemon_kind="ui.activity.history",
        summary_template="Read metadata Activity history",
    ),
    ToolEntry(
        name="ui.maintenance.settings",
        description=(
            "Read daemon freshness policy and source/job state for the active profile, "
            "including whether report-read tools may run opted-in refresh jobs."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_maintenance_settings",
        daemon_kind="ui.maintenance.settings",
        summary_template="Read maintenance settings",
    ),
    ToolEntry(
        name="ui.workspace.health",
        description=(
            "Read active workspace/profile health: wallet and transaction counts, "
            "journal freshness, quarantine count, and report-readiness hints."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_workspace_health",
        daemon_kind="ui.workspace.health",
        summary_template="Read workspace health",
    ),
    ToolEntry(
        name="ui.next_actions",
        description=(
            "Read structured recommended next actions for the active workspace. "
            "This only advises; it never runs the actions."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_next_actions",
        daemon_kind="ui.next_actions",
        summary_template="Read next actions",
    ),
    ToolEntry(
        name="ui.source_funds.sources.list",
        description=(
            "Read reviewed source-funds root sources for the active profile. "
            "No descriptors, wallet files, raw evidence URLs, stored attachment "
            "paths, or secrets are returned."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_source_funds_sources_list",
        daemon_kind="ui.source_funds.sources.list",
        summary_template="Read source-funds sources",
    ),
    ToolEntry(
        name="ui.source_funds.links.list",
        description=(
            "Read reviewed/suggested source-funds links, optionally scoped to one "
            "target transaction. Raw evidence URLs and stored attachment paths are "
            "redacted. Use this before adding or reviewing provenance."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "target_transaction": {
                    "type": "string",
                    "description": "Optional transaction id or txid to scope the link list.",
                },
                "state": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_LINK_STATES),
                    "description": "Optional link review-state filter.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_source_funds_links_list",
        daemon_kind="ui.source_funds.links.list",
        summary_template="Read source-funds links",
    ),
    ToolEntry(
        name="ui.source_funds.preview",
        description=(
            "Preview a source-funds path and export gates for one target transaction. "
            "This is read-only and surfaces blockers such as missing history, "
            "heuristic allocations, privacy-hop ambiguity, or missing pricing."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["target_transaction"],
            "properties": {
                "target_transaction": {
                    "type": "string",
                    "description": "Target transaction id or txid.",
                },
                "target_amount": {
                    "type": "string",
                    "description": "Optional BTC amount of the target to trace.",
                },
                "report_purpose": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_REPORT_PURPOSES),
                    "description": "Purpose of the source-funds preview.",
                },
                "planned_destination": {"type": "string"},
                "planned_note": {"type": "string"},
                "reveal_mode": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_REVEAL_MODES),
                },
                "max_depth": {"type": "integer", "minimum": 1, "maximum": 32},
                "recipient": {"type": "string"},
            },
        },
        kind_class="read_only",
        wire_name="ui_source_funds_preview",
        daemon_kind="ui.source_funds.preview",
        summary_template="Preview source-funds path",
    ),
    ToolEntry(
        name="read_skill_reference",
        description=(
            "Read one compact Kassiber skill reference by allowlisted name when more "
            "workflow detail is needed."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["name"],
            "properties": {
                "name": {
                    "type": "string",
                    "enum": list(SKILL_REFERENCE_NAMES),
                    "description": "Reference file name without the .md suffix.",
                },
            },
        },
        kind_class="read_only",
        summary_template="Read Kassiber skill reference",
    ),
    ToolEntry(
        name="ui.wallets.sync",
        description=(
            "Refresh configured watch-only sources after the user explicitly "
            "allows this mutating action."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "wallet": {"type": "string"},
                "all": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_wallets_sync",
        daemon_kind="ui.wallets.sync",
        summary_template="Refresh watch-only sources",
    ),
    ToolEntry(
        name="ui.journals.process",
        description=(
            "Rebuild journals for the active profile after the user explicitly "
            "allows this mutating action, so reports can use fresh processed data."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="mutating",
        wire_name="ui_journals_process",
        daemon_kind="ui.journals.process",
        summary_template="Process journals",
    ),
    ToolEntry(
        name="ui.maintenance.configure",
        description=(
            "Change daemon freshness policy after explicit consent. The legacy "
            "auto_sync_before_report_reads flag maps to report_read_sync plus source-class opt-ins."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "auto_sync_before_report_reads": {
                    "type": "boolean",
                    "description": (
                        "Legacy compatibility flag. When true, report/read tools may "
                        "refresh opted-in configured sources before refreshing journals."
                    ),
                },
                "report_read_sync": {
                    "type": "boolean",
                    "description": "Allow report/read tools to run opted-in freshness jobs before local journal refresh.",
                },
                "background_enabled": {
                    "type": "boolean",
                    "description": "Allow daemon-owned background refresh while the app/daemon is running.",
                },
                "bitcoin_rail_carrying_value": {
                    "type": "boolean",
                    "description": "When true, BTC/LBTC rail-change suggestions default to carrying-value treatment.",
                },
                "source_classes": {
                    "type": "object",
                    "additionalProperties": {"type": "boolean"},
                    "description": "Per-source-class opt-ins such as onchain_wallet, btcpay_wallet, btcpay_provenance, market_rates, and journals.",
                },
                "market_rate_provider": {
                    "type": "string",
                    "enum": ["coinbase-exchange", "coingecko"],
                    "description": "Default live market-rate provider for automatic price refresh and default rate rebuilds.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_maintenance_configure",
        daemon_kind="ui.maintenance.configure",
        summary_template="Configure AI maintenance",
    ),
    ToolEntry(
        name="ui.maintenance.run",
        description=(
            "Run local maintenance after explicit consent: optional watch-only refresh "
            "and journal processing, then return report blockers."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "sync": {
                    "type": "string",
                    "enum": ["never", "if_enabled", "always"],
                    "description": (
                        "Use never for local-only journal refresh, if_enabled to "
                        "respect settings, or always for an explicit sync request."
                    ),
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_maintenance_run",
        daemon_kind="ui.maintenance.run",
        summary_template="Run AI maintenance",
    ),
    ToolEntry(
        name="ui.source_funds.sources.create",
        description=(
            "Create a reviewed source-funds root source after explicit consent. "
            "Use for user-attested acquisitions, exchange withdrawals, income, "
            "opening-balance attestations, or known missing-history stops."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["source_type", "label"],
            "properties": {
                "source_type": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_SOURCE_TYPES),
                },
                "label": {"type": "string"},
                "asset": {"type": "string", "description": "Asset code, defaults to BTC."},
                "amount": {"type": "string", "description": "Optional BTC amount."},
                "fiat_currency": {"type": "string"},
                "fiat_value": {"type": "string"},
                "acquired_at": {"type": "string", "description": "Optional RFC3339 timestamp."},
                "description": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_sources_create",
        daemon_kind="ui.source_funds.sources.create",
        summary_template="Create source-funds source",
    ),
    ToolEntry(
        name="ui.source_funds.links.create",
        description=(
            "Create a reviewed or suggested source-funds link after explicit consent. "
            "Provide exactly one of from_transaction or from_source. Supports "
            "self-transfers, exchange transfers, trades, swaps, pegs, Lightning "
            "hops, CoinJoin/PayJoin privacy hops, manual roots, and missing-history "
            "edges; never claim exact provenance for heuristic privacy links."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["to_transaction", "link_type", "allocation_amount", "explanation"],
            "properties": {
                "from_transaction": {
                    "type": "string",
                    "description": "Parent transaction id or txid. Mutually exclusive with from_source.",
                },
                "from_source": {
                    "type": "string",
                    "description": "Source-funds source id or label. Mutually exclusive with from_transaction.",
                },
                "to_transaction": {"type": "string", "description": "Target transaction id or txid."},
                "link_type": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_LINK_TYPES),
                },
                "state": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_LINK_STATES),
                    "description": "Defaults to reviewed.",
                },
                "confidence": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_CONFIDENCE_LEVELS),
                    "description": "Use weak/unknown for heuristic or privacy-boundary links.",
                },
                "method": {"type": "string", "description": "Evidence method label, default manual."},
                "asset": {"type": "string"},
                "allocation_amount": {"type": "string", "description": "BTC amount allocated to the target."},
                "from_asset": {"type": "string"},
                "from_allocation_amount": {
                    "type": "string",
                    "description": "Optional BTC amount consumed from the parent/source.",
                },
                "allocation_policy": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_ALLOCATION_POLICIES),
                    "description": "Use heuristic unless the allocation is explicitly reviewed.",
                },
                "explanation": {"type": "string"},
                "uses_chain_observation": {"type": "boolean"},
                "chain_data_confirmed": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_links_create",
        daemon_kind="ui.source_funds.links.create",
        summary_template="Create source-funds link",
    ),
    ToolEntry(
        name="ui.source_funds.links.review",
        description=(
            "Update review state, confidence, allocation, or explanation for an "
            "existing source-funds link after explicit consent. Use this to accept, "
            "reject, or downgrade suggested provenance without changing tax pairs."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["link"],
            "properties": {
                "link": {"type": "string", "description": "Source-funds link id."},
                "state": {"type": "string", "enum": list(_SOURCE_FUNDS_LINK_STATES)},
                "link_type": {"type": "string", "enum": list(_SOURCE_FUNDS_LINK_TYPES)},
                "confidence": {"type": "string", "enum": list(_SOURCE_FUNDS_CONFIDENCE_LEVELS)},
                "allocation_amount": {"type": "string"},
                "from_allocation_amount": {"type": "string"},
                "allocation_policy": {
                    "type": "string",
                    "enum": list(_SOURCE_FUNDS_ALLOCATION_POLICIES),
                },
                "explanation": {"type": "string"},
                "uses_chain_observation": {"type": "boolean"},
                "chain_data_confirmed": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_links_review",
        daemon_kind="ui.source_funds.links.review",
        summary_template="Review source-funds link",
    ),
    ToolEntry(
        name="ui.source_funds.suggest",
        description=(
            "Seed source-funds link suggestions after explicit consent. Deterministic "
            "same-txid, reviewed transaction-pair, provider-id, and Samourai "
            "Whirlpool boundary suggestions can cover non-CoinJoin and CoinJoin-like "
            "flows; broad time/amount hints are disabled unless explicitly requested."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "target_transaction": {
                    "type": "string",
                    "description": "Optional target transaction id or txid to keep suggestions scoped.",
                },
                "include_broad_hints": {
                    "type": "boolean",
                    "description": "Include weak same-day time/amount hints. Defaults to false.",
                },
                "max_suggestions": {"type": "integer", "minimum": 1, "maximum": 500},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_suggest",
        daemon_kind="ui.source_funds.suggest",
        summary_template="Seed source-funds suggestions",
    ),
    ToolEntry(
        name="ui.source_funds.links.bulk_review",
        description=(
            "Accept only deterministic source-funds suggestions for one target after "
            "explicit consent. Weak time/amount hints, broad provider ids, and chain "
            "observations remain manual review items."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["target_transaction"],
            "properties": {
                "target_transaction": {
                    "type": "string",
                    "description": "Target transaction id or txid whose deterministic suggestions should be reviewed.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_links_bulk_review",
        daemon_kind="ui.source_funds.links.bulk_review",
        summary_template="Review deterministic source-funds links",
    ),
    ToolEntry(
        name="ui.transfers.suggest",
        description=(
            "Read transfer/swap candidate pairings the matcher infers from unpaired "
            "transactions. Surfaces exact deterministic candidates (payment_hash, "
            "provider_swap_id, htlc_refund) and strong time + amount heuristic "
            "candidates with computed fee deltas and conflict cluster ids. No DB writes."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "confidence": {
                    "type": "string",
                    "enum": ["exact", "strong"],
                    "description": "Optional filter pinning to one confidence band.",
                },
                "method": {
                    "type": "string",
                    "enum": list(_TRANSFER_MATCH_METHODS),
                    "description": "Optional filter pinning to one match method.",
                },
                "asset_pair": {
                    "type": "string",
                    "description": "OUT-IN asset shape, e.g. 'LBTC-BTC' for a peg-out.",
                },
                "candidate_type": {
                    "type": "string",
                    "enum": ["transfer", "swap"],
                    "description": "Optional filter for Bitcoin movements or other cross-asset swaps.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_transfers_suggest",
        daemon_kind="ui.transfers.suggest",
        summary_template="Read transfer/swap candidates",
    ),
    ToolEntry(
        name="ui.transfers.review_context",
        description=(
            "Read a deterministic pair-review packet for the active profile: "
            "candidate legs, confidence reasons, fee assessment, conflict "
            "status, metadata clues, journal impact if left unpaired, active "
            "pairs, rules, and saved review views. No DB writes."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Maximum candidate review items to return.",
                },
                "confidence": {
                    "type": "string",
                    "enum": ["exact", "strong"],
                    "description": "Optional filter pinning to one confidence band.",
                },
                "method": {
                    "type": "string",
                    "enum": list(_TRANSFER_MATCH_METHODS),
                    "description": "Optional filter pinning to one match method.",
                },
                "asset_pair": {
                    "type": "string",
                    "description": "OUT-IN asset shape, e.g. 'LBTC-BTC'.",
                },
                "route_pair": {
                    "type": "string",
                    "description": "Rail-aware OUT-IN route, e.g. 'LNBTC-LBTC'.",
                },
                "candidate_type": {
                    "type": "string",
                    "enum": ["transfer", "swap"],
                    "description": "Optional filter for Bitcoin movements or other cross-asset swaps.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_transfers_review_context",
        daemon_kind="ui.transfers.review_context",
        summary_template="Read swap review context",
    ),
    ToolEntry(
        name="ui.transfers.list",
        description=(
            "Read active reviewed transfer/swap pairs (soft-deleted excluded) with their "
            "computed swap_fee_msat, kind, policy, pair_source, and confidence."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_transfers_list",
        daemon_kind="ui.transfers.list",
        summary_template="Read swap pairs",
    ),
    ToolEntry(
        name="ui.transfers.payouts.list",
        description=(
            "Read reviewed direct/split swap payouts where Kassiber knows the outbound "
            "transaction but there is no imported inbound payout leg."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_transfers_payouts_list",
        daemon_kind="ui.transfers.payouts.list",
        summary_template="Read direct swap payouts",
    ),
    ToolEntry(
        name="ui.transfers.components.list",
        description=(
            "Read privacy-safe custody components for the active profile. Use this before "
            "authoring a missing-wallet, 1:N, N:1, N:M, Liquid, or Lightning quarantine "
            "repair so the AI does not duplicate an existing reviewed interpretation."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "state": {
                    "type": "string",
                    "enum": ["draft", "active", "superseded"],
                },
                "component_type": {
                    "type": "string",
                    "enum": list(_CUSTODY_COMPONENT_TYPES),
                },
                "transaction": {"type": "string"},
                "effective_only": {"type": "boolean"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
            },
        },
        kind_class="read_only",
        wire_name="ui_transfers_components_list",
        daemon_kind="ui.transfers.components.list",
        summary_template="Read custody components",
    ),
    ToolEntry(
        name="ui.transfers.components.bulk_resolve",
        description=(
            "Validate or atomically author one or more reviewed custody components after "
            "explicit consent. This is the quarantine repair path for missing wallets and "
            "1:N, N:1, or explicit N:M custody flows that cannot be represented by one pair. "
            "Prefer dry_run=true first. Activation remains fail-closed: every anchor, amount, "
            "fee, network, asset, chronology, and allocation must conserve exactly. The result "
            "omits local-only evidence and location references."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["components"],
            "properties": {
                "components": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 50,
                    "items": _CUSTODY_COMPONENT_SPEC_SCHEMA,
                    "description": "Custody component specifications using the documented typed leg/allocation contract.",
                },
                "activate": {
                    "type": "boolean",
                    "description": (
                        "Activate complete quantity-conserving components. Defaults to true. "
                        "Conversion components must remain drafts for separate human review."
                    ),
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Validate and return an exact preview without writing. Defaults to false.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_components_bulk_resolve",
        daemon_kind="ui.transfers.components.bulk_resolve",
        summary_template="Resolve custody gaps in bulk",
    ),
    ToolEntry(
        name="ui.transfers.payouts.create",
        description=(
            "Record a reviewed direct or split swap payout after explicit consent. "
            "Use only when the outbound transaction is known but the payout leg is not imported."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["tx_out", "payout_asset", "payout_amount"],
            "properties": {
                "tx_out": {"type": "string"},
                "payout_asset": {"type": "string", "enum": list(_DIRECT_PAYOUT_ASSETS)},
                "payout_amount": {
                    "type": "string",
                    "description": "Positive payout amount in whole BTC-style asset units.",
                },
                "out_amount": {
                    "type": "string",
                    "description": "Optional portion of the outbound used by this payout.",
                },
                "policy": {"type": "string", "enum": ["carrying-value", "taxable"]},
                "payout_occurred_at": {"type": "string"},
                "payout_fiat_value": {
                    "type": ["string", "number"],
                    "description": "Optional reviewed fiat proceeds for a taxable payout.",
                },
                "payout_external_id": {
                    "type": "string",
                    "description": "Optional public/provider payout reference.",
                },
                "counterparty": {"type": "string"},
                "notes": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_payouts_create",
        daemon_kind="ui.transfers.payouts.create",
        summary_template="Create direct swap payout",
    ),
    ToolEntry(
        name="ui.transfers.payouts.delete",
        description="Remove one reviewed direct payout after explicit consent while retaining its audit history.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["payout_id"],
            "properties": {"payout_id": {"type": "string"}},
        },
        kind_class="mutating",
        wire_name="ui_transfers_payouts_delete",
        daemon_kind="ui.transfers.payouts.delete",
        summary_template="Delete direct swap payout",
    ),
    ToolEntry(
        name="ui.transfers.update",
        description="Correct the kind, policy, or notes on one reviewed transfer pair after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["pair_id"],
            "properties": {
                "pair_id": {"type": "string"},
                "kind": {"type": "string", "enum": list(_TRANSFER_PAIR_KINDS)},
                "policy": {"type": "string", "enum": ["carrying-value", "taxable"]},
                "notes": {"type": ["string", "null"]},
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_update",
        daemon_kind="ui.transfers.update",
        summary_template="Update reviewed transfer pair",
    ),
    ToolEntry(
        name="ui.transfers.rules.list",
        description=(
            "Read the active profile's swap auto-pair rules — predicates, "
            "default kind/policy, and enabled state. No DB writes."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_transfers_rules_list",
        daemon_kind="ui.transfers.rules.list",
        summary_template="Read swap rules",
    ),
    ToolEntry(
        name="ui.saved_views.list",
        description=(
            "Read saved review-queue filters for the active profile, "
            "optionally scoped to one surface (e.g. 'swap_candidates')."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "surface": {
                    "type": "string",
                    "description": "Optional surface label to filter on.",
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_saved_views_list",
        daemon_kind="ui.saved_views.list",
        summary_template="Read saved views",
    ),
    ToolEntry(
        name="ui.transfers.pair",
        description=(
            "Pair one outbound + one inbound transaction after explicit "
            "consent. Use kind='coinjoin' or kind='whirlpool' for a reviewed "
            "same-asset Coinjoin ownership hop. Computes swap_fee_msat at pair "
            "time for swap-like pairs and invalidates the journal so the next "
            "report read reflects the change."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["tx_out", "tx_in"],
            "properties": {
                "tx_out": {"type": "string", "description": "Outbound transaction id."},
                "tx_in": {"type": "string", "description": "Inbound transaction id."},
                "kind": {
                    "type": "string",
                    "enum": list(_TRANSFER_PAIR_KINDS),
                },
                "policy": {
                    "type": "string",
                    "enum": ["carrying-value", "taxable"],
                },
                "notes": {"type": "string"},
                "out_amount": {
                    "type": "string",
                    "description": (
                        "Optional BTC amount from the outbound used by the cross-asset "
                        "swap; the remainder can resolve as a same-asset self-transfer."
                    ),
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_pair",
        daemon_kind="ui.transfers.pair",
        summary_template="Pair transfer legs",
    ),
    ToolEntry(
        name="ui.transfers.unpair",
        description=(
            "Soft-delete one swap pair after explicit consent. Sets "
            "deleted_at so the audit row survives and the legs are immediately "
            "eligible for re-pairing."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["pair_id"],
            "properties": {
                "pair_id": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_unpair",
        daemon_kind="ui.transfers.unpair",
        summary_template="Unpair swap legs",
    ),
    ToolEntry(
        name="ui.transfers.bulk_pair",
        description=(
            "Auto-pair every solo (non-conflicted) candidate at or above the "
            "chosen confidence after explicit consent. Defaults to 'exact' "
            "so only deterministic links auto-apply without further review."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "confidence": {
                    "type": "string",
                    "enum": ["exact", "strong"],
                    "description": "Minimum confidence to auto-pair. Default 'exact'.",
                },
                "candidate_type": {
                    "type": "string",
                    "enum": ["transfer", "swap"],
                    "description": "Optional filter for Bitcoin movements or other cross-asset swaps.",
                },
                "method": {
                    "type": "string",
                    "enum": list(_TRANSFER_MATCH_METHODS),
                    "description": "Optional filter pinning to one match method.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_bulk_pair",
        daemon_kind="ui.transfers.bulk_pair",
        summary_template="Bulk-pair transfer/swap candidates",
    ),
    ToolEntry(
        name="ui.transfers.dismiss",
        description=(
            "Record a dismissal so the matcher stops "
            "suggesting this exact pair. Default expiry 90 days."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["tx_out", "tx_in"],
            "properties": {
                "tx_out": {"type": "string"},
                "tx_in": {"type": "string"},
                "reason": {"type": "string"},
                "expires_in_days": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "0 = never expire.",
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_dismiss",
        daemon_kind="ui.transfers.dismiss",
        summary_template="Dismiss swap candidate",
    ),
    ToolEntry(
        name="ui.transfers.rules.create",
        description=(
            "Create one swap auto-pair rule after explicit consent. The "
            "predicate is a JSON object whose non-empty fields constrain "
            "which candidates auto-pair (out_wallet_id, in_wallet_id, "
            "out_wallet_kind, in_wallet_kind, out_asset, in_asset, "
            "max_fee_pct, min_confidence)."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["kind", "policy"],
            "properties": {
                "name": {"type": "string"},
                "predicate": {"type": "object"},
                "kind": {
                    "type": "string",
                    "enum": list(_TRANSFER_PAIR_KINDS),
                },
                "policy": {
                    "type": "string",
                    "enum": ["carrying-value", "taxable"],
                },
                "enabled": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_rules_create",
        daemon_kind="ui.transfers.rules.create",
        summary_template="Create swap rule",
    ),
    ToolEntry(
        name="ui.transfers.rules.delete",
        description="Delete one swap auto-pair rule after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["rule_id"],
            "properties": {"rule_id": {"type": "string"}},
        },
        kind_class="mutating",
        wire_name="ui_transfers_rules_delete",
        daemon_kind="ui.transfers.rules.delete",
        summary_template="Delete swap rule",
    ),
    ToolEntry(
        name="ui.transfers.rules.set_enabled",
        description="Enable or disable one swap auto-pair rule after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["rule_id", "enabled"],
            "properties": {
                "rule_id": {"type": "string"},
                "enabled": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transfers_rules_set_enabled",
        daemon_kind="ui.transfers.rules.set_enabled",
        summary_template="Toggle swap rule",
    ),
    ToolEntry(
        name="ui.transfers.rules.apply",
        description=(
            "Apply all currently enabled swap auto-pair rules to non-conflicted "
            "candidates after explicit consent. Writes reviewed pairs with "
            "pair_source='rule_auto'."
        ),
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="mutating",
        wire_name="ui_transfers_rules_apply",
        daemon_kind="ui.transfers.rules.apply",
        summary_template="Apply swap rules",
    ),
    ToolEntry(
        name="ui.saved_views.create",
        description=(
            "Save a named filter for one review surface (e.g. 'swap_candidates') "
            "after explicit consent. The filter payload is opaque to the daemon."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["surface", "name"],
            "properties": {
                "surface": {"type": "string"},
                "name": {"type": "string"},
                "filter": {"type": "object"},
            },
        },
        kind_class="mutating",
        wire_name="ui_saved_views_create",
        daemon_kind="ui.saved_views.create",
        summary_template="Save filter view",
    ),
    ToolEntry(
        name="ui.saved_views.delete",
        description="Delete one saved view after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["view_id"],
            "properties": {"view_id": {"type": "string"}},
        },
        kind_class="mutating",
        wire_name="ui_saved_views_delete",
        daemon_kind="ui.saved_views.delete",
        summary_template="Delete saved view",
    ),
)


_TRANSACTION_REF_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["transaction"],
    "properties": {"transaction": {"type": "string"}},
}

_EXPANDED_TOOL_CATALOG: tuple[ToolEntry, ...] = (
    ToolEntry(
        name="ui.workspace.overview.snapshot",
        description=(
            "Read an explicit book-set treasury overview across all books in one workspace. "
            "Preserves per-book boundaries and never sums mixed fiat currencies."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["workspace_id"],
            "properties": {"workspace_id": {"type": "string"}},
        },
        kind_class="read_only",
        wire_name="ui_workspace_overview_snapshot",
        daemon_kind="ui.workspace.overview.snapshot",
        summary_template="Read book-set treasury overview",
    ),
    ToolEntry(
        name="ui.transactions.resolve",
        description="Resolve one internal transaction id or public txid to a safe transaction display row.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["query"],
            "properties": {"query": {"type": "string"}},
        },
        kind_class="read_only",
        wire_name="ui_transactions_resolve",
        daemon_kind="ui.transactions.resolve",
        summary_template="Resolve transaction",
    ),
    ToolEntry(
        name="ui.transactions.graph",
        description=(
            "Read one transaction's safe local graph, ownership annotations, fee metadata, "
            "and reviewed route context. Never returns descriptors, xpubs, backend URLs, or raw JSON."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction"],
            "properties": {"transaction": {"type": "string"}},
        },
        kind_class="read_only",
        wire_name="ui_transactions_graph",
        daemon_kind="ui.transactions.graph",
        summary_template="Read transaction graph",
    ),
    ToolEntry(
        name="ui.transactions.review_context",
        description=(
            "Read one bounded transaction-review packet combining the safe transaction row, graph, "
            "journal and transfer impact, pricing, edit history, evidence readiness, commercial "
            "provenance, source-funds links, privacy findings, and deterministic next actions."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction"],
            "properties": {
                "transaction": {"type": "string"},
                "history_limit": {"type": "integer", "minimum": 1, "maximum": 50},
                "include_graph": {"type": "boolean"},
                "include_privacy": {"type": "boolean"},
                "include_evidence": {"type": "boolean"},
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_review_context",
        daemon_kind="ui.transactions.review_context",
        summary_template="Read transaction review context",
    ),
    ToolEntry(
        name="ui.activity.stale",
        description="Read the metadata edits that make processed journals stale.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_activity_stale",
        daemon_kind="ui.activity.stale",
        summary_template="Read stale edit summary",
    ),
    ToolEntry(
        name="ui.attachments.list",
        description=(
            "Read attachment labels and evidence metadata, optionally for one transaction. "
            "Raw URLs, local paths, and stored relative paths are removed for AI."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "transaction": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
                "cursor": {"type": "string"},
            },
        },
        kind_class="read_only",
        wire_name="ui_attachments_list",
        daemon_kind="ui.attachments.list",
        summary_template="Read evidence attachments",
    ),
    ToolEntry(
        name="ui.audit.evidence.summary",
        description=(
            "Read deterministic audit-evidence readiness for the active profile, selected transactions, "
            "or one saved source-funds case. File paths and URL targets are not returned."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "transaction": {"type": "string"},
                "transactions": {"type": "array", "items": {"type": "string"}, "maxItems": 100},
                "source_funds_case": {"type": "string"},
                "include_journal_state": {"type": "boolean"},
                "include_review_state": {"type": "boolean"},
                "include_edit_history": {"type": "boolean"},
            },
        },
        kind_class="read_only",
        wire_name="ui_audit_evidence_summary",
        daemon_kind="ui.audit.evidence.summary",
        summary_template="Read audit evidence readiness",
    ),
    ToolEntry(
        name="ui.review.badges",
        description="Read cheap unresolved quarantine, stale-journal, and transfer-review counts.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_review_badges",
        daemon_kind="ui.review.badges",
        summary_template="Read unresolved review counts",
    ),
    ToolEntry(
        name="ui.review.worklist",
        description=(
            "Read one bounded deterministic worklist combining readiness blockers, quarantine, "
            "stale edits, transfer candidates, open loan locks, commercial matches, and "
            "source-of-funds gaps. Local-only and read-only."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {"type": "integer", "minimum": 1, "maximum": 50},
                "categories": {
                    "type": "array",
                    "items": {"type": "string", "enum": list(_REVIEW_WORKLIST_CATEGORIES)},
                    "uniqueItems": True,
                    "maxItems": len(_REVIEW_WORKLIST_CATEGORIES),
                },
            },
        },
        kind_class="read_only",
        wire_name="ui_review_worklist",
        daemon_kind="ui.review.worklist",
        summary_template="Read accounting review worklist",
    ),
    ToolEntry(
        name="ui.loans.list",
        description=(
            "Read reviewed Bitcoin-backed-loan collateral/principal marks and heuristic open-lock hints. "
            "Open locks are reconcile signals, never proof of liquidation."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
            },
        },
        kind_class="read_only",
        wire_name="ui_loans_list",
        daemon_kind="ui.loans.list",
        summary_template="Read loan accounting marks",
    ),
    ToolEntry(
        name="ui.loans.mark",
        description=(
            "Mark one transaction as collateral posted/returned or loan principal received/repaid "
            "after explicit consent. Direction guards apply and journals become stale."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["txid", "as"],
            "properties": {
                "txid": {"type": "string"},
                "as": {"type": "string", "enum": list(_LOAN_MARK_TYPES)},
                "loan_id": {"type": "string"},
                "note": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_loans_mark",
        daemon_kind="ui.loans.mark",
        summary_template="Mark loan transaction",
    ),
    ToolEntry(
        name="ui.loans.link",
        description="Link two or more already-marked loan transactions under one loan id after consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["txids"],
            "properties": {
                "txids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 2,
                    "maxItems": 50,
                    "uniqueItems": True,
                },
                "loan_id": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_loans_link",
        daemon_kind="ui.loans.link",
        summary_template="Link loan transactions",
    ),
    ToolEntry(
        name="ui.loans.unmark",
        description=(
            "Remove one loan accounting mark after explicit consent so the transaction returns "
            "to normal tax classification. Journals become stale."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["txid"],
            "properties": {"txid": {"type": "string"}},
        },
        kind_class="mutating",
        wire_name="ui_loans_unmark",
        daemon_kind="ui.loans.unmark",
        summary_template="Remove loan transaction mark",
    ),
    ToolEntry(
        name="ui.transactions.metadata.update",
        description=(
            "Update reviewed transaction metadata after explicit consent. Supports notes, tags, exclusion, "
            "review/tax state, Austrian overrides, and explicit pricing provenance. Invalidates journals."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction"],
            "properties": {
                "transaction": {"type": "string"},
                "note": {"type": ["string", "null"]},
                "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 50},
                "excluded": {"type": "boolean"},
                "review_status": {"type": ["string", "null"]},
                "taxable": {"type": ["boolean", "null"]},
                "at_regime": {"type": ["string", "null"]},
                "at_category": {"type": ["string", "null"]},
                "fiat_currency": {"type": ["string", "null"]},
                "fiat_rate": {"type": ["string", "number", "null"]},
                "fiat_value": {"type": ["string", "number", "null"]},
                "pricing_source_kind": {"type": ["string", "null"]},
                "pricing_quality": {"type": ["string", "null"]},
                "pricing_external_ref": {"type": ["string", "null"]},
                "reason": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transactions_metadata_update",
        daemon_kind="ui.transactions.metadata.update",
        summary_template="Update transaction metadata",
    ),
    ToolEntry(
        name="ui.transactions.history.revert",
        description="Revert one audited metadata edit by creating a new forward edit after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["transaction", "event"],
            "properties": {
                "transaction": {"type": "string"},
                "event": {"type": "string"},
                "field": {"type": "string"},
                "reason": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_transactions_history_revert",
        daemon_kind="ui.transactions.history.revert",
        summary_template="Revert transaction edit",
    ),
    ToolEntry(
        name="ui.attachments.copy",
        description="Copy selected managed evidence from one transaction to another after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["source_transaction", "transaction", "attachments"],
            "properties": {
                "source_transaction": {"type": "string"},
                "transaction": {"type": "string"},
                "attachments": {"type": "array", "items": {"type": "string"}, "maxItems": 100},
            },
        },
        kind_class="mutating",
        wire_name="ui_attachments_copy",
        daemon_kind="ui.attachments.copy",
        summary_template="Copy transaction evidence",
    ),
    ToolEntry(
        name="ui.source_funds.evidence.list",
        description="Read AI-safe evidence labels and transaction associations for source-funds review.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
                "cursor": {"type": "string"},
            },
        },
        kind_class="read_only",
        wire_name="ui_source_funds_evidence_list",
        daemon_kind="ui.source_funds.evidence.list",
        summary_template="Read source-funds evidence",
    ),
    ToolEntry(
        name="ui.source_funds.sources.attach",
        description=(
            "Associate an existing managed attachment with a source-of-funds source after consent. "
            "The assistant never receives or selects a local path."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["source", "attachment_id"],
            "properties": {
                "source": {"type": "string"},
                "attachment_id": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_sources_attach",
        daemon_kind="ui.source_funds.sources.attach",
        summary_template="Attach existing evidence to source",
    ),
    ToolEntry(
        name="ui.source_funds.links.attach",
        description=(
            "Associate an existing managed attachment with a reviewed source-of-funds link after consent. "
            "The assistant never receives or selects a local path."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["link", "attachment_id"],
            "properties": {
                "link": {"type": "string"},
                "attachment_id": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_links_attach",
        daemon_kind="ui.source_funds.links.attach",
        summary_template="Attach existing evidence to link",
    ),
    ToolEntry(
        name="ui.source_funds.coverage",
        description="Read profile-wide source-funds coverage, ambiguity, and missing-history counts.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "max_depth": {"type": "integer", "minimum": 1, "maximum": 32},
                "max_transactions": {"type": "integer", "minimum": 1, "maximum": 50000},
            },
        },
        kind_class="read_only",
        wire_name="ui_source_funds_coverage",
        daemon_kind="ui.source_funds.coverage",
        summary_template="Read source-funds coverage",
    ),
    ToolEntry(
        name="ui.source_funds.cases.list",
        description="Read saved source-funds cases and their review/export state.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_source_funds_cases_list",
        daemon_kind="ui.source_funds.cases.list",
        summary_template="Read source-funds cases",
    ),
    ToolEntry(
        name="ui.source_funds.assemble",
        description=(
            "Run the local deterministic source-funds assembly loop for one target after explicit consent. "
            "It may seed suggestions and accept only deterministic links; it never contacts a backend."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["target_transaction"],
            "properties": {
                "target_transaction": {"type": "string"},
                "include_broad_hints": {"type": "boolean"},
                "max_passes": {"type": "integer", "minimum": 1, "maximum": 16},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_assemble",
        daemon_kind="ui.source_funds.assemble",
        summary_template="Assemble source-funds history",
    ),
    ToolEntry(
        name="ui.source_funds.cases.save",
        description="Save a reviewed source-funds case after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["target_transaction"],
            "properties": {
                "target_transaction": {"type": "string"},
                "case_label": {"type": "string"},
                "target_amount": {"type": "string"},
                "report_purpose": {"type": "string", "enum": list(_SOURCE_FUNDS_REPORT_PURPOSES)},
                "reveal_mode": {"type": "string", "enum": list(_SOURCE_FUNDS_REVEAL_MODES)},
                "max_depth": {"type": "integer", "minimum": 1, "maximum": 32},
                "recipient": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_cases_save",
        daemon_kind="ui.source_funds.cases.save",
        summary_template="Save source-funds case",
    ),
    ToolEntry(
        name="ui.source_funds.export",
        description="Export a saved, gate-checked source-funds case as PDF or evidence bundle after consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["case", "format"],
            "properties": {
                "case": {"type": "string"},
                "format": {"type": "string", "enum": ["pdf", "bundle"]},
            },
        },
        kind_class="mutating",
        wire_name="ui_source_funds_export",
        summary_template="Export source-funds case",
    ),
    ToolEntry(
        name="ui.transactions.commercial_context",
        description="Read redacted BTCPay invoice/payment and external-document provenance for one transaction.",
        parameters=_TRANSACTION_REF_SCHEMA,
        kind_class="read_only",
        wire_name="ui_transactions_commercial_context",
        daemon_kind="ui.transactions.commercial_context",
        summary_template="Read commercial provenance",
    ),
    ToolEntry(
        name="ui.btcpay.provenance.list",
        description="Read bounded redacted BTCPay invoice/payment provenance records.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "record_type": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
            },
        },
        kind_class="read_only",
        wire_name="ui_btcpay_provenance_list",
        daemon_kind="ui.btcpay.provenance.list",
        summary_template="Read BTCPay provenance",
    ),
    ToolEntry(
        name="ui.btcpay.provenance.suggest",
        description="Run deterministic commercial document/payment matching without writing review decisions.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 200}},
        },
        kind_class="read_only",
        wire_name="ui_btcpay_provenance_suggest",
        daemon_kind="ui.btcpay.provenance.suggest",
        summary_template="Suggest commercial matches",
    ),
    ToolEntry(
        name="ui.btcpay.provenance.links",
        description="Read reviewed or suggested commercial reconciliation links.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "state": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 200},
            },
        },
        kind_class="read_only",
        wire_name="ui_btcpay_provenance_links",
        daemon_kind="ui.btcpay.provenance.links",
        summary_template="Read commercial reconciliation links",
    ),
    ToolEntry(
        name="ui.documents.list",
        description="Read external document metadata such as labels, issuers, dates, values, and review state; no file bytes.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 200}},
        },
        kind_class="read_only",
        wire_name="ui_documents_list",
        daemon_kind="ui.documents.list",
        summary_template="Read external documents",
    ),
    ToolEntry(
        name="ui.btcpay.provenance.review",
        description="Accept, reject, or amend one commercial reconciliation link after explicit consent.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["link", "state"],
            "properties": {
                "link": {"type": "string"},
                "state": {"type": "string", "enum": ["suggested", "reviewed", "rejected"]},
                "reconciliation_state": {"type": "string"},
                "commercial_kind": {"type": "string"},
                "notes": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_btcpay_provenance_review",
        daemon_kind="ui.btcpay.provenance.review",
        summary_template="Review commercial reconciliation link",
    ),
    ToolEntry(
        name="ui.documents.create",
        description="Create external invoice/receipt/contract metadata after explicit consent; does not read or attach a local file.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["label", "document_type"],
            "properties": {
                "label": {"type": "string"},
                "document_type": {"type": "string"},
                "external_ref": {"type": "string"},
                "issuer": {"type": "string"},
                "counterparty": {"type": "string"},
                "issued_at": {"type": "string"},
                "due_at": {"type": "string"},
                "fiat_currency": {"type": "string"},
                "fiat_value": {"type": ["string", "number"]},
                "notes": {"type": "string"},
            },
        },
        kind_class="mutating",
        wire_name="ui_documents_create",
        daemon_kind="ui.documents.create",
        summary_template="Create external document metadata",
    ),
    ToolEntry(
        name="ui.reports.exit_tax_preview",
        description="Read the deterministic Austrian exit-tax preview for a departure date and destination class.",
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["departure_date", "destination"],
            "properties": {
                "departure_date": {"type": "string"},
                "destination": {"type": "string", "enum": ["eu_eea", "third_country"]},
            },
        },
        kind_class="read_only",
        wire_name="ui_reports_exit_tax_preview",
        daemon_kind="ui.reports.exit_tax_preview",
        summary_template="Preview Austrian exit tax",
    ),
    ToolEntry(
        name="ui.rates.latest",
        description=(
            "Fetch and cache the latest public market rate after explicit consent. "
            "This contacts the configured live provider and requires the book's live-rate opt-in."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "pair": {"type": "string", "description": "Optional pair such as BTC-EUR."},
                "source": {
                    "type": "string",
                    "enum": ["coinbase-exchange", "coingecko"],
                },
            },
        },
        kind_class="mutating",
        wire_name="ui_rates_latest",
        daemon_kind="ui.rates.latest",
        summary_template="Fetch latest market rate",
    ),
    ToolEntry(
        name="ui.reports.export",
        description=(
            "Export a deterministic report or advisor handoff artifact after explicit consent. "
            "The AI receives only artifact metadata, never the managed local path."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["report", "format"],
            "properties": {
                "report": {
                    "type": "string",
                    "enum": ["full", "summary", "capital_gains", "austrian_e1kv", "exit_tax", "audit_package"],
                },
                "format": {"type": "string", "enum": ["pdf", "xlsx", "csv", "package"]},
                "year": {"type": "integer", "minimum": 2009, "maximum": 2100},
                "wallet": {"type": "string"},
                "departure_date": {"type": "string"},
                "destination": {"type": "string", "enum": ["eu_eea", "third_country"]},
                "transaction": {"type": "string"},
                "transactions": {"type": "array", "items": {"type": "string"}, "maxItems": 100},
                "source_funds_case": {"type": "string"},
                "verify": {"type": "boolean"},
            },
        },
        kind_class="mutating",
        wire_name="ui_reports_export",
        summary_template="Export report artifact",
    ),
    ToolEntry(
        name="ui.egress.snapshot",
        description=(
            "Read an AI-safe aggregate of recent outbound activity by subsystem. "
            "Configured backend hosts, paths, query strings, headers, and request bodies are not returned."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "after_id": {"type": "integer", "minimum": 0},
                "limit": {"type": "integer", "minimum": 1, "maximum": 500},
            },
        },
        kind_class="read_only",
        wire_name="ui_egress_snapshot",
        daemon_kind="ui.egress.snapshot",
        summary_template="Read outbound privacy summary",
    ),
)


TOOL_CATALOG: tuple[ToolEntry, ...] = (*_BASE_TOOL_CATALOG, *_EXPANDED_TOOL_CATALOG)

TOOL_CAPABILITY_NAMES = (
    "core",
    "workspace",
    "transactions",
    "reports",
    "wallets",
    "loans",
    "privacy",
    "source_funds",
    "merchant",
    "transfers",
    "operations",
)

_CORE_TOOL_NAMES = {
    "status",
    "ui.overview.snapshot",
    "ui.report.blockers",
    "ui.workspace.health",
    "ui.next_actions",
    "read_skill_reference",
}


def tool_capabilities(tool: ToolEntry) -> frozenset[str]:
    """Return the small-model capability packs that should advertise a tool."""

    name = tool.name
    capabilities: set[str] = set()
    if name in _CORE_TOOL_NAMES:
        capabilities.add("core")
    if name in {"ui.profiles.snapshot", "ui.workspace.overview.snapshot"}:
        capabilities.add("workspace")
    if name.startswith(("ui.transactions.", "ui.activity.", "ui.attachments.")):
        capabilities.add("transactions")
    if name in {"ui.audit.evidence.summary", "ui.review.badges", "ui.review.worklist"}:
        capabilities.update({"transactions", "operations"})
    if name.startswith("ui.loans."):
        capabilities.add("loans")
    if name.startswith(("ui.reports.", "ui.rates.")) or name in {
        "ui.report.blockers",
        "ui.audit.changes_since_last_answer",
    }:
        capabilities.add("reports")
    if name.startswith(("ui.wallets.", "ui.backends.")) or name in {
        "ui.maintenance.settings",
        "ui.maintenance.configure",
        "ui.maintenance.run",
        "ui.connections.node.snapshot",
    }:
        capabilities.update({"wallets", "operations"})
    if "privacy" in name or name == "ui.egress.snapshot":
        capabilities.add("privacy")
    if name.startswith("ui.source_funds."):
        capabilities.add("source_funds")
    if name.startswith(("ui.btcpay.", "ui.documents.")) or name == "ui.transactions.commercial_context":
        capabilities.add("merchant")
    if name.startswith(("ui.transfers.", "ui.saved_views.")) or name == "ui.journals.transfers.list":
        capabilities.add("transfers")
    if name.startswith("ui.journals."):
        capabilities.update({"transactions", "reports"})
    return frozenset(capabilities or {"core"})


def select_tool_capabilities(
    messages: list[dict[str, Any]] | None,
    screen_context: dict[str, Any] | None = None,
) -> frozenset[str] | None:
    """Select bounded tool packs from the question and current typed screen context.

    ``None`` means advertise the full catalog. This is reserved for explicit
    capability-discovery questions and keeps the public helper backward compatible.
    """

    if messages is None:
        return None
    recent_user_messages = [
        str(message["content"]).lower()
        for message in messages
        if message.get("role") == "user" and isinstance(message.get("content"), str)
    ][-3:]
    latest = recent_user_messages[-1] if recent_user_messages else ""
    if any(phrase in latest for phrase in ("what can you do", "all tools", "capabilities", "tool catalog")):
        return None

    selected = {"core"}
    route = str((screen_context or {}).get("route") or "").lower()
    requested = (screen_context or {}).get("capabilities")
    if isinstance(requested, list):
        selected.update(str(item) for item in requested if str(item) in TOOL_CAPABILITY_NAMES)

    haystack = f"{route} {' '.join(recent_user_messages)}"
    keyword_groups = {
        "workspace": (
            "all books", "book set", "books set", "treasury", "across books",
            "workspace overview", "alle bücher", "alle buecher", "buchset",
            "buch-set", "über alle bücher", "ueber alle buecher",
            "gesamtvermögen", "gesamtvermoegen",
        ),
        "transactions": ("transaction", "txid", "note", "tag", "metadata", "evidence", "attachment", "quarantine", "edit"),
        "reports": ("report", "summary", "total", "journal", "tax", "gain", "balance", "portfolio", "price", "rate", "export", "steuer", "e1kv", "exit tax"),
        "wallets": ("wallet", "backend", "sync", "source", "utxo", "connection"),
        "loans": ("loan", "collateral", "borrowed", "principal", "liquidation", "darlehen", "kredit"),
        "privacy": ("privacy", "linkable", "egress", "outbound", "psbt"),
        "source_funds": ("source of funds", "source-of-funds", "provenance", "audit package", "proof of funds"),
        "merchant": ("btcpay", "invoice", "receipt", "merchant", "commercial", "document"),
        "transfers": ("transfer", "swap", "payout", "boltz", "peg", "coinjoin", "whirlpool", "pair"),
        "operations": ("health", "pending", "next", "ready", "stale", "maintenance", "diagnose", "broken", "failed", "review", "worklist", "unresolved", "to do", "todo"),
    }
    for capability, keywords in keyword_groups.items():
        if any(keyword in haystack for keyword in keywords):
            selected.add(capability)
    if "transfers" in selected:
        selected.add("transactions")
    if route == "/books":
        selected.add("wallets")
    return frozenset(selected)

TOOL_BY_NAME: dict[str, ToolEntry] = {}
for tool in TOOL_CATALOG:
    TOOL_BY_NAME[tool.name] = tool
    TOOL_BY_NAME[tool.provider_name] = tool
TOOL_BY_NAME["ui_reports_report_blockers"] = TOOL_BY_NAME["ui.report.blockers"]


def get_tool(name: str) -> ToolEntry | None:
    return TOOL_BY_NAME.get(name)


def openai_tool_definitions(
    *,
    include_mutating: bool = False,
    capabilities: frozenset[str] | None = None,
    allowed_names: frozenset[str] | None = None,
) -> list[dict[str, Any]]:
    return [
        tool.to_openai_tool()
        for tool in TOOL_CATALOG
        if (include_mutating or tool.kind_class == "read_only")
        and (allowed_names is None or tool.name in allowed_names)
        and (capabilities is None or bool(tool_capabilities(tool) & capabilities))
    ]


def redact_tool_arguments(value: Any) -> Any:
    """Return a UI-safe preview of model-supplied tool arguments."""
    if isinstance(value, dict):
        redacted: dict[str, Any] = {}
        for key, item in value.items():
            key_text = str(key)
            lowered = key_text.lower()
            # `debug` carries sanitized tracebacks on error envelopes; never let
            # one ride into provider-bound content even if an envelope is embedded.
            if key_text == "debug":
                continue
            if is_sensitive_key(key_text) or any(
                part in lowered for part in SENSITIVE_ARGUMENT_KEY_PARTS
            ):
                redacted[key_text] = "<redacted>"
            else:
                redacted[key_text] = redact_tool_arguments(item)
        return redacted
    if isinstance(value, (list, tuple)):
        return [redact_tool_arguments(item) for item in value]
    if isinstance(value, str):
        return redact_secret_text(value)
    return value


def redact_ai_tool_result(value: Any) -> Any:
    """Remove local paths and URLs from provider-bound AI tool results.

    Tool arguments come from the model and use ``redact_tool_arguments`` for
    secret-safe consent previews. Tool *results* cross the opposite trust
    boundary: local builders and exception messages can contain filesystem
    paths or URLs embedded anywhere inside a free-text value. Apply the secret
    floor first, then recursively scrub those locations before the result can
    reach either the model or renderer tool chrome.
    """

    secret_safe = redact_tool_arguments(value)

    def scrub_locations(item: Any) -> Any:
        if isinstance(item, dict):
            return {key: scrub_locations(child) for key, child in item.items()}
        if isinstance(item, (list, tuple)):
            return [scrub_locations(child) for child in item]
        if isinstance(item, str):
            without_urls = _AI_TOOL_RESULT_URL_RE.sub("<redacted-url>", item)
            path_match = _AI_TOOL_RESULT_ABSOLUTE_PATH_RE.search(without_urls)
            if path_match is not None:
                # Filesystem paths may legally contain punctuation and spaces,
                # so there is no safe generic delimiter for their tail. Keep
                # useful text before the path, then redact the entire remainder
                # rather than risk leaking a suffix the regex did not consume.
                return without_urls[: path_match.start()] + "<redacted-path>"
            return without_urls
        return item

    return scrub_locations(secret_safe)


def summarize_tool_call(tool: ToolEntry, arguments: dict[str, Any]) -> str:
    """Build a short, non-secret consent summary for an allowlisted tool."""
    if tool.name == "ui.wallets.sync":
        wallet = arguments.get("wallet")
        if isinstance(wallet, str) and wallet.strip():
            return f"Refresh source {wallet.strip()}"
        return "Refresh all watch-only sources"
    if tool.name == "ui.journals.process":
        return "Process journals"
    if tool.name == "ui.journals.quarantine.resolve":
        target = arguments.get("transaction")
        transaction = (
            target.strip()
            if isinstance(target, str) and target.strip()
            else "quarantined transaction"
        )
        if arguments.get("action") == "exclude":
            return f"Exclude {transaction} from accounting"
        return f"Apply reviewed price to {transaction}"
    if tool.name == "ui.rates.rebuild":
        pair = arguments.get("pair")
        if isinstance(pair, str) and pair.strip():
            return f"Fetch spot prices for {pair.strip()}"
        return "Fetch missing spot prices and reprocess journals"
    if tool.name == "ui.rates.latest":
        pair = arguments.get("pair")
        source = arguments.get("source")
        target = pair.strip() if isinstance(pair, str) and pair.strip() else "the active book pair"
        if isinstance(source, str) and source.strip():
            return f"Fetch latest {target} rate from {source.strip()}"
        return f"Fetch latest {target} market rate"
    if tool.name == "ui.maintenance.configure":
        enabled = arguments.get("report_read_sync", arguments.get("auto_sync_before_report_reads"))
        provider = arguments.get("market_rate_provider")
        if enabled is None and isinstance(provider, str) and provider.strip():
            return f"Set market-rate provider to {provider.strip()}"
        return (
            "Enable freshness refresh before report reads"
            if enabled is True
            else "Disable freshness refresh before report reads"
        )
    if tool.name == "ui.maintenance.run":
        sync_mode = arguments.get("sync", "if_enabled")
        if sync_mode == "always":
            return "Refresh sources and process journals"
        if sync_mode == "never":
            return "Process journals without source refresh"
        return "Run maintenance using current settings"
    if tool.name == "ui.source_funds.sources.create":
        label = arguments.get("label")
        if isinstance(label, str) and label.strip():
            return f"Create source-funds source {label.strip()}"
        return "Create source-funds source"
    if tool.name == "ui.source_funds.links.create":
        target = arguments.get("to_transaction")
        link_type = arguments.get("link_type")
        if isinstance(target, str) and target.strip():
            label = link_type.strip() if isinstance(link_type, str) and link_type.strip() else "link"
            return f"Create {label} source-funds link to {target.strip()}"
        return "Create source-funds link"
    if tool.name == "ui.source_funds.links.review":
        link = arguments.get("link")
        state = arguments.get("state")
        if isinstance(link, str) and link.strip():
            if isinstance(state, str) and state.strip():
                return f"Mark source-funds link {link.strip()} as {state.strip()}"
            return f"Review source-funds link {link.strip()}"
        return "Review source-funds link"
    if tool.name == "ui.source_funds.suggest":
        target = arguments.get("target_transaction")
        if isinstance(target, str) and target.strip():
            return f"Seed source-funds suggestions for {target.strip()}"
        return "Seed source-funds suggestions"
    if tool.name == "ui.source_funds.links.bulk_review":
        target = arguments.get("target_transaction")
        if isinstance(target, str) and target.strip():
            return f"Review deterministic source-funds suggestions for {target.strip()}"
        return "Review deterministic source-funds suggestions"
    if tool.name == "ui.source_funds.sources.attach":
        source = arguments.get("source")
        return (
            f"Attach existing evidence to source {source.strip()}"
            if isinstance(source, str) and source.strip()
            else "Attach existing evidence to source"
        )
    if tool.name == "ui.source_funds.links.attach":
        link = arguments.get("link")
        return (
            f"Attach existing evidence to link {link.strip()}"
            if isinstance(link, str) and link.strip()
            else "Attach existing evidence to source-funds link"
        )
    if tool.name == "ui.source_funds.assemble":
        target = arguments.get("target_transaction")
        return (
            f"Assemble source-funds history for {target.strip()}"
            if isinstance(target, str) and target.strip()
            else "Assemble source-funds history"
        )
    if tool.name == "ui.transactions.metadata.update":
        target = arguments.get("transaction")
        return (
            f"Update reviewed metadata for {target.strip()}"
            if isinstance(target, str) and target.strip()
            else "Update transaction metadata"
        )
    if tool.name == "ui.transactions.history.revert":
        target = arguments.get("transaction")
        return (
            f"Revert an audited edit on {target.strip()}"
            if isinstance(target, str) and target.strip()
            else "Revert transaction edit"
        )
    if tool.name == "ui.loans.mark":
        target = arguments.get("txid")
        role = arguments.get("as")
        if isinstance(target, str) and target.strip() and isinstance(role, str):
            return f"Mark {target.strip()} as loan {role}"
        return "Mark loan transaction"
    if tool.name == "ui.loans.link":
        txids = arguments.get("txids")
        count = len(txids) if isinstance(txids, list) else 0
        return f"Link {count} loan transactions" if count else "Link loan transactions"
    if tool.name == "ui.loans.unmark":
        target = arguments.get("txid")
        return (
            f"Remove loan mark from {target.strip()}"
            if isinstance(target, str) and target.strip()
            else "Remove loan transaction mark"
        )
    if tool.name == "ui.transfers.payouts.create":
        target = arguments.get("tx_out")
        asset = arguments.get("payout_asset")
        amount = arguments.get("payout_amount")
        if all(isinstance(value, str) and value.strip() for value in (target, asset, amount)):
            return f"Record {amount.strip()} {asset.strip()} direct payout for {target.strip()}"
        return "Create direct swap payout"
    if tool.name == "ui.transfers.payouts.delete":
        payout = arguments.get("payout_id")
        return (
            f"Delete direct payout {payout.strip()}"
            if isinstance(payout, str) and payout.strip()
            else "Delete direct swap payout"
        )
    if tool.name == "ui.transfers.update":
        pair = arguments.get("pair_id")
        return (
            f"Update reviewed transfer pair {pair.strip()}"
            if isinstance(pair, str) and pair.strip()
            else "Update reviewed transfer pair"
        )
    if tool.name == "ui.transfers.components.bulk_resolve":
        components = arguments.get("components")
        count = len(components) if isinstance(components, list) else 0
        noun = "component" if count == 1 else "components"
        if arguments.get("dry_run") is True:
            return f"Preview {count or 'custody'} gap-resolution {noun}"
        return f"Create {count or 'custody'} gap-resolution {noun}"
    if tool.name == "ui.reports.export":
        report = arguments.get("report")
        export_format = arguments.get("format")
        if isinstance(report, str) and isinstance(export_format, str):
            return f"Export {report.replace('_', ' ')} as {export_format.upper()}"
        return "Export report artifact"
    if tool.name == "ui.source_funds.export":
        export_format = arguments.get("format")
        return (
            f"Export source-funds {export_format}"
            if isinstance(export_format, str)
            else "Export source-funds case"
        )
    return tool.summary_template or tool.name


SKILL_REFERENCE_INDEX = """# Kassiber In-App Skill Index

Use this compact index to choose an allowlisted deeper reference. Do not ask
users to paste secrets, wallet files, descriptors, xpub material, tokens, auth
headers, cookies, or raw config JSON into chat.

Core workflow: create/select workspace and profile -> configure backend and
wallet -> sync or import transactions -> add metadata/tags/notes/exclusions ->
process journals -> review quarantine and transfer/swap pairs -> run reports ->
export or back up.

Before answering workspace-specific questions, use safe read tools such as
ui.workspace.health, ui.next_actions, ui.wallets.list, ui.wallets.utxos,
ui.wallets.identify, ui.backends.list,
ui.workspace.overview.snapshot (only after an explicit all-books request),
ui.transactions.list, ui.transactions.extremes, ui.transactions.search,
ui.journals.quarantine, ui.journals.events.list,
ui.journals.transfers.list, ui.review.worklist, ui.loans.list,
ui.transfers.review_context, ui.transfers.payouts.list,
ui.transfers.components.list, ui.rates.summary,
ui.rates.coverage, ui.report.blockers, ui.audit.changes_since_last_answer,
ui.maintenance.settings, ui.reports.summary, ui.reports.balance_sheet,
ui.reports.portfolio_summary, ui.reports.tax_summary, ui.reports.balance_history,
ui.reports.privacy_hygiene, ui.reports.privacy_mirror,
ui.source_funds.sources.list, ui.source_funds.links.list,
ui.source_funds.preview, and report snapshots. Use
ui.reports.summary for exact all-time inflow/outflow rollups,
including reviewed transfer_pairs that explain swaps or pegs inside raw flows,
ui.reports.balance_sheet for current bucket holdings,
ui.reports.portfolio_summary for current wallet holdings,
ui.transactions.extremes for largest/smallest transactions, and
ui.transactions.search for specific notes, counterparties, tags, ids, or txids.
Use ui.review.worklist for a bounded cross-workflow review queue. Treat loan
open-lock rows as heuristic reconcile hints, not liquidation proof. A book-set
overview preserves book boundaries and must not combine mixed fiat currencies.
Use ui.report.blockers before saying reports are ready, ui.rates.coverage for
missing-price questions, ui.reports.privacy_mirror for what is linkable, who can
infer it, unknown coverage, and worst privacy risk questions,
ui.reports.privacy_hygiene for privacy posture configuration questions, and
ui.audit.changes_since_last_answer when checking
whether a previous answer is still current. Do not invent calculations when
Kassiber can read program-derived output.
For a price-only quarantine, use ui.journals.quarantine.resolve only after the
user has reviewed a rate or total fiat value. Exclusion requires explicit user
confirmation. Never use that tool to hide an ownership or custody gap.
For Boltz/submarine swap, peg, and Bitcoin rail questions, read
ui.transfers.review_context first; use ui.transfers.suggest/list for focused
candidate or pair follow-ups. Treat Bitcoin swaps as carrying-value candidates
only when both legs are known owned-wallet legs; swap-routed payments or
receipts should remain unpaired. Read swap-matching when review policy,
confidence bands, or pairing workflow matters.
For missing wallets, multi-hop migrations, or 1:N/N:1/N:M self-custody gaps,
read ui.transfers.components.list and the relevant transaction/transfer review
context, then validate ui.transfers.components.bulk_resolve with dry_run=true.
Ask for consent before the final atomic write; exact conservation and complete
anchor coverage are mandatory, and tax policy is downstream of custody proof.
For source-of-funds/provenance questions, read source-funds links and preview
before proposing writes. Source-funds AI write tools require user consent,
support non-CoinJoin link types, and write evidence records only; they do not
mutate tax/journal transaction_pairs. Keep CoinJoin/PayJoin links explicit about
privacy-boundary ambiguity unless the user has reviewed stronger evidence.

Stale local journals are maintenance, not a question for the user; read/report
tools may refresh them before answering. Watch-only source refresh contacts
external services, so use ui.maintenance.settings to inspect the active-profile
setting and ui.maintenance.run or ui.wallets.sync only after explicit consent.

Read command-templates for exact CLI command shapes and common fast paths.
Read onboarding for first-run setup, data roots, and context selection.
Read wallets-backends for wallet kinds, backend selection, source refresh, and
imports. Read journal-processing for processing order, stale journals,
quarantines, and transfer/swap pairing. Read swap-matching for candidate review,
conflicts, auto-pair rules, and saved review views. Read metadata for notes, tags,
exclusions, BIP329 labels, and attachments. Read reports for summary,
portfolio, capital gains, balance history, Austrian handoff, and exports.
Read verification for quick state checks and smoke validation. Read
troubleshooting for common errors and path confusion. Read secrets-and-backup
for SQLCipher, passphrase/fd handling, credential migration, and backups.
"""


def skill_reference_root() -> Path:
    return Path(__file__).resolve().parent / "skill_references"


def skill_reference_roots(*, root: Path | None = None) -> list[Path]:
    roots: list[Path] = []
    if root is not None:
        roots.append(root)
    roots.append(skill_reference_root())
    roots.append(Path.cwd() / "kassiber" / "ai" / "skill_references")
    bundle_root = getattr(sys, "_MEIPASS", None)
    if bundle_root:
        roots.append(Path(bundle_root) / "kassiber" / "ai" / "skill_references")

    unique: list[Path] = []
    seen: set[Path] = set()
    for candidate in roots:
        resolved = candidate.resolve(strict=False)
        if resolved not in seen:
            seen.add(resolved)
            unique.append(candidate)
    return unique


def read_skill_reference(name: str, *, root: Path | None = None) -> dict[str, str]:
    if name not in SKILL_REFERENCE_NAMES:
        raise AppError(
            "skill reference is not allowlisted",
            code="tool_not_allowed",
            details={"name": name, "allowed": list(SKILL_REFERENCE_NAMES)},
            retryable=False,
        )
    if name == "index":
        return {"name": name, "content": SKILL_REFERENCE_INDEX}
    attempted: list[str] = []
    for reference_root in skill_reference_roots(root=root):
        path = reference_root / f"{name}.md"
        attempted.append(str(path))
        try:
            content = path.read_text(encoding="utf-8")
        except OSError:
            continue
        return {"name": name, "content": content}
    raise AppError(
        "skill reference could not be read",
        code="tool_reference_unavailable",
        details={"name": name, "attempted": attempted},
        retryable=False,
    )
