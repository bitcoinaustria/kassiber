from __future__ import annotations

import json
import platform
import re
import sqlite3
import sys
import traceback
import ipaddress
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from . import __version__
from .core.runtime import resolve_runtime_paths
from .envelope import SCHEMA_VERSION, derive_kind, json_ready
from .time_utils import now_iso


REPORT_SCHEMA_VERSION = 1
PUBLIC_SAFE_NOTICE = (
    "This report is designed for public bug reports. It omits labels, paths, "
    "addresses, txids, exact amounts, notes, raw config, API payloads, and stack locals."
)
DIAGNOSTICS_SUBDIR = Path("exports") / "diagnostics"

_SENSITIVE_KEY_TOKENS = {
    "address",
    "amount",
    "auth",
    "balance",
    "basis",
    "blinding",
    "code",
    "config",
    "cookie",
    "counterparty",
    "credential",
    "cursor",
    "date",
    "description",
    "descriptor",
    "end",
    "env",
    "external",
    "fee",
    "file",
    "fingerprint",
    "gain",
    "id",
    "key",
    "label",
    "loss",
    "name",
    "note",
    "output",
    "password",
    "path",
    "price",
    "proceeds",
    "profile",
    "rate",
    "raw",
    "ref",
    "secret",
    "source",
    "start",
    "store",
    "time",
    "timestamp",
    "token",
    "tx",
    "url",
    "user",
    "value",
    "wallet",
    "workspace",
}
_SENSITIVE_EXACT_KEYS = {
    "account",
    "asset",
    "backend",
    "pair_id",
    "tag",
    "transaction",
    "type",
}
_SAFE_ENUM_KEYS = {
    "all",
    "chain",
    "command",
    "debug",
    "dry_run",
    "format",
    "insecure",
    "kind",
    "machine",
    "network",
    "policy",
    "save",
    "sync_all",
}
_SKIP_ARG_KEYS = {
    "command",
    "runtime_config",
    "data_root",
    "env_file",
    "output",
    "diagnostics_out",
}
_PATHISH_RE = re.compile(
    r"(?P<path>(?:/[A-Za-z0-9_.@+-][^\s'\":)]*)|(?:[A-Za-z]:\\[^\s'\":)]+))"
)
_URL_RE = re.compile(r"\b[a-z][a-z0-9+.-]*://[^\s'\"<>]+", re.IGNORECASE)
_TIMESTAMP_RE = re.compile(
    r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2})?\b",
    re.IGNORECASE,
)
_HEX64_RE = re.compile(r"\b[0-9a-fA-F]{64}\b")
_XPUB_RE = re.compile(r"\b(?:xpub|ypub|zpub|tpub|upub|vpub)[A-Za-z0-9]{20,}\b", re.IGNORECASE)
_BECH32_RE = re.compile(r"\b(?:bc1|tb1|bcrt1|lq1|ex1)[0-9a-z]{8,90}\b", re.IGNORECASE)
_AMOUNT_UNIT_RE = re.compile(
    r"(?<![A-Za-z0-9_])[-+]?\d+(?:\.\d+)?\s*(?:msat|sats?|btc|lbtc|usd|eur)\b",
    re.IGNORECASE,
)
_NUMBER_RE = re.compile(r"(?<![A-Za-z_])[-+]?\d+(?:\.\d+)?(?![A-Za-z_])")


@dataclass(frozen=True)
class DiagnosticsSaveResult:
    path: Path
    relative_path: str | None
    target: str


def collect_public_diagnostics(
    conn: sqlite3.Connection | None,
    args: Any,
    *,
    runtime_config: dict[str, Any] | None = None,
    error: BaseException | None = None,
    stack: list[traceback.FrameSummary] | traceback.StackSummary | None = None,
    unhandled: bool = False,
) -> dict[str, Any]:
    paths = _safe_runtime_paths(args)
    config = runtime_config or getattr(args, "runtime_config", None) or {}
    report = {
        "report_schema_version": REPORT_SCHEMA_VERSION,
        "public_safe": True,
        "generated_at": now_iso(),
        "notice": PUBLIC_SAFE_NOTICE,
        "privacy_contract": _privacy_contract(),
        "environment": _environment_summary(),
        "invocation": _invocation_summary(args),
        "storage": _storage_summary(paths),
        "state": _state_summary(conn, config),
        "checks": _checks_summary(conn),
    }
    if error is not None:
        report["error"] = _error_summary(error, unhandled=unhandled)
    if stack:
        report["stack"] = _stack_summary(stack)
    return report


def save_public_diagnostics_report(
    report: dict[str, Any],
    *,
    target: str | None,
    data_root: str | None = None,
) -> DiagnosticsSaveResult | None:
    if not target:
        return None
    if target == "auto":
        paths = resolve_runtime_paths(data_root=data_root)
        directory = Path(paths.exports_root) / "diagnostics"
        filename = _diagnostics_filename(report.get("generated_at"))
        path = directory / filename
        relative_path = str(DIAGNOSTICS_SUBDIR / filename)
        target_label = "auto"
    else:
        path = Path(target).expanduser()
        relative_path = None
        target_label = "custom"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(json_ready(report), indent=2, sort_keys=False) + "\n", encoding="utf-8")
    return DiagnosticsSaveResult(path=path, relative_path=relative_path, target=target_label)


def write_error_diagnostics(
    args: Any,
    runtime: Any,
    exc: BaseException,
    *,
    stack: list[traceback.FrameSummary] | traceback.StackSummary | None,
    unhandled: bool,
) -> DiagnosticsSaveResult | None:
    target = getattr(args, "diagnostics_out", None)
    if not target:
        return None
    try:
        report = collect_public_diagnostics(
            getattr(runtime, "conn", None) if runtime is not None else None,
            args,
            runtime_config=getattr(runtime, "runtime_config", None)
            if runtime is not None
            else getattr(args, "runtime_config", None),
            error=exc,
            stack=stack,
            unhandled=unhandled,
        )
        result = save_public_diagnostics_report(
            report,
            target=target,
            data_root=getattr(args, "data_root", None),
        )
        if result:
            location = result.relative_path or str(result.path)
            print(f"diagnostics: wrote public report to {location}", file=sys.stderr)
        return result
    except Exception as write_error:  # pragma: no cover - best-effort error path
        print(
            f"diagnostics: failed to write public report: {sanitize_text(str(write_error))}",
            file=sys.stderr,
        )
        return None


def _safe_runtime_paths(args: Any):
    try:
        return resolve_runtime_paths(
            data_root=getattr(args, "data_root", None),
            env_file=getattr(args, "env_file", None),
        )
    except Exception:
        return None


def _privacy_contract() -> dict[str, Any]:
    return {
        "intended_use": "public_bug_report",
        "omits": [
            "addresses",
            "attachment filenames and URLs",
            "backend hostnames and full URLs",
            "descriptors and xpubs",
            "exact BTC, asset, fiat, fee, rate, balance, and tax values",
            "labels and notes",
            "local filesystem paths",
            "raw API, SQL, import, and wallet rows",
            "raw txids and transaction ids",
            "secrets and credential material",
            "stack frame local variables",
        ],
        "correlation_policy": "no stable hashes of sensitive values",
    }


def _environment_summary() -> dict[str, Any]:
    return {
        "kassiber_version": __version__,
        "machine_schema_version": SCHEMA_VERSION,
        "python_version": platform.python_version(),
        "python_implementation": platform.python_implementation(),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
        },
        "sqlite_version": sqlite3.sqlite_version,
    }


def _invocation_summary(args: Any) -> dict[str, Any]:
    kind = derive_kind(args)
    return {
        "command_path": kind.split(".") if kind else [],
        "output_format": getattr(args, "format", None),
        "machine": bool(getattr(args, "machine", False)),
        "debug_requested": bool(getattr(args, "debug", False)),
        "diagnostics_out_requested": bool(getattr(args, "diagnostics_out", None)),
        "provided_arguments": _argument_summary(args),
    }


def _argument_summary(args: Any) -> list[dict[str, Any]]:
    output = []
    subcommand_keys = {key for key in vars(args) if key.endswith("_command")}
    for key, value in sorted(vars(args).items()):
        if key in _SKIP_ARG_KEYS or key in subcommand_keys:
            continue
        if value in (None, False, [], ()):
            continue
        output.append({"name": key, **_argument_value_class(key, value)})
    return output


def _argument_value_class(key: str, value: Any) -> dict[str, Any]:
    if isinstance(value, bool):
        return {"value_class": "boolean", "value": value}
    if _is_sensitive_key(key):
        return {"value_class": "redacted"}
    if key in _SAFE_ENUM_KEYS and isinstance(value, str):
        return {"value_class": "enum", "value": sanitize_text(value)}
    if isinstance(value, (int, float)):
        return {"value_class": "number"}
    if isinstance(value, (list, tuple, set)):
        return {"value_class": "list", "items": len(value)}
    return {"value_class": "text", "value": sanitize_text(str(value))}


def _storage_summary(paths: Any) -> dict[str, Any]:
    if paths is None:
        return {
            "layout": "unknown",
            "paths_available": False,
            "diagnostics_location": str(DIAGNOSTICS_SUBDIR),
        }
    return {
        "layout": "app_state_root",
        "paths_available": True,
        "database_exists": Path(paths.database).exists(),
        "settings_file_exists": Path(paths.settings_file).exists(),
        "exports_root_exists": Path(paths.exports_root).exists(),
        "attachments_root_exists": Path(paths.attachments_root).exists(),
        "diagnostics_location": str(DIAGNOSTICS_SUBDIR),
    }


def _state_summary(conn: sqlite3.Connection | None, runtime_config: dict[str, Any]) -> dict[str, Any]:
    if conn is None:
        return {"available": False}
    return {
        "available": True,
        "counts": _entity_counts(conn),
        "profiles": _profile_summary(conn),
        "wallets": _wallet_summary(conn),
        "backends": _backend_summary(runtime_config),
        "transactions": _transaction_summary(conn),
        "journals": _journal_summary(conn),
        "rates": _rates_summary(conn),
        "attachments": _attachment_summary(conn),
        "manual_pairs": _manual_pair_summary(conn),
    }


def _entity_counts(conn: sqlite3.Connection) -> dict[str, int | None]:
    tables = (
        "workspaces",
        "profiles",
        "accounts",
        "wallets",
        "transactions",
        "journal_entries",
        "journal_quarantines",
        "transaction_pairs",
        "bip329_labels",
        "backends",
        "rates_cache",
        "attachments",
    )
    return {table: _count_rows(conn, table) for table in tables}


def _profile_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute(
        """
        SELECT
            p.id,
            p.tax_country,
            p.fiat_currency,
            p.gains_algorithm,
            p.last_processed_at,
            p.last_processed_tx_count,
            COUNT(t.id) AS active_transactions
        FROM profiles p
        LEFT JOIN transactions t ON t.profile_id = p.id AND t.excluded = 0
        GROUP BY p.id
        """
    ).fetchall()
    stale = 0
    processed = 0
    by_country = Counter()
    by_currency = Counter()
    by_algorithm = Counter()
    for row in rows:
        by_country[row["tax_country"] or "unknown"] += 1
        by_currency[row["fiat_currency"] or "unknown"] += 1
        by_algorithm[row["gains_algorithm"] or "unknown"] += 1
        if row["last_processed_at"] and row["active_transactions"] == row["last_processed_tx_count"]:
            processed += 1
        else:
            stale += 1
    return {
        "total": len(rows),
        "processed": processed,
        "stale_or_unprocessed": stale,
        "by_tax_country": _counter_rows(by_country, "tax_country"),
        "by_fiat_currency": _counter_rows(by_currency, "fiat_currency"),
        "by_gains_algorithm": _counter_rows(by_algorithm, "gains_algorithm"),
    }


def _wallet_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute("SELECT kind, config_json FROM wallets").fetchall()
    by_kind = Counter()
    by_chain_network = Counter()
    descriptor = 0
    change_descriptor = 0
    addresses = 0
    source_file = 0
    btcpay = 0
    configured_backend = 0
    invalid_config = 0
    for row in rows:
        by_kind[row["kind"] or "unknown"] += 1
        config = _load_json_object(row["config_json"])
        if config is None:
            invalid_config += 1
            config = {}
        chain = _safe_category(config.get("chain") or "unspecified")
        network = _safe_category(config.get("network") or "unspecified")
        by_chain_network[f"{chain}/{network}"] += 1
        descriptor += int(bool(config.get("descriptor")))
        change_descriptor += int(bool(config.get("change_descriptor")))
        addresses += int(bool(config.get("addresses")))
        source_file += int(bool(config.get("source_file")))
        btcpay += int(config.get("sync_source") == "btcpay")
        configured_backend += int(bool(config.get("backend")))
    return {
        "total": len(rows),
        "by_kind": _counter_rows(by_kind, "kind"),
        "by_chain_network": _counter_rows(by_chain_network, "chain_network"),
        "descriptor_configured": descriptor,
        "change_descriptor_configured": change_descriptor,
        "address_lists_configured": addresses,
        "source_files_configured": source_file,
        "btcpay_sync_configured": btcpay,
        "explicit_backend_configured": configured_backend,
        "invalid_config_rows": invalid_config,
    }


def _backend_summary(runtime_config: dict[str, Any]) -> dict[str, Any]:
    backends = runtime_config.get("backends") or {}
    by_kind = Counter()
    by_chain_network = Counter()
    by_source = Counter()
    by_locality = Counter()
    credentials = Counter()
    tor_proxy = 0
    for backend in backends.values():
        if not isinstance(backend, dict):
            continue
        by_kind[_safe_category(backend.get("kind") or "unknown")] += 1
        chain = _safe_category(backend.get("chain") or "unspecified")
        network = _safe_category(backend.get("network") or "unspecified")
        by_chain_network[f"{chain}/{network}"] += 1
        by_source[_safe_category(backend.get("source") or "unknown")] += 1
        by_locality[_backend_locality(backend.get("url"))] += 1
        for key in ("auth_header", "token", "username", "password", "cookiefile"):
            credentials[key] += int(bool(backend.get(key)))
        tor_proxy += int(bool(backend.get("tor_proxy")))
    return {
        "total": sum(by_kind.values()),
        "default_backend_configured": bool(runtime_config.get("default_backend")),
        "by_kind": _counter_rows(by_kind, "kind"),
        "by_chain_network": _counter_rows(by_chain_network, "chain_network"),
        "by_source": _counter_rows(by_source, "source"),
        "by_locality": _counter_rows(by_locality, "locality"),
        "credential_presence": dict(sorted(credentials.items())),
        "tor_proxy_configured": tor_proxy,
    }


def _transaction_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    by_direction = Counter()
    by_asset = Counter()
    excluded = 0
    priced = 0
    confirmed = 0
    with_notes = 0
    rows = conn.execute(
        "SELECT direction, asset, excluded, fiat_rate, fiat_value, confirmed_at, note FROM transactions"
    ).fetchall()
    for row in rows:
        by_direction[_safe_category(row["direction"] or "unknown")] += 1
        by_asset[_asset_bucket(row["asset"])] += 1
        excluded += int(bool(row["excluded"]))
        priced += int(row["fiat_rate"] is not None or row["fiat_value"] is not None)
        confirmed += int(bool(row["confirmed_at"]))
        with_notes += int(bool(row["note"]))
    return {
        "total": len(rows),
        "excluded": excluded,
        "priced": priced,
        "confirmed": confirmed,
        "with_notes": with_notes,
        "by_direction": _counter_rows(by_direction, "direction"),
        "by_asset_bucket": _counter_rows(by_asset, "asset_bucket"),
    }


def _journal_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    entries = conn.execute("SELECT entry_type, asset, at_category FROM journal_entries").fetchall()
    quarantines = conn.execute("SELECT reason FROM journal_quarantines").fetchall()
    by_entry_type = Counter()
    by_asset = Counter()
    by_at_category = Counter()
    by_quarantine_reason = Counter()
    for row in entries:
        by_entry_type[_safe_category(row["entry_type"] or "unknown")] += 1
        by_asset[_asset_bucket(row["asset"])] += 1
        by_at_category[_safe_category(row["at_category"] or "none")] += 1
    for row in quarantines:
        by_quarantine_reason[sanitize_text(row["reason"] or "unknown")] += 1
    return {
        "entries": len(entries),
        "quarantines": len(quarantines),
        "entries_by_type": _counter_rows(by_entry_type, "entry_type"),
        "entries_by_asset_bucket": _counter_rows(by_asset, "asset_bucket"),
        "entries_by_at_category": _counter_rows(by_at_category, "at_category"),
        "quarantines_by_reason": _counter_rows(by_quarantine_reason, "reason"),
    }


def _rates_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    row = conn.execute(
        """
        SELECT
            COUNT(*) AS samples,
            COUNT(DISTINCT pair) AS pairs,
            COUNT(DISTINCT source) AS sources
        FROM rates_cache
        """
    ).fetchone()
    return {
        "samples": row["samples"],
        "pairs": row["pairs"],
        "sources": row["sources"],
    }


def _attachment_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute("SELECT attachment_type, source_url, stored_relpath FROM attachments").fetchall()
    by_type = Counter()
    url_refs = 0
    stored_files = 0
    for row in rows:
        by_type[_safe_category(row["attachment_type"] or "unknown")] += 1
        url_refs += int(bool(row["source_url"]))
        stored_files += int(bool(row["stored_relpath"]))
    return {
        "total": len(rows),
        "by_type": _counter_rows(by_type, "attachment_type"),
        "url_references": url_refs,
        "stored_files": stored_files,
    }


def _manual_pair_summary(conn: sqlite3.Connection) -> dict[str, Any]:
    rows = conn.execute("SELECT kind, policy FROM transaction_pairs").fetchall()
    by_kind = Counter()
    by_policy = Counter()
    for row in rows:
        by_kind[_safe_category(row["kind"] or "unknown")] += 1
        by_policy[_safe_category(row["policy"] or "unknown")] += 1
    return {
        "total": len(rows),
        "by_kind": _counter_rows(by_kind, "kind"),
        "by_policy": _counter_rows(by_policy, "policy"),
    }


def _checks_summary(conn: sqlite3.Connection | None) -> dict[str, Any]:
    if conn is None:
        return {"database_integrity": "unavailable"}
    try:
        rows = conn.execute("PRAGMA integrity_check").fetchall()
    except sqlite3.Error:
        return {"database_integrity": "error"}
    if len(rows) == 1 and rows[0][0] == "ok":
        return {"database_integrity": "ok"}
    return {"database_integrity": "issues", "issue_count": len(rows)}


def _error_summary(error: BaseException, *, unhandled: bool) -> dict[str, Any]:
    return {
        "type": error.__class__.__name__,
        "code": sanitize_text(str(getattr(error, "code", "internal_error") or "internal_error")),
        "message": sanitize_text(str(error) or error.__class__.__name__),
        "hint": sanitize_text(getattr(error, "hint", None)),
        "details": sanitize_details(getattr(error, "details", None)),
        "retryable": bool(getattr(error, "retryable", False)),
        "unhandled": bool(unhandled),
    }


def _stack_summary(stack: list[traceback.FrameSummary] | traceback.StackSummary) -> list[dict[str, Any]]:
    frames = []
    for frame in stack:
        frames.append(
            {
                "module": _module_name_for_frame(frame.filename),
                "function": sanitize_text(frame.name),
                "line": frame.lineno,
            }
        )
    return frames[-20:]


def _module_name_for_frame(filename: str) -> str:
    path = Path(filename)
    try:
        resolved = path.resolve()
    except OSError:
        return "<unknown>"
    repo_root = Path(__file__).resolve().parent.parent
    try:
        rel = resolved.relative_to(repo_root)
        return ".".join(rel.with_suffix("").parts)
    except ValueError:
        pass
    parts = resolved.parts
    if "site-packages" in parts:
        index = parts.index("site-packages") + 1
        package_parts = parts[index : min(len(parts), index + 3)]
        if package_parts:
            return ".".join(Path(*package_parts).with_suffix("").parts)
    if "python" in resolved.name.lower():
        return "<python-runtime>"
    return "<external>"


def sanitize_details(value: Any) -> Any:
    if value is None:
        return None
    if isinstance(value, dict):
        sanitized = {}
        for key, item in value.items():
            safe_key = sanitize_text(str(key))
            if _is_sensitive_key(safe_key):
                sanitized[safe_key] = _redacted_shape(item)
            else:
                sanitized[safe_key] = sanitize_details(item)
        return sanitized
    if isinstance(value, (list, tuple, set)):
        return {"items": len(value)}
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return "<number>"
    return sanitize_text(str(value))


def sanitize_text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value)
    text = _URL_RE.sub("<url>", text)
    text = _TIMESTAMP_RE.sub("<timestamp>", text)
    text = _HEX64_RE.sub("<hex64>", text)
    text = _XPUB_RE.sub("<xpub>", text)
    text = _BECH32_RE.sub("<address>", text)
    text = _PATHISH_RE.sub("<path>", text)
    text = _AMOUNT_UNIT_RE.sub("<amount>", text)
    text = _NUMBER_RE.sub("<number>", text)
    return text


def _redacted_shape(value: Any) -> Any:
    if isinstance(value, dict):
        return {"keys": sorted(sanitize_text(str(key)) for key in value.keys())}
    if isinstance(value, (list, tuple, set)):
        return {"items": len(value)}
    if value is None:
        return None
    return "<redacted>"


def _count_rows(conn: sqlite3.Connection, table: str) -> int | None:
    try:
        return conn.execute(f"SELECT COUNT(*) AS count FROM {table}").fetchone()["count"]
    except sqlite3.Error:
        return None


def _load_json_object(value: Any) -> dict[str, Any] | None:
    try:
        loaded = json.loads(value or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return None
    return loaded if isinstance(loaded, dict) else None


def _counter_rows(counter: Counter, key: str) -> list[dict[str, Any]]:
    return [{key: item, "count": count} for item, count in sorted(counter.items())]


def _safe_category(value: Any) -> str:
    text = sanitize_text(str(value).strip().lower() if value is not None else "unknown")
    return text or "unknown"


def _asset_bucket(value: Any) -> str:
    asset = str(value or "").upper()
    if asset in {"BTC", "LBTC"}:
        return asset
    if not asset:
        return "unknown"
    return "other"


def _backend_locality(url: Any) -> str:
    if not url:
        return "missing"
    try:
        parts = urlsplit(str(url))
    except ValueError:
        return "invalid"
    hostname = parts.hostname
    if not hostname:
        return "unknown"
    lowered = hostname.lower()
    if lowered in {"localhost", "ip6-localhost"} or lowered.endswith(".localhost"):
        return "loopback"
    try:
        ip = ipaddress.ip_address(lowered.strip("[]"))
    except ValueError:
        if lowered.endswith(".local"):
            return "private_lan"
        return "remote"
    if ip.is_loopback:
        return "loopback"
    if ip.is_private:
        return "private_lan"
    return "remote"


def _is_sensitive_key(key: str) -> bool:
    lowered = key.lower().replace("-", "_")
    if lowered in _SENSITIVE_EXACT_KEYS:
        return True
    tokens = {part for part in lowered.split("_") if part}
    return bool(tokens & _SENSITIVE_KEY_TOKENS)


def _diagnostics_filename(generated_at: Any) -> str:
    stamp = sanitize_text(str(generated_at or now_iso())) or now_iso()
    stamp = stamp.replace(":", "-").replace("/", "-")
    stamp = re.sub(r"[^A-Za-z0-9_.-]", "-", stamp)
    return f"kassiber-diagnostics-{stamp}.json"
