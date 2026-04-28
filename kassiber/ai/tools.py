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
            },
        },
        kind_class="read_only",
        wire_name="ui_transactions_list",
        daemon_kind="ui.transactions.list",
        summary_template="Read recent transactions",
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
        name="ui.journals.snapshot",
        description="Read journal processing status, recent journal rows, and quarantine summary.",
        parameters=_EMPTY_OBJECT_SCHEMA,
        kind_class="read_only",
        wire_name="ui_journals_snapshot",
        daemon_kind="ui.journals.snapshot",
        summary_template="Read journals snapshot",
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
    return tool.summary_template or tool.name


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
