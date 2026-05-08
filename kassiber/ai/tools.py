"""Curated AI tool catalog for Kassiber's in-app assistant."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

from ..errors import AppError


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
    "troubleshooting",
    "verification",
    "wallets-backends",
)


_EMPTY_OBJECT_SCHEMA: dict[str, Any] = {
    "type": "object",
    "additionalProperties": False,
    "properties": {},
}


TOOL_CATALOG: tuple[ToolEntry, ...] = (
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
        description="Read workspaces, profiles, and the active profile summary.",
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
            "including asset_flow and wallet_flow with BTC, sat, and msat amounts."
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
        name="ui.journals.snapshot",
        description="Read journal processing status, recent journal rows, and quarantine summary.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_journals_snapshot",
        daemon_kind="ui.journals.snapshot",
        summary_template="Read journals snapshot",
    ),
    ToolEntry(
        name="ui.journals.quarantine",
        description="Read quarantine counts and a bounded recent list of quarantined transactions.",
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
            "changed since an optional prior answer timestamp."
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
        name="ui.maintenance.settings",
        description=(
            "Read AI maintenance settings for the active profile, including whether "
            "wallet sync is allowed before report reads."
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
            "Sync configured wallets after the user explicitly allows this "
            "mutating action."
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
        summary_template="Sync wallets",
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
            "Change AI maintenance settings after explicit consent. Currently "
            "controls whether wallet sync may run automatically before report reads."
        ),
        parameters={
            "type": "object",
            "additionalProperties": False,
            "required": ["auto_sync_before_report_reads"],
            "properties": {
                "auto_sync_before_report_reads": {
                    "type": "boolean",
                    "description": (
                        "When true, report/read tools may sync configured wallets "
                        "before refreshing journals."
                    ),
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
            "Run local maintenance after explicit consent: optional wallet sync "
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
)

TOOL_BY_NAME: dict[str, ToolEntry] = {}
for tool in TOOL_CATALOG:
    TOOL_BY_NAME[tool.name] = tool
    TOOL_BY_NAME[tool.provider_name] = tool


def get_tool(name: str) -> ToolEntry | None:
    return TOOL_BY_NAME.get(name)


def openai_tool_definitions(*, include_mutating: bool = False) -> list[dict[str, Any]]:
    return [
        tool.to_openai_tool()
        for tool in TOOL_CATALOG
        if include_mutating or tool.kind_class == "read_only"
    ]


def redact_tool_arguments(value: Any) -> Any:
    """Return a UI-safe preview of model-supplied tool arguments."""
    if isinstance(value, dict):
        redacted: dict[str, Any] = {}
        for key, item in value.items():
            key_text = str(key)
            lowered = key_text.lower()
            if any(part in lowered for part in SENSITIVE_ARGUMENT_KEY_PARTS):
                redacted[key_text] = "<redacted>"
            else:
                redacted[key_text] = redact_tool_arguments(item)
        return redacted
    if isinstance(value, list):
        return [redact_tool_arguments(item) for item in value]
    return value


def summarize_tool_call(tool: ToolEntry, arguments: dict[str, Any]) -> str:
    """Build a short, non-secret consent summary for an allowlisted tool."""
    if tool.name == "ui.wallets.sync":
        wallet = arguments.get("wallet")
        if isinstance(wallet, str) and wallet.strip():
            return f"Sync wallet {wallet.strip()}"
        return "Sync all wallets"
    if tool.name == "ui.journals.process":
        return "Process journals"
    if tool.name == "ui.maintenance.configure":
        enabled = arguments.get("auto_sync_before_report_reads")
        return (
            "Enable automatic wallet sync before report reads"
            if enabled is True
            else "Disable automatic wallet sync before report reads"
        )
    if tool.name == "ui.maintenance.run":
        sync_mode = arguments.get("sync", "if_enabled")
        if sync_mode == "always":
            return "Sync wallets and process journals"
        if sync_mode == "never":
            return "Process journals without wallet sync"
        return "Run maintenance using current settings"
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
ui.workspace.health, ui.next_actions, ui.wallets.list, ui.backends.list,
ui.transactions.list, ui.transactions.extremes, ui.transactions.search,
ui.journals.quarantine, ui.journals.transfers.list, ui.rates.summary,
ui.rates.coverage, ui.report.blockers, ui.audit.changes_since_last_answer,
ui.maintenance.settings, ui.reports.summary, ui.reports.balance_sheet,
ui.reports.portfolio_summary, ui.reports.tax_summary,
ui.reports.balance_history, and report snapshots. Use
ui.reports.summary for exact all-time inflow/outflow rollups,
ui.reports.balance_sheet for current bucket holdings,
ui.reports.portfolio_summary for current wallet holdings,
ui.transactions.extremes for largest/smallest transactions, and
ui.transactions.search for specific notes, counterparties, tags, ids, or txids.
Use ui.report.blockers before saying reports are ready, ui.rates.coverage for
missing-price questions, and ui.audit.changes_since_last_answer when checking
whether a previous answer is still current. Do not invent calculations when
Kassiber can read program-derived output.

Stale local journals are maintenance, not a question for the user; read/report
tools may refresh them before answering. Wallet sync contacts external services,
so use ui.maintenance.settings to inspect the active-profile setting and
ui.maintenance.run or ui.wallets.sync only after explicit consent.

Read command-templates for exact CLI command shapes and common fast paths.
Read onboarding for first-run setup, data roots, and context selection.
Read wallets-backends for wallet kinds, backend selection, live sync, and
imports. Read journal-processing for processing order, stale journals,
quarantines, and transfer/swap pairing. Read metadata for notes, tags,
exclusions, BIP329 labels, and attachments. Read reports for summary,
portfolio, capital gains, balance history, Austrian handoff, and exports.
Read verification for quick state checks and smoke validation. Read
troubleshooting for common errors and path confusion. Read secrets-and-backup
for SQLCipher, passphrase/fd handling, credential migration, and backups.
"""


def skill_reference_root() -> Path:
    return Path(__file__).resolve().parents[2] / "skills" / "kassiber" / "references"


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
    reference_root = root or skill_reference_root()
    path = reference_root / f"{name}.md"
    try:
        content = path.read_text(encoding="utf-8")
    except OSError as exc:
        raise AppError(
            "skill reference could not be read",
            code="tool_reference_unavailable",
            details={"name": name},
            retryable=False,
        ) from exc
    return {"name": name, "content": content}
