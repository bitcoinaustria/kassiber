from __future__ import annotations

import csv
import ipaddress
import json
import sqlite3
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import urlparse

from . import pricing
from . import rates as core_rates
from .austrian import kennzahl_for_disposal_category
from .exit_tax import (  # re-exported so CLI/daemon reach exit-tax via core_reports.*
    build_exit_tax_report_lines,
    compute_deemed_disposal,
    format_exit_tax_lines,
    report_exit_tax,
)
from .privacy_linkage import analyze_psbt_privacy, build_privacy_linkage_graph
from ..errors import AppError
from ..msat import btc_to_msat, dec, msat_to_btc
from ..secrets.sqlcipher import looks_like_plaintext_sqlite
from ..tax_policy import require_tax_processing_supported
from ..transfers import apply_manual_pairs, detect_intra_transfers
from ..wallet_descriptors import normalize_asset_code

INTERVAL_CHOICES = ("hour", "day", "week", "month")
DEFAULT_BALANCE_HISTORY_INTERVAL = "month"
EUR_CENT = Decimal("0.01")
SWAP_FEE_PAIR_KINDS = (
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
)

AUSTRIAN_E1KV_REVIEW_GATE = (
    "Review this Austrian E 1kv export with a Steuerberater before filing; "
    "Kassiber is not tax advice."
)
AUSTRIAN_E1KV_SELF_CUSTODY_ASSUMPTION = (
    "Kassiber currently maps crypto rows to the auslaendisch / self-custody "
    "E 1kv Kennzahlen 172, 174, and 176. It does not populate domestic-provider "
    "or withheld-KESt fields because that metadata is not stored yet."
)
AUSTRIAN_E1KV_DETAIL_LIMITATION = (
    "Lot acquisition dates and holding-period day counts are not persisted in "
    "journal rows; the export relies on RP2's Austrian category classification "
    "and Kassiber's journal amounts."
)
AUSTRIAN_E1KV_REPROCESS_HINT = (
    "Capital-yield income (income_capital_yield) now maps to Kennzahl 172 "
    "instead of the old 175. Re-run `journals process` after upgrading so "
    "stored at_kennzahl values match this export; any mismatches are listed "
    "in the Data Quality section."
)
AUSTRIAN_E1KV_FORM_SECTION = "E 1kv 1.3.5 Einkuenfte aus Kryptowaehrungen"
AUSTRIAN_E1KV_KENNZAHL_LABELS = {
    172: "Auslaendische laufende Einkuenfte aus Kryptowaehrungen",
    174: "Auslaendische Ueberschuesse aus realisierten Wertsteigerungen",
    176: "Auslaendische realisierte Wertverluste",
    801: "Spekulationsgeschaefte Altbestand (outside E 1kv)",
}
AUSTRIAN_E1KV_SUPPORTED_KENNZAHL_ORDER = (172, 174, 176)
AUSTRIAN_OUTSIDE_E1KV_KENNZAHL_ORDER = (801,)
AUSTRIAN_KENNZAHL_FORM_BY_CODE = {
    172: "E 1kv",
    174: "E 1kv",
    176: "E 1kv",
    801: "E 1",
}
AUSTRIAN_KENNZAHL_FORM_SECTION_BY_CODE = {
    172: AUSTRIAN_E1KV_FORM_SECTION,
    174: AUSTRIAN_E1KV_FORM_SECTION,
    176: AUSTRIAN_E1KV_FORM_SECTION,
    801: "E 1 Spekulationsgeschaefte",
}
AUSTRIAN_E1KV_CATEGORY_LABELS = {
    "income_general": "Laufende Einkuenfte aus Kryptowaehrungen",
    "income_capital_yield": "Laufende Einkuenfte aus Ueberlassung von Kryptowaehrungen",
    "neu_gain": "Realisierte Wertsteigerung Neuvermoegen",
    "neu_loss": "Realisierter Wertverlust Neuvermoegen",
    "neu_swap": "Krypto-zu-Krypto Tausch mit Buchwertfortfuehrung",
    "alt_spekulation": "Altbestand innerhalb Spekulationsfrist",
    "alt_taxfree": "Altbestand ausserhalb Spekulationsfrist",
}
_TAX_SUMMARY_INCOME_TRANSACTION_TYPE_BY_KIND = {
    "airdrop": "airdrop",
    "hard_fork": "hardfork",
    "hardfork": "hardfork",
    "income": "income",
    "interest": "interest",
    "lending_interest": "interest",
    "mining": "mining",
    "mining_reward": "mining",
    "routing_income": "income",
    "staking": "staking",
    "wages": "wages",
}
AUSTRIAN_TAX_SECTION_ORDER = (
    "1.1",
    "1.2",
    "1.3",
    "2.1",
    "2.2",
    "3.1",
    "3.2",
    "3.3",
    "4.1",
    "4.2",
    "4.3",
    "4.4",
    "4.5",
)
AUSTRIAN_TAX_SECTION_METADATA = {
    "1.1": {
        "label": "Steuerpflichtige Einkuenfte aus dem Handel mit Kryptowaehrungen",
        "law": "27b Abs 3 EStG",
        "supported": True,
        "kennzahlen": (174, 176),
    },
    "1.2": {
        "label": "Steuerpflichtige Einkuenfte aus Margin, Derivaten und Futures",
        "law": "27 Z 4 EStG",
        "supported": False,
        "kennzahlen": (),
    },
    "1.3": {
        "label": "Steuerpflichtige Einkuenfte aus Spekulationsgeschaeften mit Kryptowaehrungen",
        "law": "31 EStG Altvermoegen innerhalb Spekulationsfrist",
        "supported": True,
        "kennzahlen": (801,),
    },
    "2.1": {
        "label": "Steuerpflichtige laufende Einkuenfte aus der Ueberlassung von Kryptowaehrungen",
        "law": "27b Abs 2 Z 1 EStG",
        "supported": True,
        "kennzahlen": (172,),
    },
    "2.2": {
        "label": "Steuerpflichtige laufende Einkuenfte aus Leistungen zur Transaktionsverarbeitung",
        "law": "27b Abs 2 Z 2 EStG",
        "supported": True,
        "kennzahlen": (172,),
    },
    "3.1": {
        "label": "Nicht steuerbare Einkuenfte aus Altvermoegen ausserhalb der Spekulationsfrist",
        "law": "31 EStG Altvermoegen",
        "supported": True,
        "kennzahlen": (),
    },
    "3.2": {
        "label": "Nicht steuerbare Einkuenfte mit Bewertung 0",
        "law": "27b Abs 2 Z 2 Satz 2 EStG",
        "supported": False,
        "kennzahlen": (),
    },
    "3.3": {
        "label": "Nicht steuerbare Steuergebuehren und Rueckerstattungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.1": {
        "label": "Eingegangene Spenden und Schenkungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.2": {
        "label": "Ausgegangene Spenden und Schenkungen",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.3": {
        "label": "Verlorene und gestohlene Coins",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.4": {
        "label": "Mining gewerblich",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
    "4.5": {
        "label": "Minting",
        "law": "",
        "supported": False,
        "kennzahlen": (),
    },
}
AUSTRIAN_TAX_SECTION_GROUPS = (
    (
        "1. Steuerpflichtige Einkuenfte aus dem Handel mit Kryptowaehrungen",
        ("1.1", "1.2", "1.3"),
    ),
    ("2. Steuerpflichtige laufende Einkuenfte", ("2.1", "2.2")),
    ("3. Nicht steuerbare Einkuenfte", ("3.1", "3.2", "3.3")),
    ("4. Sonstige Ein- und Ausgaenge", ("4.1", "4.2", "4.3", "4.4", "4.5")),
)

ScopeResolver = Callable[[sqlite3.Connection, str | None, str | None], tuple[Mapping[str, Any], Mapping[str, Any]]]
AccountResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
WalletResolver = Callable[[sqlite3.Connection, str, str], Mapping[str, Any]]
RequireProcessedJournals = Callable[[sqlite3.Connection, Mapping[str, Any]], None]
BuildLedgerState = Callable[[sqlite3.Connection, Mapping[str, Any]], Mapping[str, Any]]
ListJournalEntries = Callable[..., list[Mapping[str, Any]]]
ListWallets = Callable[..., list[Mapping[str, Any]]]
ParseIsoDateTime = Callable[[str | None, str], Any]
IsoFormatter = Callable[[Any], str]
NowIso = Callable[[], str]
FormatTable = Callable[..., list[str]]
WriteTextPdf = Callable[[str, str, Sequence[str]], Mapping[str, Any]]

PRIVACY_HYGIENE_SCHEMA_VERSION = 1
PRIVACY_HYGIENE_REDACTION = "ai_export_safe"
_PRIVACY_POSTURES = ("on_device", "self_hosted", "shielded", "remote", "unknown")
_PRIVACY_PROXY_AWARE_TRANSPORTS = {
    "bitcoinrpc",
    "btcpay",
    "electrum",
    "esplora",
    "liquid-esplora",
    "mempool",
}
_PRIVACY_SEVERITY_RANK = {"info": 1, "warning": 2, "alert": 3}
_BACKEND_SECRET_CONFIG_KEYS = {
    "certificate",
    "cookiefile",
    "commando_peer_id",
    "lightning_cli",
    "lightning_dir",
    "password",
    "rpc_file",
    "username",
}
_WALLET_WATCH_ONLY_CONFIG_KEYS = {
    "addresses",
    "change_descriptor",
    "descriptor",
    "samourai",
    "sp_descriptor",
    "xpub",
}


@dataclass(frozen=True)
class ReportHooks:
    resolve_scope: ScopeResolver
    resolve_account: AccountResolver
    resolve_wallet: WalletResolver
    require_processed_journals: RequireProcessedJournals
    build_ledger_state: BuildLedgerState
    list_journal_entries: ListJournalEntries
    list_wallets: ListWallets
    parse_iso_datetime: ParseIsoDateTime
    iso_z: IsoFormatter
    now_iso: NowIso
    format_table: FormatTable
    write_text_pdf: WriteTextPdf


def _resolve_report_scope(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    return workspace, profile


def _json_object(raw_json: Any) -> dict[str, Any]:
    try:
        value = json.loads(raw_json or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def _safe_count(
    conn: sqlite3.Connection,
    sql: str,
    params: Sequence[Any] = (),
    *,
    limitations: list[dict[str, Any]] | None = None,
    code: str | None = None,
) -> int | None:
    try:
        row = conn.execute(sql, tuple(params)).fetchone()
    except sqlite3.OperationalError as exc:
        if limitations is not None:
            limitations.append(
                {
                    "code": code or "count_unavailable",
                    "message": "A local database count could not be read.",
                    "evidence_level": "unknown",
                    "details": {"error": str(exc)},
                }
            )
        return None
    if row is None:
        return 0
    try:
        return int(row["count"] if "count" in row.keys() else row[0])
    except (KeyError, TypeError, ValueError, IndexError):
        return 0


def _main_database_path(conn: sqlite3.Connection) -> Path | None:
    try:
        rows = conn.execute("PRAGMA database_list").fetchall()
    except sqlite3.DatabaseError:
        return None
    for row in rows:
        try:
            name = row["name"]
            filename = row["file"]
        except (KeyError, TypeError, IndexError):
            name = row[1] if len(row) > 1 else None
            filename = row[2] if len(row) > 2 else None
        if name == "main" and filename:
            return Path(str(filename))
    return None


def _database_encryption_fact(conn: sqlite3.Connection) -> dict[str, Any]:
    db_path = _main_database_path(conn)
    if db_path is None:
        return {"status": "unknown", "evidence_level": "unknown"}
    try:
        exists = db_path.exists()
        size = db_path.stat().st_size if exists else 0
    except OSError:
        return {"status": "unknown", "evidence_level": "unknown"}
    if not exists or size == 0:
        return {"status": "missing_or_empty", "evidence_level": "exact"}
    plaintext = looks_like_plaintext_sqlite(db_path)
    return {
        "status": "plaintext" if plaintext else "encrypted_like",
        "evidence_level": "exact",
    }


def _endpoint_host(url: Any) -> str:
    raw = str(url or "").strip()
    if not raw:
        return ""
    parse_value = raw if "://" in raw else f"https://{raw}"
    try:
        parsed = urlparse(parse_value)
    except ValueError:
        return raw.lower().strip("[]")
    return (parsed.hostname or raw).lower().strip("[]")


def _is_onion_host(host: str) -> bool:
    return host.lower().endswith(".onion")


def _is_local_or_private_host(host: str) -> bool:
    normalized = (host or "").lower().strip("[]")
    if not normalized:
        return False
    if normalized in {"localhost", "0.0.0.0", "::1"}:
        return True
    if normalized.endswith(".local") or normalized.endswith(".localhost"):
        return True
    try:
        ip = ipaddress.ip_address(normalized)
    except ValueError:
        return False
    return bool(ip.is_loopback or ip.is_private or ip.is_link_local)


def _backend_privacy_posture(
    url: Any,
    *,
    has_proxy: bool,
    kind: str,
    ownership: Any,
) -> str:
    host = _endpoint_host(url)
    normalized_kind = str(kind or "").strip().lower()
    normalized_ownership = str(ownership or "").strip().lower()
    if _is_local_or_private_host(host):
        return "on_device"
    if normalized_ownership == "self":
        return "self_hosted"
    proxy_honored = bool(has_proxy) and (
        not normalized_kind or normalized_kind in _PRIVACY_PROXY_AWARE_TRANSPORTS
    )
    if _is_onion_host(host) or proxy_honored:
        return "shielded"
    if host:
        return "remote"
    return "unknown"


def _empty_posture_counts() -> dict[str, int]:
    return {posture: 0 for posture in _PRIVACY_POSTURES}


def _highest_privacy_severity(findings: Sequence[Mapping[str, Any]]) -> str:
    highest = "none"
    rank = 0
    for finding in findings:
        severity = str(finding.get("severity") or "")
        severity_rank = _PRIVACY_SEVERITY_RANK.get(severity, 0)
        if severity_rank > rank:
            highest = severity
            rank = severity_rank
    return highest


def _privacy_finding(
    findings: list[dict[str, Any]],
    *,
    finding_id: str,
    category: str,
    severity: str,
    title: str,
    detail: str,
    evidence_level: str,
    evidence: Mapping[str, Any],
    recommendation: str | None = None,
) -> None:
    findings.append(
        {
            "id": finding_id,
            "category": category,
            "severity": severity,
            "title": title,
            "detail": detail,
            "evidence_level": evidence_level,
            "evidence": dict(evidence),
            "recommendation": recommendation,
        }
    )


def _journal_freshness_fact(
    profile: Mapping[str, Any],
    active_transaction_count: int,
    journal_entry_count: int,
    quarantine_count: int,
) -> dict[str, Any]:
    last_processed_at = profile["last_processed_at"]
    last_processed_tx_count = int(profile["last_processed_tx_count"] or 0)
    journal_input_version = int(profile["journal_input_version"] or 0)
    last_processed_input_version = int(profile["last_processed_input_version"] or 0)
    if active_transaction_count == 0:
        status = "no_transactions"
    elif not last_processed_at:
        status = "not_processed"
    elif last_processed_tx_count != active_transaction_count:
        status = "stale"
    elif journal_input_version != last_processed_input_version:
        status = "stale"
    else:
        status = "current"
    return {
        "status": status,
        "needs_processing": status in {"not_processed", "stale"},
        "active_transaction_count": active_transaction_count,
        "journal_entry_count": journal_entry_count,
        "quarantine_count": quarantine_count,
        "evidence_level": "exact",
    }


def _backend_privacy_facts(conn: sqlite3.Connection) -> dict[str, Any]:
    postures = _empty_posture_counts()
    by_surface = {
        "bitcoin": _empty_posture_counts(),
        "liquid": _empty_posture_counts(),
        "lightning": _empty_posture_counts(),
        "market": _empty_posture_counts(),
        "other": _empty_posture_counts(),
    }
    credentialed = 0
    proxy_count = 0
    total = 0
    rows = conn.execute(
        """
        SELECT kind, chain, network, url, auth_header, token, tor_proxy, config_json
        FROM backends
        ORDER BY name ASC
        """
    ).fetchall()
    for row in rows:
        total += 1
        config = _json_object(row["config_json"])
        has_proxy = bool(str(row["tor_proxy"] or "").strip())
        proxy_count += 1 if has_proxy else 0
        if row["auth_header"] or row["token"] or any(config.get(key) for key in _BACKEND_SECRET_CONFIG_KEYS):
            credentialed += 1
        posture = _backend_privacy_posture(
            row["url"],
            has_proxy=has_proxy,
            kind=str(row["kind"] or ""),
            ownership=config.get("infrastructure_owner"),
        )
        postures[posture] = postures.get(posture, 0) + 1
        chain = str(row["chain"] or "").strip().lower()
        kind = str(row["kind"] or "").strip().lower()
        if kind in {"lnd", "coreln", "nwc"}:
            surface = "lightning"
        elif kind in {"coinbase-exchange", "coingecko", "mempool-rates"}:
            surface = "market"
        elif chain == "liquid" or kind == "liquid-esplora":
            surface = "liquid"
        elif chain == "bitcoin" or kind in {"bitcoinrpc", "btcpay", "electrum", "esplora", "mempool"}:
            surface = "bitcoin"
        else:
            surface = "other"
        by_surface.setdefault(surface, _empty_posture_counts())[posture] += 1
    return {
        "backend_count": total,
        "posture_counts": postures,
        "by_surface": by_surface,
        "credentialed_backend_count": credentialed,
        "proxy_configured_backend_count": proxy_count,
        "evidence_level": "derived",
    }


def _ai_provider_privacy_facts(conn: sqlite3.Connection) -> dict[str, Any]:
    provider_count = 0
    local_count = 0
    remote_count = 0
    tee_count = 0
    cli_count = 0
    credentialed_count = 0
    unacknowledged_off_device = 0
    rows = conn.execute(
        """
        SELECT name, base_url, api_key, kind, acknowledged_at
        FROM ai_providers
        ORDER BY name ASC
        """
    ).fetchall()
    for row in rows:
        provider_count += 1
        kind = str(row["kind"] or "").strip().lower()
        base_url = str(row["base_url"] or "").strip().lower()
        if base_url in {"claude-cli://default", "codex-cli://default"}:
            cli_count += 1
        elif kind == "local":
            local_count += 1
        elif kind == "tee":
            tee_count += 1
        else:
            remote_count += 1
        if row["api_key"]:
            credentialed_count += 1
        if kind != "local" and not row["acknowledged_at"]:
            unacknowledged_off_device += 1
    return {
        "provider_count": provider_count,
        "local_provider_count": local_count,
        "remote_provider_count": remote_count,
        "tee_provider_count": tee_count,
        "cli_provider_count": cli_count,
        "credentialed_provider_count": credentialed_count,
        "unacknowledged_off_device_provider_count": unacknowledged_off_device,
        "evidence_level": "exact",
    }


def _wallet_privacy_facts(
    conn: sqlite3.Connection,
    profile_id: str,
) -> dict[str, Any]:
    wallet_count = 0
    watch_only_material_wallet_count = 0
    backend_linked_wallet_count = 0
    descriptor_wallet_count = 0
    xpub_wallet_count = 0
    address_wallet_count = 0
    silent_payment_wallet_count = 0
    source_file_wallet_count = 0
    rows = conn.execute(
        """
        SELECT kind, config_json
        FROM wallets
        WHERE profile_id = ?
        ORDER BY kind ASC
        """,
        (profile_id,),
    ).fetchall()
    for row in rows:
        wallet_count += 1
        config = _json_object(row["config_json"])
        if config.get("backend"):
            backend_linked_wallet_count += 1
        if config.get("descriptor") or config.get("change_descriptor"):
            descriptor_wallet_count += 1
        if config.get("xpub"):
            xpub_wallet_count += 1
        if config.get("addresses"):
            address_wallet_count += 1
        if config.get("sp_descriptor"):
            silent_payment_wallet_count += 1
        if config.get("source_file"):
            source_file_wallet_count += 1
        if any(config.get(key) for key in _WALLET_WATCH_ONLY_CONFIG_KEYS):
            watch_only_material_wallet_count += 1
    return {
        "wallet_count": wallet_count,
        "watch_only_material_wallet_count": watch_only_material_wallet_count,
        "backend_linked_wallet_count": backend_linked_wallet_count,
        "descriptor_wallet_count": descriptor_wallet_count,
        "xpub_wallet_count": xpub_wallet_count,
        "address_wallet_count": address_wallet_count,
        "silent_payment_wallet_count": silent_payment_wallet_count,
        "source_file_wallet_count": source_file_wallet_count,
        "evidence_level": "exact",
    }


def report_privacy_hygiene(
    conn: sqlite3.Connection,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
) -> dict[str, Any]:
    """Return a redacted local privacy-hygiene payload for CLI/UI/AI parity.

    The payload is intentionally count- and category-only. It never includes
    addresses, scripts, descriptors, xpubs, backend URLs/tokens, wallet config,
    raw importer JSON, branch/index values, or derivation paths.
    """

    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    profile_id = profile["id"]
    limitations: list[dict[str, Any]] = [
        {
            "code": "local_only_no_probe",
            "message": "No network request was made; endpoint posture is inferred from local configuration only.",
            "evidence_level": "derived",
        },
        {
            "code": "redacted_payload",
            "message": (
                "Addresses, scripts, descriptors, xpubs, backend URLs/tokens, "
                "wallet config, raw_json, branch/index values, and derivation "
                "paths are omitted from this payload."
            ),
            "evidence_level": "exact",
        },
    ]
    findings: list[dict[str, Any]] = []

    database = _database_encryption_fact(conn)
    if database["status"] == "plaintext":
        _privacy_finding(
            findings,
            finding_id="database_plaintext",
            category="storage",
            severity="warning",
            title="Database file is plaintext SQLite",
            detail=(
                "The active database header is plaintext SQLite; SQLCipher "
                "encryption is not active for this data root."
            ),
            evidence_level="exact",
            evidence={"database_encryption": "plaintext"},
            recommendation="Use `kassiber secrets init` if this book should be encrypted at rest.",
        )
    elif database["status"] == "unknown":
        _privacy_finding(
            findings,
            finding_id="database_encryption_unknown",
            category="storage",
            severity="info",
            title="Database encryption status is unknown",
            detail="Kassiber could not classify the active database header from the local file.",
            evidence_level="unknown",
            evidence={"database_encryption": "unknown"},
        )

    network = _backend_privacy_facts(conn)
    remote_backends = int(network["posture_counts"].get("remote", 0))
    if remote_backends:
        _privacy_finding(
            findings,
            finding_id="remote_backend_endpoints",
            category="network",
            severity="warning",
            title="Third-party backend endpoints are configured",
            detail=(
                f"{remote_backends} configured backend endpoint(s) are inferred "
                "as third-party and can observe wallet/indexer queries."
            ),
            evidence_level="derived",
            evidence={"remote_backend_count": remote_backends},
            recommendation="Use your own node/backend or a per-backend proxy when that exposure is not acceptable.",
        )

    ai = _ai_provider_privacy_facts(conn)
    off_device_ai = int(ai["remote_provider_count"]) + int(ai["tee_provider_count"]) + int(ai["cli_provider_count"])
    if int(ai["unacknowledged_off_device_provider_count"]):
        _privacy_finding(
            findings,
            finding_id="unacknowledged_off_device_ai_provider",
            category="ai",
            severity="alert",
            title="Off-device AI provider needs acknowledgement",
            detail=(
                f"{ai['unacknowledged_off_device_provider_count']} off-device "
                "AI provider(s) lack an acknowledgement timestamp."
            ),
            evidence_level="exact",
            evidence={
                "unacknowledged_off_device_provider_count": ai[
                    "unacknowledged_off_device_provider_count"
                ]
            },
            recommendation="Review the provider posture before sending assistant prompts.",
        )
    elif off_device_ai:
        _privacy_finding(
            findings,
            finding_id="off_device_ai_provider_configured",
            category="ai",
            severity="warning",
            title="Off-device AI providers are configured",
            detail=(
                f"{off_device_ai} configured AI provider(s) may send prompt "
                "content outside this machine when selected."
            ),
            evidence_level="exact",
            evidence={"off_device_ai_provider_count": off_device_ai},
            recommendation="Keep the assistant on a local provider for local-only inference.",
        )

    wallets = _wallet_privacy_facts(conn, profile_id)
    if int(wallets["watch_only_material_wallet_count"]):
        _privacy_finding(
            findings,
            finding_id="watch_only_wallet_material_present",
            category="wallets",
            severity="info",
            title="Watch-only wallet material is stored locally",
            detail=(
                f"{wallets['watch_only_material_wallet_count']} wallet(s) have "
                "watch-only material in the local database."
            ),
            evidence_level="exact",
            evidence={
                "watch_only_material_wallet_count": wallets[
                    "watch_only_material_wallet_count"
                ]
            },
        )

    active_transactions = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
        limitations=limitations,
        code="transactions_count_unavailable",
    ) or 0
    excluded_transactions = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded != 0",
        (profile_id,),
        limitations=limitations,
        code="excluded_transactions_count_unavailable",
    ) or 0
    raw_json_transactions = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ?
          AND raw_json IS NOT NULL
          AND trim(raw_json) NOT IN ('', '{}', 'null')
        """,
        (profile_id,),
        limitations=limitations,
        code="transaction_raw_json_count_unavailable",
    ) or 0
    privacy_boundary_transactions = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM transactions
        WHERE profile_id = ?
          AND privacy_boundary IS NOT NULL
          AND trim(privacy_boundary) != ''
        """,
        (profile_id,),
        limitations=limitations,
        code="privacy_boundary_count_unavailable",
    ) or 0
    if privacy_boundary_transactions:
        _privacy_finding(
            findings,
            finding_id="privacy_boundary_transactions_present",
            category="transactions",
            severity="info",
            title="Privacy-boundary transaction markers are present",
            detail=(
                f"{privacy_boundary_transactions} transaction(s) carry a "
                "privacy-boundary marker; provenance across those hops remains "
                "opaque unless reviewed with stronger evidence."
            ),
            evidence_level="exact",
            evidence={"privacy_boundary_transaction_count": privacy_boundary_transactions},
        )
    if raw_json_transactions:
        _privacy_finding(
            findings,
            finding_id="import_raw_json_present",
            category="transactions",
            severity="info",
            title="Imported raw metadata is present",
            detail=(
                f"{raw_json_transactions} transaction(s) retain importer raw_json "
                "inside the local database; this payload reports only the count."
            ),
            evidence_level="exact",
            evidence={"transaction_raw_json_count": raw_json_transactions},
        )

    utxo_count = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM wallet_utxos WHERE profile_id = ?",
        (profile_id,),
        limitations=limitations,
        code="utxo_count_unavailable",
    ) or 0
    active_utxo_count = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM wallet_utxos WHERE profile_id = ? AND spent_at IS NULL",
        (profile_id,),
        limitations=limitations,
        code="active_utxo_count_unavailable",
    ) or 0
    utxos_with_address = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM wallet_utxos
        WHERE profile_id = ?
          AND address IS NOT NULL
          AND trim(address) != ''
        """,
        (profile_id,),
        limitations=limitations,
        code="utxo_address_count_unavailable",
    ) or 0
    utxos_with_script = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM wallet_utxos
        WHERE profile_id = ?
          AND script_pubkey IS NOT NULL
          AND trim(script_pubkey) != ''
        """,
        (profile_id,),
        limitations=limitations,
        code="utxo_script_count_unavailable",
    ) or 0
    utxos_with_derivation = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM wallet_utxos
        WHERE profile_id = ?
          AND (branch_index IS NOT NULL OR address_index IS NOT NULL)
        """,
        (profile_id,),
        limitations=limitations,
        code="utxo_derivation_count_unavailable",
    ) or 0
    if utxos_with_address or utxos_with_script or utxos_with_derivation:
        _privacy_finding(
            findings,
            finding_id="local_utxo_location_metadata_present",
            category="inventory",
            severity="info",
            title="Local UTXO location metadata is present",
            detail=(
                "The local output inventory contains address, script, or "
                "derivation metadata; this payload reports counts only."
            ),
            evidence_level="exact",
            evidence={
                "utxos_with_address_count": utxos_with_address,
                "utxos_with_script_count": utxos_with_script,
                "utxos_with_derivation_metadata_count": utxos_with_derivation,
            },
        )

    journal_entry_count = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM journal_entries WHERE profile_id = ?",
        (profile_id,),
        limitations=limitations,
        code="journal_entry_count_unavailable",
    ) or 0
    quarantine_count = _safe_count(
        conn,
        "SELECT COUNT(*) AS count FROM journal_quarantines WHERE profile_id = ?",
        (profile_id,),
        limitations=limitations,
        code="quarantine_count_unavailable",
    ) or 0
    privacy_quarantine_count = _safe_count(
        conn,
        """
        SELECT COUNT(*) AS count
        FROM journal_quarantines
        WHERE profile_id = ?
          AND reason LIKE 'privacy%'
        """,
        (profile_id,),
        limitations=limitations,
        code="privacy_quarantine_count_unavailable",
    ) or 0
    if privacy_quarantine_count:
        _privacy_finding(
            findings,
            finding_id="privacy_quarantines_present",
            category="journals",
            severity="warning",
            title="Privacy-related journal quarantines are present",
            detail=(
                f"{privacy_quarantine_count} journal quarantine(s) are "
                "privacy-related and need review before derived reports are complete."
            ),
            evidence_level="exact",
            evidence={"privacy_quarantine_count": privacy_quarantine_count},
        )
    journal_freshness = _journal_freshness_fact(
        profile,
        active_transactions,
        journal_entry_count,
        quarantine_count,
    )

    inventory = {
        "utxo_count": utxo_count,
        "active_utxo_count": active_utxo_count,
        "utxos_with_address_count": utxos_with_address,
        "utxos_with_script_count": utxos_with_script,
        "utxos_with_derivation_metadata_count": utxos_with_derivation,
        "evidence_level": "exact",
    }
    transactions = {
        "active_transaction_count": active_transactions,
        "excluded_transaction_count": excluded_transactions,
        "transaction_raw_json_count": raw_json_transactions,
        "privacy_boundary_transaction_count": privacy_boundary_transactions,
        "evidence_level": "exact",
    }
    journals = {
        **journal_freshness,
        "privacy_quarantine_count": privacy_quarantine_count,
    }
    summary = {
        "finding_count": len(findings),
        "highest_severity": _highest_privacy_severity(findings),
        "remote_backend_count": remote_backends,
        "off_device_ai_provider_count": off_device_ai,
        "wallet_count": wallets["wallet_count"],
        "watch_only_material_wallet_count": wallets["watch_only_material_wallet_count"],
        "active_transaction_count": active_transactions,
        "privacy_boundary_transaction_count": privacy_boundary_transactions,
        "privacy_quarantine_count": privacy_quarantine_count,
    }
    return {
        "payload_schema_version": PRIVACY_HYGIENE_SCHEMA_VERSION,
        "redaction": PRIVACY_HYGIENE_REDACTION,
        "local_only": True,
        "read_only": True,
        "scope": {
            "workspace": "active",
            "profile": "active",
            "workspace_selected": bool(workspace["id"]),
            "profile_selected": bool(profile["id"]),
        },
        "summary": summary,
        "facts": {
            "database": database,
            "network": network,
            "ai": ai,
            "wallets": wallets,
            "transactions": transactions,
            "inventory": inventory,
            "journals": journals,
        },
        "findings": findings,
        "limitations": limitations,
    }


def privacy_hygiene_table_rows(payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for finding in payload.get("findings", []) or []:
        if not isinstance(finding, Mapping):
            continue
        rows.append(
            {
                "severity": finding.get("severity"),
                "category": finding.get("category"),
                "finding": finding.get("id"),
                "evidence_level": finding.get("evidence_level"),
                "detail": finding.get("detail"),
            }
        )
    return rows


def _mirror_evidence_level(items: Sequence[Mapping[str, Any]]) -> str:
    levels = [str(item.get("evidence_level") or "exact") for item in items]
    if any(level == "unknown" for level in levels):
        return "unknown"
    if any(level == "derived" for level in levels):
        return "derived"
    return "exact"


def _mirror_severity_rank(value: Any) -> int:
    return {"alert": 3, "warning": 2, "info": 1}.get(str(value or ""), 0)


def _mirror_finding_candidates(
    graph_payload: Mapping[str, Any],
    hygiene_payload: Mapping[str, Any],
) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    for finding in graph_payload.get("findings", []) or []:
        if not isinstance(finding, Mapping):
            continue
        candidates.append(
            {
                "source": "linkage_graph",
                "id": finding.get("id"),
                "kind": finding.get("kind"),
                "severity": finding.get("severity"),
                "title": finding.get("title"),
                "detail": finding.get("detail"),
                "evidence_level": finding.get("evidence_level"),
                "evidence": finding.get("evidence") if isinstance(finding.get("evidence"), Mapping) else {},
            }
        )
    for finding in hygiene_payload.get("findings", []) or []:
        if not isinstance(finding, Mapping):
            continue
        candidates.append(
            {
                "source": "privacy_hygiene",
                "id": finding.get("id"),
                "kind": finding.get("category"),
                "severity": finding.get("severity"),
                "title": finding.get("title"),
                "detail": finding.get("detail"),
                "evidence_level": finding.get("evidence_level"),
                "evidence": finding.get("evidence") if isinstance(finding.get("evidence"), Mapping) else {},
            }
        )
    return candidates


def _privacy_mirror_worst_risk(
    graph_payload: Mapping[str, Any],
    hygiene_payload: Mapping[str, Any],
    unknowns: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    candidates = _mirror_finding_candidates(graph_payload, hygiene_payload)
    ranked = sorted(
        candidates,
        key=lambda item: (
            _mirror_severity_rank(item.get("severity")),
            str(item.get("id") or ""),
        ),
        reverse=True,
    )
    if ranked:
        top = ranked[0]
        title = str(top.get("title") or top.get("id") or "Privacy finding")
        detail = str(top.get("detail") or "A local privacy finding is present.")
        return {
            "kind": top.get("kind"),
            "severity": top.get("severity"),
            "title": title,
            "answer": f"Worst local privacy risk: {title}. {detail}",
            "evidence_level": top.get("evidence_level") or "unknown",
            "source": top.get("source"),
            "finding_id": top.get("id"),
            "evidence": dict(top.get("evidence") or {}),
        }
    if unknowns:
        first = unknowns[0]
        title = str(first.get("title") or first.get("code") or "Coverage gap")
        return {
            "kind": "unknown_coverage",
            "severity": "info",
            "title": title,
            "answer": (
                "Worst local privacy risk is unresolved coverage: Kassiber lacks "
                "enough local evidence to rule in or rule out stronger linkage."
            ),
            "evidence_level": "unknown",
            "source": first.get("source") or "privacy_mirror_unknowns",
            "finding_id": first.get("id") or first.get("code"),
            "evidence": {"unknown_count": len(unknowns)},
        }
    return {
        "kind": "bounded_local_model",
        "severity": "info",
        "title": "No stronger local finding in reduced model",
        "answer": (
            "No stronger local privacy finding is present in this reduced model; "
            "this is not a safety guarantee and does not include external lookup."
        ),
        "evidence_level": "derived",
        "source": "privacy_mirror_summary",
        "finding_id": None,
        "evidence": {"external_lookup": False},
    }


def _privacy_mirror_wallet_rows(graph_payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    nodes = [node for node in graph_payload.get("nodes", []) or [] if isinstance(node, Mapping)]
    edges = [edge for edge in graph_payload.get("edges", []) or [] if isinstance(edge, Mapping)]
    clusters_by_wallet: dict[str, set[str]] = defaultdict(set)
    linked_edges_by_wallet: dict[str, int] = defaultdict(int)
    wallet_rows: dict[str, dict[str, Any]] = {}
    node_wallet = {str(node.get("node_id")): str(node.get("wallet_id") or "") for node in nodes}
    for node in nodes:
        wallet_id = str(node.get("wallet_id") or "")
        row = wallet_rows.setdefault(
            wallet_id,
            {
                "wallet_id": wallet_id,
                "coin_count": 0,
                "amount_msat": 0,
                "change_like_coin_count": 0,
                "receive_coin_count": 0,
                "unknown_role_coin_count": 0,
                "linkage_edge_count": 0,
                "cluster_count": 0,
                "evidence_level": "exact",
            },
        )
        row["coin_count"] += 1
        row["amount_msat"] += int(node.get("amount_msat") or 0)
        role = str(node.get("branch_role") or "unknown")
        if role == "change":
            row["change_like_coin_count"] += 1
        elif role == "receive":
            row["receive_coin_count"] += 1
        else:
            row["unknown_role_coin_count"] += 1
            row["evidence_level"] = "unknown"
    for cluster in graph_payload.get("adversary_views", []) or []:
        if not isinstance(cluster, Mapping) or cluster.get("tier") != "passive_chain_watcher":
            continue
        for item in cluster.get("clusters", []) or []:
            if not isinstance(item, Mapping):
                continue
            cluster_id = str(item.get("cluster_id") or "")
            for wallet_id in item.get("wallet_ids", []) or []:
                clusters_by_wallet[str(wallet_id)].add(cluster_id)
    for edge in edges:
        for key in ("from_node_id", "to_node_id"):
            wallet_id = node_wallet.get(str(edge.get(key) or ""))
            if wallet_id:
                linked_edges_by_wallet[wallet_id] += 1
    for wallet_id, row in wallet_rows.items():
        row["cluster_count"] = len(clusters_by_wallet.get(wallet_id, set()))
        row["linkage_edge_count"] = linked_edges_by_wallet.get(wallet_id, 0)
    return sorted(wallet_rows.values(), key=lambda item: item["wallet_id"])


def _privacy_mirror_transaction_rows(graph_payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    by_txid: dict[str, dict[str, Any]] = {}
    for tell in graph_payload.get("transaction_tells", []) or []:
        if not isinstance(tell, Mapping):
            continue
        txid = str(tell.get("txid") or "unknown")
        row = by_txid.setdefault(
            txid,
            {
                "txid": txid,
                "tell_count": 0,
                "tell_kinds": [],
                "wallet_penalty_count": 0,
                "evidence_level": "exact",
                "sources": [],
            },
        )
        row["tell_count"] += 1
        row["tell_kinds"].append(tell.get("kind"))
        if tell.get("penalizes_wallet"):
            row["wallet_penalty_count"] += 1
        row["sources"].append(tell.get("source"))
        row["evidence_level"] = _mirror_evidence_level([row, tell])
    rows = []
    for row in by_txid.values():
        row["tell_kinds"] = sorted({str(kind) for kind in row["tell_kinds"] if kind})
        row["sources"] = sorted({str(source) for source in row["sources"] if source})
        rows.append(row)
    return sorted(rows, key=lambda item: (-item["tell_count"], item["txid"]))[:100]


def _privacy_mirror_utxo_rows(graph_payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    proximity = {
        str(fact.get("coin_id")): fact
        for fact in graph_payload.get("source_proximity", []) or []
        if isinstance(fact, Mapping)
    }
    rows: list[dict[str, Any]] = []
    for node in graph_payload.get("nodes", []) or []:
        if not isinstance(node, Mapping):
            continue
        fact = proximity.get(str(node.get("node_id")))
        evidence_items = [node]
        if isinstance(fact, Mapping):
            evidence_items.append(fact)
        rows.append(
            {
                "coin_id": node.get("node_id"),
                "txid": node.get("txid"),
                "vout": node.get("vout"),
                "wallet_id": node.get("wallet_id"),
                "asset": node.get("asset"),
                "amount_msat": node.get("amount_msat"),
                "branch_role": node.get("branch_role"),
                "change_evidence": node.get("change_evidence"),
                "source_proximity": fact.get("provenance_status") if isinstance(fact, Mapping) else "unknown_provenance",
                "evidence_level": _mirror_evidence_level(evidence_items),
            }
        )
    return sorted(rows, key=lambda item: str(item.get("coin_id") or ""))[:250]


def _privacy_mirror_timeline(graph_payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    for edge in graph_payload.get("edges", []) or []:
        if not isinstance(edge, Mapping):
            continue
        events.append(
            {
                "id": edge.get("edge_id"),
                "kind": edge.get("kind"),
                "category": "linkage",
                "txid": edge.get("txid"),
                "evidence_level": edge.get("evidence_level"),
                "detail": edge.get("heuristic"),
                "new_linkage": bool(edge.get("new_linkage")),
            }
        )
    for tell in graph_payload.get("transaction_tells", []) or []:
        if not isinstance(tell, Mapping):
            continue
        events.append(
            {
                "id": tell.get("tell_id"),
                "kind": tell.get("kind"),
                "category": "transaction_tell",
                "txid": tell.get("txid"),
                "evidence_level": tell.get("evidence_level"),
                "detail": tell.get("source"),
                "new_linkage": False,
            }
        )
    return sorted(events, key=lambda item: (str(item.get("txid") or ""), str(item.get("id") or "")))[:200]


def _privacy_mirror_unknowns(
    graph_payload: Mapping[str, Any],
    hygiene_payload: Mapping[str, Any],
) -> list[dict[str, Any]]:
    unknowns: list[dict[str, Any]] = []
    for source, payload in (
        ("linkage_graph", graph_payload),
        ("privacy_hygiene", hygiene_payload),
    ):
        for limitation in payload.get("limitations", []) or []:
            if not isinstance(limitation, Mapping):
                continue
            if limitation.get("evidence_level") == "unknown" or "unknown" in str(limitation.get("code") or ""):
                unknowns.append(
                    {
                        "source": source,
                        "code": limitation.get("code"),
                        "title": limitation.get("message") or limitation.get("code"),
                        "evidence_level": limitation.get("evidence_level") or "unknown",
                        "evidence": limitation.get("evidence") if isinstance(limitation.get("evidence"), Mapping) else {},
                    }
                )
    return unknowns


def _privacy_mirror_evidence_drilldowns(graph_payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    drilldowns: list[dict[str, Any]] = []
    for section, key in (
        ("findings", "id"),
        ("edges", "edge_id"),
        ("transaction_tells", "tell_id"),
        ("source_proximity", "coin_id"),
    ):
        for item in graph_payload.get(section, []) or []:
            if not isinstance(item, Mapping):
                continue
            drilldowns.append(
                {
                    "section": section,
                    "id": item.get(key),
                    "kind": item.get("kind") or item.get("provenance_status"),
                    "evidence_level": item.get("evidence_level") or "unknown",
                    "evidence": item.get("evidence") if isinstance(item.get("evidence"), Mapping) else {},
                }
            )
    return drilldowns[:250]


_PRIVACY_SCORE_LINKAGE_WEIGHT = 0.55
_PRIVACY_SCORE_LEAK_WEIGHT = 0.45

# Per-transaction leak weights, mirroring am-i-exposed's severity-graded
# heuristics for the tell kinds Kassiber actually emits into `tell_kinds`
# (verified in privacy_linkage.py `_load_transaction_tells`). Ownership-proving
# tells weigh most; wallet-software fingerprints and embedded metadata weigh
# little. Protective heuristics (CoinJoin, Taproot, entropy) are never leaks and
# never appear here. Unmapped/new tells get a small non-zero floor.
_PRIVACY_LEAK_TELL_WEIGHTS = {
    "sender_common_input": 1.0,  # mirrors h3: strongest same-owner link
    "fee_fingerprint": 0.3,  # mirrors h6: weak wallet-software fingerprint
    "sender_rbf": 0.3,  # mirrors h11: RBF wallet-fingerprint signal
    "op_return_output": 0.25,  # mirrors h7: embedded-metadata intent, not a link
}
_PRIVACY_LEAK_TELL_FLOOR = 0.2


def _tx_leak_weight(row: Mapping[str, Any]) -> float:
    kinds = row.get("tell_kinds") or []
    if not kinds:
        return 0.0
    return max(_PRIVACY_LEAK_TELL_WEIGHTS.get(str(kind), _PRIVACY_LEAK_TELL_FLOOR) for kind in kinds)


def _privacy_mirror_score(
    wallet_rows: Sequence[Mapping[str, Any]],
    transaction_rows: Sequence[Mapping[str, Any]],
    hygiene_summary: Mapping[str, Any],
    coverage_known: int,
    coverage_unknown: int,
) -> dict[str, Any]:
    """Grounded 0-100 privacy score (higher = more private).

    Derived from real local quantities via a fixed, documented formula rather
    than an arbitrary base. Two exposure fractions drive it:

    - wallet linkage: the share of wallets that can be tied to another wallet
      (a linked wallet has at least one common-input/linkage edge);
    - transaction leaks: the share of transactions that emit a linking tell.

    The score is ``100 - 100*(0.55*linkage_fraction + 0.45*leak_fraction)``.
    Uncertainty (coins whose origin is unknown) deliberately does NOT lower the
    score: it is reported separately as ``coverage_ratio`` so a confident score
    cannot silently hide missing data. Every factor ships its own counts so the
    number is fully explainable in the UI rather than opaque.
    """

    wallet_count = len(wallet_rows)
    linked_wallets = sum(
        1 for row in wallet_rows if int(row.get("linkage_edge_count") or 0) > 0
    )
    leaking_transactions = sum(
        1 for row in transaction_rows if int(row.get("tell_count") or 0) > 0
    )
    # Weight each transaction's leak by its strongest tell kind (MAX, not sum)
    # rather than a flat 1.0. MAX because a transaction's tells are correlated
    # (a multi-input send usually also carries a fee/OP_RETURN tell), so summing
    # would double-penalise one economic event; MAX = "this tx is at least this
    # linkable". This mirrors am-i-exposed's severity ordering locally.
    weighted_leak_sum = sum(_tx_leak_weight(row) for row in transaction_rows)
    active_transactions = int(hygiene_summary.get("active_transaction_count") or 0)
    transaction_total = max(active_transactions, leaking_transactions)

    linkage_fraction = (linked_wallets / wallet_count) if wallet_count else 0.0
    leak_fraction = (
        (weighted_leak_sum / transaction_total) if transaction_total else 0.0
    )

    linkage_points = round(100 * _PRIVACY_SCORE_LINKAGE_WEIGHT * linkage_fraction)
    leak_points = round(100 * _PRIVACY_SCORE_LEAK_WEIGHT * leak_fraction)
    value = max(0, min(100, 100 - linkage_points - leak_points))

    known_total = coverage_known + coverage_unknown
    coverage_ratio = (coverage_known / known_total) if known_total else 1.0

    return {
        "value": value,
        "base": 100,
        "evidence_level": "derived",
        "coverage_ratio": round(coverage_ratio, 3),
        "factors": [
            {
                "key": "wallet_linkage",
                "linked": linked_wallets,
                "total": wallet_count,
                "weight": _PRIVACY_SCORE_LINKAGE_WEIGHT,
                "points": -linkage_points,
            },
            {
                "key": "transaction_leaks",
                "leaking": leaking_transactions,
                "total": transaction_total,
                "weight": _PRIVACY_SCORE_LEAK_WEIGHT,
                "points": -leak_points,
            },
        ],
    }


def report_privacy_mirror(
    conn: sqlite3.Connection,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
) -> dict[str, Any]:
    """Return the north-star redacted Privacy Mirror payload.

    The payload composes local privacy hygiene facts with the Bitcoin UTXO
    linkage graph. It is advisory-only and omits raw wallet/PSBT/config data.
    """

    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    profile_id = profile["id"]
    graph_payload = build_privacy_linkage_graph(conn, profile_id).to_redacted_payload()
    hygiene_payload = report_privacy_hygiene(conn, workspace_ref, profile_ref, hooks)
    wallet_rows = _privacy_mirror_wallet_rows(graph_payload)
    transaction_rows = _privacy_mirror_transaction_rows(graph_payload)
    utxo_rows = _privacy_mirror_utxo_rows(graph_payload)
    timeline = _privacy_mirror_timeline(graph_payload)
    unknowns = _privacy_mirror_unknowns(graph_payload, hygiene_payload)
    evidence_drilldowns = _privacy_mirror_evidence_drilldowns(graph_payload)
    worst_risk = _privacy_mirror_worst_risk(graph_payload, hygiene_payload, unknowns)
    graph_summary = graph_payload.get("summary") if isinstance(graph_payload.get("summary"), Mapping) else {}
    hygiene_summary = hygiene_payload.get("summary") if isinstance(hygiene_payload.get("summary"), Mapping) else {}
    source_unknown = int(graph_summary.get("source_proximity_unknown_coin_count") or 0)
    source_known = int(graph_summary.get("source_proximity_known_coin_count") or 0)
    summary = {
        "evidence_level": _mirror_evidence_level(
            [
                {"evidence_level": graph_payload.get("redaction") and "derived"},
                worst_risk,
                *unknowns,
            ]
        ),
        "privacy_score": _privacy_mirror_score(
            wallet_rows,
            transaction_rows,
            hygiene_summary,
            source_known,
            source_unknown,
        ),
        "linkage_score": int(graph_summary.get("linkage_score") or 0),
        "linkable_cluster_count": int(graph_summary.get("observer_entity_count") or 0),
        "adversary_view_count": int(graph_summary.get("adversary_view_count") or 0),
        "wallet_count": len(wallet_rows),
        "transaction_tell_count": int(graph_summary.get("transaction_tell_count") or 0),
        "utxo_count": int(graph_summary.get("node_count") or 0),
        "unknown_count": len(unknowns) + source_unknown,
        "finding_count": int(graph_summary.get("finding_count") or 0) + int(hygiene_summary.get("finding_count") or 0),
        "worst_risk": worst_risk,
    }
    return {
        "payload_schema_version": 1,
        "redaction": "ai_export_safe",
        "local_only": True,
        "read_only": True,
        "advisory_only": True,
        "scope": {
            "workspace": "active",
            "profile": "active",
            "workspace_selected": bool(workspace["id"]),
            "profile_selected": bool(profile["id"]),
        },
        "summary": summary,
        "exposure_summary": {
            "evidence_level": summary["evidence_level"],
            "linkage": graph_summary,
            "hygiene": hygiene_summary,
        },
        "adversary_cards": graph_payload.get("adversary_views", []),
        "wallet_view": wallet_rows,
        "transaction_view": transaction_rows,
        "utxo_view": utxo_rows,
        "timeline": timeline,
        "psbt_what_if_panel": {
            "evidence_level": "derived",
            "status": "available_via_reports_psbt_privacy",
            "accepted_inputs": ["base64_psbt_text", "local_psbt_file"],
            "raw_psbt_exposed_to_ai": False,
            "notes": "Run reports psbt-privacy for a reduced pre-broadcast finding payload.",
        },
        "coverage": {
            "evidence_level": _mirror_evidence_level(unknowns),
            "source_proximity_known_coin_count": graph_summary.get("source_proximity_known_coin_count", 0),
            "source_proximity_unknown_coin_count": source_unknown,
            "unknown_coverage_count": len(unknowns),
            "degraded": bool(unknowns or source_unknown),
        },
        "unknowns": unknowns,
        "evidence_drilldowns": evidence_drilldowns,
        "methodology": {
            "evidence_level": "exact",
            "inputs": [
                "local wallet_utxos",
                "local transactions",
                "local reviewed source-funds links",
                "local privacy hygiene configuration counts",
            ],
            "heuristics": ["address_reuse", "common_input", "change_output"],
            "non_goals": [
                "no external lookup",
                "no entity attribution database",
                "no tax or accounting mutation",
                "no signing or broadcasting",
                "no transaction input proposal",
            ],
        },
        "limitations": [
            *graph_payload.get("limitations", []),
            *hygiene_payload.get("limitations", []),
        ],
    }


def privacy_mirror_table_rows(payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    worst = payload.get("summary", {}).get("worst_risk") if isinstance(payload.get("summary"), Mapping) else None
    if isinstance(worst, Mapping):
        rows.append(
            {
                "surface": "worst_risk",
                "severity": worst.get("severity"),
                "item": worst.get("finding_id") or worst.get("kind"),
                "evidence_level": worst.get("evidence_level"),
                "detail": worst.get("answer"),
            }
        )
    for card in payload.get("adversary_cards", []) or []:
        if not isinstance(card, Mapping):
            continue
        summary = card.get("summary") if isinstance(card.get("summary"), Mapping) else {}
        rows.append(
            {
                "surface": "adversary",
                "severity": "info",
                "item": card.get("tier"),
                "evidence_level": card.get("evidence_level"),
                "detail": f"exposed_cluster_count={summary.get('exposed_cluster_count', 0)} wallet_count={summary.get('wallet_count', 0)}",
            }
        )
    for unknown in payload.get("unknowns", []) or []:
        if not isinstance(unknown, Mapping):
            continue
        rows.append(
            {
                "surface": "unknown",
                "severity": "info",
                "item": unknown.get("code"),
                "evidence_level": unknown.get("evidence_level"),
                "detail": unknown.get("title"),
            }
        )
    return rows or [
        {
            "surface": "summary",
            "severity": "info",
            "item": "privacy_mirror",
            "evidence_level": payload.get("summary", {}).get("evidence_level") if isinstance(payload.get("summary"), Mapping) else "unknown",
            "detail": "Privacy Mirror payload is available; no row-level findings were emitted.",
        }
    ]


def report_psbt_privacy(
    conn: sqlite3.Connection,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    *,
    psbt_text: str,
) -> dict[str, Any]:
    workspace, profile = hooks.resolve_scope(conn, workspace_ref, profile_ref)
    try:
        analysis = analyze_psbt_privacy(conn, profile["id"], psbt_text)
    except ValueError as exc:
        raise AppError(
            "Could not decode the PSBT",
            code="validation",
            hint="Provide a base64 PSBT with a v0 unsigned transaction.",
            details={"reason": str(exc)},
            retryable=False,
        ) from exc
    payload = analysis.to_redacted_payload()
    payload["scope"] = {
        "workspace": "active",
        "profile": "active",
        "workspace_bound": bool(workspace["id"]),
        "profile_bound": bool(profile["id"]),
    }
    return payload


def psbt_privacy_table_rows(payload: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for finding in payload.get("findings", []) or []:
        if not isinstance(finding, Mapping):
            continue
        rows.append(
            {
                "severity": finding.get("severity"),
                "kind": finding.get("kind"),
                "result": finding.get("id"),
                "evidence_level": finding.get("evidence_level"),
                "detail": finding.get("detail"),
            }
        )
    if rows:
        return rows
    summary = payload.get("summary") if isinstance(payload.get("summary"), Mapping) else {}
    return [
        {
            "severity": "info",
            "kind": "psbt_privacy",
            "result": "summary",
            "evidence_level": summary.get("evidence_level"),
            "detail": (
                "Decoded PSBT privacy analysis produced no finding rows; "
                f"cluster_merge_delta={summary.get('cluster_merge_delta', 0)}, "
                f"unknown_input_count={summary.get('unknown_input_count', 0)}."
            ),
        }
    ]


def latest_transaction_rates_for_profile(conn, profile_id):
    try:
        rows = conn.execute(
            """
            SELECT asset, fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            ORDER BY occurred_at DESC, created_at DESC
            """,
            (profile_id,),
        ).fetchall()
    except sqlite3.OperationalError:
        rows = conn.execute(
            """
            SELECT asset, fiat_rate, fiat_value, amount
            FROM transactions
            WHERE profile_id = ? AND excluded = 0
            ORDER BY occurred_at DESC, created_at DESC
            """,
            (profile_id,),
        ).fetchall()
    rates = {}
    for row in rows:
        asset = row["asset"]
        if asset in rates:
            continue
        rate = row["fiat_rate_exact"] if "fiat_rate_exact" in row.keys() else None
        value = row["fiat_value_exact"] if "fiat_value_exact" in row.keys() else None
        rate_dec = dec(rate) if rate is not None else None
        value_dec = dec(value) if value is not None else None
        if rate_dec is None and row["fiat_rate"] is not None:
            rate_dec = dec(row["fiat_rate"])
        if value_dec is None and row["fiat_value"] is not None:
            value_dec = dec(row["fiat_value"])
        if rate_dec is not None:
            rates[asset] = rate_dec
        elif value_dec is not None and row["amount"]:
            rates[asset] = value_dec / msat_to_btc(row["amount"])
    return rates


def _profile_market_rate_assets(conn, profile_id):
    assets = set()
    for table in ("journal_account_holdings", "journal_wallet_holdings", "transactions"):
        try:
            rows = conn.execute(
                f"""
                SELECT DISTINCT asset
                FROM {table}
                WHERE profile_id = ?
                """,
                (profile_id,),
            ).fetchall()
        except sqlite3.OperationalError:
            continue
        for row in rows:
            asset = str(row["asset"] or "").strip().upper()
            if asset:
                assets.add(asset)
    return sorted(assets)


def latest_market_rates_for_profile(conn, profile, *, assets=None, fallback_rates=None):
    """Return current market rates for profile holdings, falling back to tx prices.

    Current portfolio reports should value remaining holdings at the latest
    cached market quote for the profile fiat currency. Transaction prices are
    still a useful fallback for older databases or books without a rate cache,
    but they are not the definition of "current market value."
    """
    profile_id = profile["id"]
    fiat_currency = profile["fiat_currency"]
    fallback = dict(fallback_rates or latest_transaction_rates_for_profile(conn, profile_id))
    if assets is None:
        asset_list = _profile_market_rate_assets(conn, profile_id)
        if not asset_list:
            asset_list = sorted(fallback)
    else:
        asset_list = sorted(
            {str(asset or "").strip().upper() for asset in assets if str(asset or "").strip()}
        )

    market_rates = {}
    for asset in asset_list:
        pair = core_rates.transaction_rate_pair(asset, fiat_currency)
        rate = None
        if pair is not None:
            try:
                cached_rate = core_rates.get_latest_rate(conn, pair)
            except sqlite3.OperationalError:
                cached_rate = None
            except AppError as exc:
                if exc.code != "not_found":
                    raise
                cached_rate = None
            if cached_rate is not None:
                rate = pricing.decimal_from_exact(cached_rate.get("rate_exact"), cached_rate.get("rate"))
        if rate is None:
            rate = fallback.get(asset)
        if rate is not None:
            market_rates[asset] = rate

    for asset, rate in fallback.items():
        market_rates.setdefault(asset, rate)
    return market_rates


def market_rates_for_profile_at_or_before(conn, profile, as_of, *, assets=None, fallback_rates=None):
    """Return market rates at ``as_of``, falling back to transaction prices.

    Historical portfolio views are "as of the selected report date", so cached
    market rates at or before that date take precedence over the latest
    transaction-specific import price. Transaction prices remain a fallback for
    books without a usable rate cache.
    """
    profile_id = profile["id"]
    fiat_currency = profile["fiat_currency"]
    fallback = dict(fallback_rates or latest_transaction_rates_for_profile(conn, profile_id))
    if assets is None:
        asset_list = _profile_market_rate_assets(conn, profile_id)
        if not asset_list:
            asset_list = sorted(fallback)
    else:
        asset_list = sorted(
            {str(asset or "").strip().upper() for asset in assets if str(asset or "").strip()}
        )

    market_rates = {}
    for asset in asset_list:
        pair = core_rates.transaction_rate_pair(asset, fiat_currency)
        rate = None
        if pair is not None:
            try:
                cached_rate = core_rates.get_cached_rate_at_or_before(conn, pair, as_of)
            except sqlite3.OperationalError:
                cached_rate = None
            except AppError as exc:
                if exc.code != "not_found":
                    raise
                cached_rate = None
            if cached_rate is not None:
                rate = pricing.decimal_from_exact(cached_rate.get("rate_exact"), cached_rate.get("rate"))
        if rate is None:
            rate = fallback.get(asset)
        if rate is not None:
            market_rates[asset] = rate

    for asset, rate in fallback.items():
        market_rates.setdefault(asset, rate)
    return market_rates


def _profile_has_journal_entries(conn, profile_id):
    row = conn.execute(
        "SELECT 1 FROM journal_entries WHERE profile_id = ? LIMIT 1",
        (profile_id,),
    ).fetchone()
    return row is not None


def report_balance_sheet(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    fallback_rates = None
    rows = []
    try:
        holding_rows = conn.execute(
            """
            SELECT account_code, account_label, asset, quantity, cost_basis
            FROM journal_account_holdings
            WHERE profile_id = ?
            ORDER BY account_code ASC, asset ASC, id ASC
            """,
            (profile["id"],),
        ).fetchall()
    except sqlite3.OperationalError:
        holding_rows = None
    if holding_rows is None or (not holding_rows and _profile_has_journal_entries(conn, profile["id"])):
        state = hooks.build_ledger_state(conn, profile)
        fallback_rates = state["latest_rates"]
        holding_rows = [
            {
                "account_code": account_code,
                "account_label": account_label,
                "asset": asset,
                "quantity": btc_to_msat(value["quantity"]),
                "cost_basis": value["cost_basis"],
            }
            for (_account_id, account_code, account_label, asset), value in state[
                "account_holdings"
            ].items()
        ]
    latest_rates = latest_market_rates_for_profile(
        conn,
        profile,
        assets=[row["asset"] for row in holding_rows],
        fallback_rates=fallback_rates,
    )
    for value in holding_rows:
        quantity = msat_to_btc(value["quantity"])
        if quantity <= 0:
            continue
        asset = value["asset"]
        cost_basis = dec(value["cost_basis"])
        latest_rate = latest_rates.get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        rows.append(
            {
                "account": value["account_code"] or value["account_label"],
                "asset": asset,
                "quantity": float(quantity),
                "quantity_sat": _msat_to_sat(btc_to_msat(quantity)),
                "quantity_msat": btc_to_msat(quantity),
                "cost_basis": float(cost_basis),
                "market_value": float(market_value),
                "unrealized_pnl": float(market_value - cost_basis),
            }
        )
    return rows


# Reconstructing holdings by summing raw journal_entries quantities must mirror
# the entry-type compensation baked into report_verify._holdings_quantity_formula.
# The engine books a self-transfer's network fee in TWO rows (transfer_out's
# quantity already includes the fee, and a separate transfer_fee disposes that
# same fee) and books earned coins in TWO rows (an `acquisition` lot plus an
# `income` recognition line). A naive Σ(quantity) therefore double-subtracts the
# fee and double-counts income, diverging from the BalanceSet-derived holdings
# tables the live (non-historical) reports read. These helpers keep the as-of
# portfolio and balance-history paths in lockstep with that canonical balance.
_HOLDINGS_QUANTITY_SKIP_ENTRY_TYPES = frozenset({"income", "transfer_fee"})
_HOLDINGS_BASIS_SKIP_ENTRY_TYPES = frozenset({"income"})


def _holdings_quantity_delta(entry_type, quantity):
    """Net-holdings quantity contribution of one journal entry.

    ``transfer_fee`` is skipped (its quantity is already inside ``transfer_out``'s
    ``sent``); ``income`` is skipped (the coins are already counted by their
    paired ``acquisition`` lot). Every other entry's stored quantity already
    carries the correct sign (acquisition / transfer_in positive,
    disposal / fee / transfer_out negative).
    """
    if entry_type in _HOLDINGS_QUANTITY_SKIP_ENTRY_TYPES:
        return Decimal("0")
    return quantity


def _holdings_basis_delta(entry_type, quantity, fiat_value, cost_basis):
    """Ending cost-basis contribution of one journal entry.

    Mirrors ``report_verify.basis_recompute``: add acquisition / transfer_in
    ``fiat_value``, subtract every sub-side ``cost_basis`` (disposal / fee /
    transfer_out / transfer_fee). ``income`` is skipped because its basis rides
    on the paired ``acquisition`` lot.
    """
    if entry_type in _HOLDINGS_BASIS_SKIP_ENTRY_TYPES:
        return Decimal("0")
    if quantity >= 0:
        return fiat_value
    return -cost_basis


def _historical_portfolio_summary(conn, profile, hooks: ReportHooks, as_of_dt, *, include_wallet_id=False):
    rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            je.wallet_id,
            w.label AS wallet,
            COALESCE(a.code, a.label, '') AS account,
            je.asset,
            je.entry_type,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
          AND je.occurred_at <= ?
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile["id"], hooks.iso_z(as_of_dt)),
    ).fetchall()
    rate_rows = conn.execute(
        """
        SELECT occurred_at, asset, amount, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND occurred_at <= ?
          AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL)
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"], hooks.iso_z(as_of_dt)),
    ).fetchall()

    holdings = defaultdict(
        lambda: {
            "quantity": Decimal("0"),
            "cost_basis": Decimal("0"),
        }
    )
    fallback_rates = {}
    for row in rate_rows:
        rate = None
        if row["fiat_rate"] is not None:
            rate = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rate = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
        if rate is not None:
            fallback_rates[row["asset"]] = rate

    for row in rows:
        quantity = msat_to_btc(row["quantity"])
        key = (row["wallet_id"], row["wallet"], row["account"], row["asset"])
        holdings[key]["quantity"] += _holdings_quantity_delta(row["entry_type"], quantity)
        holdings[key]["cost_basis"] += _holdings_basis_delta(
            row["entry_type"], quantity, dec(row["fiat_value"]), dec(row["cost_basis"])
        )

    # Per-wallet basis is an allocation, not the raw per-wallet journal sum:
    # a self-transfer's `transfer_out` carries no cost_basis and `transfer_in`
    # no fiat_value, so summing per wallet would strand the moved basis in the
    # source wallet and leave the destination with zero basis. Mirror the live
    # report's _accumulate_asset_holdings — price each wallet's residual quantity
    # at the asset's pooled average residual basis (the profile-scope basis total
    # is method-independent and matches report_verify). The profile-wide cost
    # basis is unchanged; only its per-wallet attribution becomes the allocation.
    pool_quantity = defaultdict(lambda: Decimal("0"))
    pool_cost_basis = defaultdict(lambda: Decimal("0"))
    for (_pwid, _pwlabel, _pacct, pool_asset), value in holdings.items():
        pool_quantity[pool_asset] += value["quantity"]
        pool_cost_basis[pool_asset] += value["cost_basis"]
    avg_basis_per_unit = {
        pool_asset: (pool_cost_basis[pool_asset] / pool_quantity[pool_asset])
        if pool_quantity[pool_asset] > 0
        else Decimal("0")
        for pool_asset in pool_quantity
    }
    latest_rates = market_rates_for_profile_at_or_before(
        conn,
        profile,
        hooks.iso_z(as_of_dt),
        assets=pool_quantity,
        fallback_rates=fallback_rates,
    )

    results = []
    for (wallet_id, wallet_label, account_code, asset), value in sorted(
        holdings.items(),
        key=lambda item: (item[0][1], item[0][3]),
    ):
        quantity = value["quantity"]
        if quantity <= 0:
            continue
        avg_cost = avg_basis_per_unit.get(asset, Decimal("0"))
        cost_basis = quantity * avg_cost
        latest_rate = latest_rates.get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        result = {
            "wallet": wallet_label,
            "account": account_code,
            "asset": asset,
            "quantity": float(quantity),
            "quantity_sat": _msat_to_sat(btc_to_msat(quantity)),
            "quantity_msat": btc_to_msat(quantity),
            "avg_cost": float(avg_cost),
            "cost_basis": float(cost_basis),
            "market_value": float(market_value),
            "unrealized_pnl": float(market_value - cost_basis),
        }
        if include_wallet_id:
            result["wallet_id"] = wallet_id
        results.append(result)
    return results


def report_portfolio_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks, as_of=None, *, include_wallet_id=False):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    if as_of is not None:
        as_of_dt = hooks.parse_iso_datetime(as_of, "as_of") if not isinstance(as_of, datetime) else as_of
        return _historical_portfolio_summary(conn, profile, hooks, as_of_dt, include_wallet_id=include_wallet_id)
    fallback_rates = None
    rows = []
    try:
        holding_rows = conn.execute(
            """
            SELECT wallet_id, wallet_label, account_code, asset, quantity, cost_basis
            FROM journal_wallet_holdings
            WHERE profile_id = ?
            ORDER BY wallet_label ASC, asset ASC, id ASC
            """,
            (profile["id"],),
        ).fetchall()
    except sqlite3.OperationalError:
        holding_rows = None
    if holding_rows is None or (not holding_rows and _profile_has_journal_entries(conn, profile["id"])):
        state = hooks.build_ledger_state(conn, profile)
        fallback_rates = state["latest_rates"]
        holding_rows = [
            {
                "wallet_id": wallet_id,
                "wallet_label": wallet_label,
                "account_code": account_code,
                "asset": asset,
                "quantity": btc_to_msat(value["quantity"]),
                "cost_basis": value["cost_basis"],
            }
            for (wallet_id, wallet_label, account_code, asset), value in state[
                "wallet_holdings"
            ].items()
        ]
    latest_rates = latest_market_rates_for_profile(
        conn,
        profile,
        assets=[row["asset"] for row in holding_rows],
        fallback_rates=fallback_rates,
    )
    for value in holding_rows:
        quantity = msat_to_btc(value["quantity"])
        if quantity <= 0:
            continue
        asset = value["asset"]
        cost_basis = dec(value["cost_basis"])
        latest_rate = latest_rates.get(asset, Decimal("0"))
        market_value = quantity * latest_rate
        avg_cost = cost_basis / quantity if quantity else Decimal("0")
        row = {
            "wallet": value["wallet_label"],
            "account": value["account_code"],
            "asset": asset,
            "quantity": float(quantity),
            "quantity_sat": _msat_to_sat(btc_to_msat(quantity)),
            "quantity_msat": btc_to_msat(quantity),
            "avg_cost": float(avg_cost),
            "cost_basis": float(cost_basis),
            "market_value": float(market_value),
            "unrealized_pnl": float(market_value - cost_basis),
        }
        if include_wallet_id:
            row["wallet_id"] = value["wallet_id"]
        rows.append(row)
    return rows


def report_capital_gains(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    where = [
        "je.profile_id = ?",
        "je.entry_type IN ('disposal', 'income')",
        "COALESCE(t.taxability_override, 1) != 0",
        "COALESCE(je.at_category, '') != 'neu_swap'",
    ]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        normalized_year = _normalize_tax_year(tax_year)
        where.append("substr(je.occurred_at, 1, 4) = ?")
        params.append(str(normalized_year))
    rows = conn.execute(
        f"""
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id,
            je.entry_type,
            je.asset,
            ABS(je.quantity) AS quantity,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl,
            je.capital_gains_type
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        params,
    ).fetchall()
    results = []
    for row in rows:
        entry = dict(row)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        if entry.get("at_category") is None:
            entry.pop("at_category", None)
        if entry.get("at_kennzahl") is None:
            entry.pop("at_kennzahl", None)
        results.append(entry)
    return results


def report_journal_entries(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    return hooks.list_journal_entries(conn, profile["workspace_id"], profile["id"], limit=None)


def _floor_to_interval(dt, interval):
    if interval == "hour":
        return dt.replace(minute=0, second=0, microsecond=0)
    if interval == "day":
        return dt.replace(hour=0, minute=0, second=0, microsecond=0)
    if interval == "week":
        floored = dt.replace(hour=0, minute=0, second=0, microsecond=0)
        return floored - timedelta(days=floored.weekday())
    if interval == "month":
        return dt.replace(day=1, hour=0, minute=0, second=0, microsecond=0)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def _next_interval(dt, interval):
    if interval == "hour":
        return dt + timedelta(hours=1)
    if interval == "day":
        return dt + timedelta(days=1)
    if interval == "week":
        return dt + timedelta(days=7)
    if interval == "month":
        if dt.month == 12:
            return dt.replace(year=dt.year + 1, month=1)
        return dt.replace(month=dt.month + 1)
    raise AppError(f"Unknown interval '{interval}'", code="validation")


def report_balance_history(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    interval=DEFAULT_BALANCE_HISTORY_INTERVAL,
    start=None,
    end=None,
    wallet_ref=None,
    account_ref=None,
    asset=None,
):
    if interval not in INTERVAL_CHOICES:
        raise AppError(
            f"Unsupported interval '{interval}'",
            code="validation",
            hint=f"Choose one of: {', '.join(INTERVAL_CHOICES)}",
        )
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    start_dt = hooks.parse_iso_datetime(start, "start")
    end_dt = hooks.parse_iso_datetime(end, "end")
    if start_dt and end_dt and start_dt > end_dt:
        raise AppError("--start must not be after --end", code="validation")

    sql = """
        SELECT
            je.occurred_at,
            je.asset,
            je.entry_type,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis
        FROM journal_entries je
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ?
    """
    params = [profile["id"]]
    if wallet_ref:
        wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref)
        sql += " AND je.wallet_id = ?"
        params.append(wallet["id"])
    if account_ref:
        account = hooks.resolve_account(conn, profile["id"], account_ref)
        sql += " AND je.account_id = ?"
        params.append(account["id"])
    if asset:
        sql += " AND je.asset = ?"
        params.append(asset)
    sql += " ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC"
    rows = conn.execute(sql, params).fetchall()
    scoped_basis_allocation = bool(wallet_ref or account_ref)
    pool_events = []
    if scoped_basis_allocation:
        # Scoped history reports quantity for the selected wallet/account, but
        # cost basis follows the profile-wide residual pool allocation used by
        # the live/as-of portfolio reports.
        pool_sql = """
            SELECT
                je.occurred_at,
                je.asset,
                je.entry_type,
                je.quantity,
                je.fiat_value,
                COALESCE(je.cost_basis, 0) AS cost_basis
            FROM journal_entries je
            WHERE je.profile_id = ?
        """
        pool_params = [profile["id"]]
        if asset:
            pool_sql += " AND je.asset = ?"
            pool_params.append(asset)
        pool_sql += " ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC"
        for row in conn.execute(pool_sql, pool_params).fetchall():
            pool_events.append(
                (
                    hooks.parse_iso_datetime(row["occurred_at"], "occurred_at"),
                    row["asset"],
                    row["entry_type"],
                    msat_to_btc(row["quantity"]),
                    dec(row["fiat_value"]),
                    dec(row["cost_basis"]),
                )
            )
    rate_rows = conn.execute(
        """
        SELECT occurred_at, asset, amount, fiat_rate, fiat_value
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND (fiat_rate IS NOT NULL OR fiat_value IS NOT NULL)
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"],),
    ).fetchall()

    if not rows and not (start_dt and end_dt):
        return []

    events = []
    for row in rows:
        row_dt = hooks.parse_iso_datetime(row["occurred_at"], "occurred_at")
        events.append(
            (
                row_dt,
                row["asset"],
                row["entry_type"],
                msat_to_btc(row["quantity"]),
                dec(row["fiat_value"]),
                dec(row["cost_basis"]),
            )
        )
    rate_events = []
    for row in rate_rows:
        rate = None
        if row["fiat_rate"] is not None:
            rate = dec(row["fiat_rate"])
        elif row["fiat_value"] is not None and row["amount"]:
            rate = dec(row["fiat_value"]) / msat_to_btc(row["amount"])
        if rate is None:
            continue
        rate_events.append((hooks.parse_iso_datetime(row["occurred_at"], "occurred_at"), row["asset"], rate))

    first_event_dt = events[0][0] if events else None
    range_start = start_dt or first_event_dt or datetime.now(timezone.utc)
    range_end = end_dt or datetime.now(timezone.utc)
    if range_start > range_end:
        return []

    rate_assets = {event[1] for event in events}
    if asset:
        rate_assets.add(asset)
    cached_rate_events = []
    for rate_asset in sorted(rate_assets):
        pair = core_rates.transaction_rate_pair(rate_asset, profile["fiat_currency"])
        if pair is None:
            continue
        try:
            rate_cache_rows = conn.execute(
                """
                SELECT timestamp, rate, rate_exact
                FROM rates_cache
                WHERE pair = ?
                  AND timestamp <= ?
                ORDER BY timestamp ASC,
                         CASE WHEN source = 'manual' THEN 1 ELSE 0 END ASC,
                         fetched_at ASC,
                         source DESC
                """,
                (pair, hooks.iso_z(range_end)),
            ).fetchall()
        except sqlite3.OperationalError:
            rate_cache_rows = []
        for row in rate_cache_rows:
            rate = pricing.decimal_from_exact(row["rate_exact"], row["rate"])
            if rate is None:
                continue
            cached_rate_events.append(
                (hooks.parse_iso_datetime(row["timestamp"], "rate_timestamp"), rate_asset, rate)
            )
    cached_rate_events.sort(key=lambda item: item[0])

    cumulative = defaultdict(lambda: Decimal("0"))
    cumulative_fiat = defaultdict(lambda: Decimal("0"))
    pool_quantity = defaultdict(lambda: Decimal("0"))
    pool_cost_basis = defaultdict(lambda: Decimal("0"))
    event_idx = 0
    pool_event_idx = 0
    rate_idx = 0
    cached_rate_idx = 0
    current_rates = {}
    cached_rates = {}
    bucket_start = _floor_to_interval(range_start, interval)
    end_cap = _floor_to_interval(range_end, interval)

    results = []
    while bucket_start <= end_cap:
        bucket_end = _next_interval(bucket_start, interval)
        while event_idx < len(events) and events[event_idx][0] < bucket_end:
            _, ev_asset, ev_entry_type, ev_qty, ev_fiat, ev_cost_basis = events[event_idx]
            cumulative[ev_asset] += _holdings_quantity_delta(ev_entry_type, ev_qty)
            cumulative_fiat[ev_asset] += _holdings_basis_delta(
                ev_entry_type, ev_qty, ev_fiat, ev_cost_basis
            )
            event_idx += 1
        while (
            scoped_basis_allocation
            and pool_event_idx < len(pool_events)
            and pool_events[pool_event_idx][0] < bucket_end
        ):
            _, ev_asset, ev_entry_type, ev_qty, ev_fiat, ev_cost_basis = pool_events[
                pool_event_idx
            ]
            pool_quantity[ev_asset] += _holdings_quantity_delta(ev_entry_type, ev_qty)
            pool_cost_basis[ev_asset] += _holdings_basis_delta(
                ev_entry_type, ev_qty, ev_fiat, ev_cost_basis
            )
            pool_event_idx += 1
        while rate_idx < len(rate_events) and rate_events[rate_idx][0] < bucket_end:
            _, rate_asset, rate = rate_events[rate_idx]
            current_rates[rate_asset] = rate
            rate_idx += 1
        while cached_rate_idx < len(cached_rate_events) and cached_rate_events[cached_rate_idx][0] < bucket_end:
            _, rate_asset, rate = cached_rate_events[cached_rate_idx]
            cached_rates[rate_asset] = rate
            cached_rate_idx += 1
        emitted_assets = set(cumulative.keys()) if asset is None else {asset}
        for ev_asset in sorted(emitted_assets):
            qty = cumulative.get(ev_asset, Decimal("0"))
            if qty == 0 and asset is None:
                continue
            rate = cached_rates.get(ev_asset, current_rates.get(ev_asset, Decimal("0")))
            if scoped_basis_allocation:
                pool_qty = pool_quantity.get(ev_asset, Decimal("0"))
                if pool_qty > 0:
                    cost_basis = qty * (
                        pool_cost_basis.get(ev_asset, Decimal("0")) / pool_qty
                    )
                else:
                    cost_basis = Decimal("0")
            else:
                cost_basis = cumulative_fiat.get(ev_asset, Decimal("0"))
            results.append(
                {
                    "period_start": hooks.iso_z(bucket_start),
                    "period_end": hooks.iso_z(bucket_end - timedelta(seconds=1)),
                    "asset": ev_asset,
                    "quantity": float(qty),
                    "cumulative_cost_basis": float(cost_basis),
                    "market_value": float(qty * rate),
                }
            )
        bucket_start = bucket_end
    return results


def _report_kv_lines(pairs, label_width=28):
    return [f"{label + ':':<{label_width}} {value}" for label, value in pairs]


def _report_btc(value):
    return f"{float(value):,.8f}"


def _report_fiat(value):
    return f"{float(value):,.2f}"


def _report_count(value):
    return f"{int(value or 0):,}"


def _markdown_table_cell(value):
    return str(value if value is not None else "").replace("|", "/").replace("\n", " ")


def _markdown_table_lines(headers, rows):
    lines = [
        "| " + " | ".join(_markdown_table_cell(header) for header in headers) + " |",
        "| " + " | ".join("---" for _header in headers) + " |",
    ]
    for row in rows:
        lines.append("| " + " | ".join(_markdown_table_cell(cell) for cell in row) + " |")
    return lines


def _aggregate_balance_rows_from_portfolio(portfolio_rows):
    grouped = {}
    for row in portfolio_rows:
        key = (row["account"], row["asset"])
        bucket = grouped.setdefault(
            key,
            {
                "account": row["account"],
                "asset": row["asset"],
                "quantity": 0.0,
                "cost_basis": 0.0,
                "market_value": 0.0,
                "unrealized_pnl": 0.0,
            },
        )
        bucket["quantity"] += float(row["quantity"])
        bucket["cost_basis"] += float(row["cost_basis"])
        bucket["market_value"] += float(row["market_value"])
        bucket["unrealized_pnl"] += float(row["unrealized_pnl"])
    return [
        grouped[key]
        for key in sorted(grouped.keys(), key=lambda item: (item[0] or "", item[1] or ""))
    ]


def _scope_wallets(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet=None):
    wallets = hooks.list_wallets(conn, workspace_ref, profile_ref)
    if wallet is None:
        return wallets
    return [row for row in wallets if row["id"] == wallet["id"]]


def _resolve_wallet_scope_refs(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_refs=None):
    all_wallets = hooks.list_wallets(conn, workspace_ref, profile_ref)
    if wallet_refs is None:
        return all_wallets
    refs = [str(ref).strip() for ref in wallet_refs if str(ref).strip()]
    if not refs:
        raise AppError("Summary PDF requires at least one wallet in scope", code="validation")
    by_id = {str(row["id"]): row for row in all_wallets}
    by_label = {str(row["label"]): row for row in all_wallets}
    resolved = []
    seen = set()
    for ref in refs:
        wallet = by_id.get(ref) or by_label.get(ref)
        if wallet is None:
            wallet = hooks.resolve_wallet(conn, profile_ref, ref)
        wallet_id = str(wallet["id"])
        if wallet_id in seen:
            continue
        seen.add(wallet_id)
        resolved.append(wallet)
    return resolved


def _wallet_scope_sql(column: str, wallets: Sequence[Mapping[str, Any]]) -> tuple[str, list[Any]]:
    ids = [row["id"] for row in wallets]
    if not ids:
        raise AppError("Summary PDF requires at least one wallet in scope", code="validation")
    placeholders = ",".join("?" for _ in ids)
    return f"{column} IN ({placeholders})", ids


def _attachment_entry(row):
    """Normalize an attachment row into a display entry.

    ``display_name`` is the human label shown in the sheet (the URL/file is the
    link target); ``url`` is empty for file attachments.
    """
    label = (row["label"] or "").strip()
    url = (row["url"] or "").strip()
    filename = (row["filename"] or "").strip()
    is_url = str(row["kind"]).lower() == "url"
    if is_url:
        display_name = label or url or "(link)"
        reference = url
    else:
        display_name = filename or label or "(file)"
        reference = filename
    return {
        "kind": "url" if is_url else "file",
        "label": label,
        "display_name": display_name,
        "url": url if is_url else "",
        "reference": reference,
    }


def _journals_current(conn, profile_id):
    """Whether the profile's journals reflect the current transaction inputs.

    Mirrors the CLI's freshness check (``last_processed_at`` set, active
    transaction count unchanged, input version unchanged) without a back-edge
    into the CLI layer. ``invalidate_journals`` leaves old ``journal_entries``
    rows in place, so a stale profile still has entries that must not be
    exported as current figures.
    """
    row = conn.execute(
        """
        SELECT last_processed_at, last_processed_tx_count,
               journal_input_version, last_processed_input_version
        FROM profiles WHERE id = ?
        """,
        (profile_id,),
    ).fetchone()
    if row is None or not row["last_processed_at"]:
        return False
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile_id,),
    ).fetchone()["count"]
    return int(current_count or 0) == int(row["last_processed_tx_count"] or 0) and int(
        row["journal_input_version"] or 0
    ) == int(row["last_processed_input_version"] or 0)


def _transaction_journal_values(conn, profile, wallet=None):
    """Aggregate per-transaction cost basis and realized gain/loss from the
    processed journal.

    Returns a map of transaction id -> ``{"cost_basis", "gain_loss"}`` where each
    value is ``None`` when no journal entry for that transaction carries the
    figure (e.g. a plain acquisition has no realized basis or gain yet), so the
    export leaves those cells blank rather than showing a misleading ``0``.
    The caller must gate on ``_journals_current``: stale journals (never
    processed, or invalidated by later edits/imports) still have leftover
    ``journal_entries`` rows, and exporting those would present outdated
    figures as current.

    Only realized tax rows count, with the same filters as
    ``report_capital_gains``: ``fee`` / ``transfer_fee`` entries carry a basis
    reduction with proceeds set equal to it and zero gain, so summing them
    would inflate ``cost_basis`` without moving ``gain_loss`` and the row
    would no longer reconcile against the Capital Detail sheet.
    """
    filters = [
        "je.profile_id = ?",
        "je.entry_type IN ('disposal', 'income')",
        "COALESCE(t.taxability_override, 1) != 0",
        "COALESCE(je.at_category, '') != 'neu_swap'",
    ]
    params: list[Any] = [profile["id"]]
    if wallet:
        filters.append("je.wallet_id = ?")
        params.append(wallet["id"])
    where = " AND ".join(filters)
    rows = conn.execute(
        f"""
        SELECT
            je.transaction_id AS tx_id,
            SUM(COALESCE(je.cost_basis, 0)) AS cost_basis,
            SUM(COALESCE(je.gain_loss, 0)) AS gain_loss,
            SUM(CASE WHEN je.cost_basis IS NOT NULL THEN 1 ELSE 0 END) AS basis_entries,
            SUM(CASE WHEN je.gain_loss IS NOT NULL THEN 1 ELSE 0 END) AS gain_entries
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE {where}
        GROUP BY je.transaction_id
        """,
        params,
    ).fetchall()
    values = {}
    for row in rows:
        values[row["tx_id"]] = {
            "cost_basis": float(row["cost_basis"]) if row["basis_entries"] else None,
            "gain_loss": float(row["gain_loss"]) if row["gain_entries"] else None,
        }
    return values


def _self_transfer_legs_by_transaction(conn, profile, journals_current=False):
    """Map each transaction that is one leg of a transfer between the user's own
    wallets to the counterparty wallet label(s).

    With current journals the processed ledger is authoritative: labels come
    from the booked ``transfer_out`` / ``transfer_in`` entries themselves
    (written pairwise per transfer audit with a shared ``occurred_at`` +
    description that embeds the wallet pair), which covers every MOVE the
    engine actually accepted — same-txid pairs, Lightning hash pairs,
    ownership-derived pairs, consolidations, and split-swap change legs — and
    nothing it withheld, quarantined, or replaced with a direct-payout
    disposal. Reviewed carrying-value cross-asset pairs never book transfer
    entries (they carry through the swap path), so those are labeled from the
    pair records unless a leg quarantined.

    Without current journals a best-effort heuristic reuses the same pure
    detection + manual-pair layer as the journal pipeline
    (``detect_intra_transfers`` + ``apply_manual_pairs``), including the
    engine's whole-row direct-payout pruning. Detection spans the whole
    profile (a transfer is two wallets), so this always works on the full
    profile row set even when the export is wallet-scoped.
    """
    rows = conn.execute(
        """
        SELECT
            t.id AS id,
            t.external_id AS external_id,
            t.asset AS asset,
            t.direction AS direction,
            t.amount AS amount,
            t.wallet_id AS wallet_id,
            t.payment_hash AS payment_hash,
            w.label AS wallet_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ? AND t.excluded = 0
        """,
        (profile["id"],),
    ).fetchall()
    rows_by_id = {row["id"]: row for row in rows}
    manual_records = conn.execute(
        "SELECT * FROM transaction_pairs WHERE profile_id = ? AND deleted_at IS NULL",
        (profile["id"],),
    ).fetchall()

    # A transaction can be the counterparty of several legs (consolidations,
    # fan-outs, a split swap's change + swap), so collect label sets and render
    # a deterministic comma-joined cell.
    label_sets: dict[str, set[str]] = {}

    def _add(tx_id, labels):
        label_sets.setdefault(tx_id, set()).update(labels)

    def _carrying_value_cross_asset_pairs():
        # Only carrying-value cross-asset pairs are reviewed as own-wallet
        # moves; `--policy taxable` pairs stay SELL + BUY in the engine and
        # must not be labeled as internal transfers.
        for record in manual_records:
            if record["policy"] != "carrying-value":
                continue
            out_row = rows_by_id.get(record["out_transaction_id"])
            in_row = rows_by_id.get(record["in_transaction_id"])
            if out_row is None or in_row is None or out_row["asset"] == in_row["asset"]:
                continue
            yield out_row, in_row

    if journals_current:
        entry_rows = conn.execute(
            """
            SELECT je.transaction_id AS tx_id, je.entry_type, je.occurred_at,
                   COALESCE(je.description, '') AS description,
                   w.label AS wallet_label
            FROM journal_entries je
            JOIN wallets w ON w.id = je.wallet_id
            WHERE je.profile_id = ?
              AND je.entry_type IN ('transfer_out', 'transfer_in')
            """,
            (profile["id"],),
        ).fetchall()
        audit_groups: dict[tuple[str, str], list] = {}
        for row in entry_rows:
            audit_groups.setdefault((row["occurred_at"], row["description"]), []).append(row)
        for group in audit_groups.values():
            outs = [row for row in group if row["entry_type"] == "transfer_out"]
            ins = [row for row in group if row["entry_type"] == "transfer_in"]
            if not outs or not ins:
                continue
            out_wallets = {row["wallet_label"] for row in outs}
            in_wallets = {row["wallet_label"] for row in ins}
            for row in outs:
                _add(row["tx_id"], in_wallets)
            for row in ins:
                _add(row["tx_id"], out_wallets)
        quarantined_ids = {
            row["transaction_id"]
            for row in conn.execute(
                "SELECT transaction_id FROM journal_quarantines WHERE profile_id = ?",
                (profile["id"],),
            ).fetchall()
        }
        for out_row, in_row in _carrying_value_cross_asset_pairs():
            # A pair the engine rejected during processing (e.g. the swap
            # validator quarantined a leg) was not booked as a carry, so do
            # not present it as an internal move.
            if out_row["id"] in quarantined_ids or in_row["id"] in quarantined_ids:
                continue
            _add(out_row["id"], {in_row["wallet_label"]})
            _add(in_row["id"], {out_row["wallet_label"]})
        return {tx_id: ", ".join(sorted(labels)) for tx_id, labels in label_sets.items()}

    auto_pairs, _ = detect_intra_transfers(rows)

    # A reviewed whole-row direct payout books its outbound as a taxable
    # disposal, and the engine prunes any auto self-transfer pair touching it
    # before apply_manual_pairs so the disposal cannot be relabelled as a MOVE.
    # Mirror that here or the export would label the payout's source as a
    # transfer to the owned change wallet. Partial and over-amount (rejected)
    # payouts keep their auto pair, matching the engine.
    payout_records = conn.execute(
        "SELECT out_transaction_id, out_amount FROM direct_swap_payouts "
        "WHERE profile_id = ? AND deleted_at IS NULL",
        (profile["id"],),
    ).fetchall()
    payout_claimed_ids = set()
    for record in payout_records:
        out_row = rows_by_id.get(record["out_transaction_id"])
        if out_row is None:
            continue
        reviewed = record["out_amount"]
        full_amount = int(out_row["amount"] or 0)
        if reviewed in (None, "") or int(reviewed) == full_amount:
            payout_claimed_ids.add(out_row["id"])
    auto_pairs = [
        pair
        for pair in auto_pairs
        if pair["out"]["id"] not in payout_claimed_ids
        and pair["in"]["id"] not in payout_claimed_ids
    ]

    manual_leg_ids = set()
    for record in manual_records:
        manual_leg_ids.add(record["out_transaction_id"])
        manual_leg_ids.add(record["in_transaction_id"])
    same_asset_pairs, _cross_asset = apply_manual_pairs(rows, auto_pairs, manual_records)

    for pair in same_asset_pairs:
        # The out leg's counterparty is the destination wallet, and vice versa.
        _add(pair["out"]["id"], {pair["in"]["wallet_label"]})
        _add(pair["in"]["id"], {pair["out"]["wallet_label"]})
    for out_row, in_row in _carrying_value_cross_asset_pairs():
        _add(out_row["id"], {in_row["wallet_label"]})
        _add(in_row["id"], {out_row["wallet_label"]})
    # A partial cross-asset pair (split swap: part swapped, part change back to
    # an owned wallet) suppresses the same-txid auto pair in apply_manual_pairs,
    # but the engine splits the source row so the change leg stays a
    # self-transfer. Recover the label for suppressed legs that are not
    # themselves part of any reviewed pair.
    for pair in auto_pairs:
        for leg, other in ((pair["out"], pair["in"]), (pair["in"], pair["out"])):
            if leg["id"] in label_sets or leg["id"] in manual_leg_ids:
                continue
            _add(leg["id"], {other["wallet_label"]})
    return {tx_id: ", ".join(sorted(labels)) for tx_id, labels in label_sets.items()}


def _tags_by_transaction(conn, tx_where, tx_params):
    rows = conn.execute(
        f"""
        SELECT tt.transaction_id AS tx_id, tg.code AS code
        FROM transaction_tags tt
        JOIN tags tg ON tg.id = tt.tag_id
        JOIN transactions t ON t.id = tt.transaction_id
        WHERE {tx_where} AND t.excluded = 0
        ORDER BY tg.code ASC
        """,
        tx_params,
    ).fetchall()
    grouped = {}
    for row in rows:
        grouped.setdefault(row["tx_id"], []).append(row["code"])
    return {tx_id: ", ".join(codes) for tx_id, codes in grouped.items()}


def _attachment_entries_by_transaction(conn, tx_where, tx_params):
    rows = conn.execute(
        f"""
        SELECT
            a.transaction_id AS tx_id,
            a.attachment_type AS kind,
            COALESCE(a.label, '') AS label,
            COALESCE(a.original_filename, '') AS filename,
            COALESCE(a.source_url, '') AS url
        FROM attachments a
        JOIN transactions t ON t.id = a.transaction_id
        WHERE {tx_where} AND t.excluded = 0
        ORDER BY a.created_at ASC
        """,
        tx_params,
    ).fetchall()
    grouped = {}
    for row in rows:
        grouped.setdefault(row["tx_id"], []).append(_attachment_entry(row))
    return grouped


def _report_query_rows(conn, profile, wallet=None):
    tx_filters = ["t.profile_id = ?"]
    tx_params = [profile["id"]]
    if wallet:
        tx_filters.append("t.wallet_id = ?")
        tx_params.append(wallet["id"])
    tx_where = " AND ".join(tx_filters)

    summary = conn.execute(
        f"""
        SELECT
            COUNT(*) AS total_transactions,
            SUM(CASE WHEN t.excluded = 0 THEN 1 ELSE 0 END) AS active_transactions,
            SUM(CASE WHEN t.excluded = 1 THEN 1 ELSE 0 END) AS excluded_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_transactions,
            SUM(CASE WHEN t.excluded = 0 AND t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_transactions,
            COUNT(DISTINCT CASE WHEN t.excluded = 0 THEN t.asset END) AS asset_count,
            MIN(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS first_transaction_at,
            MAX(CASE WHEN t.excluded = 0 THEN t.occurred_at END) AS last_transaction_at,
            SUM(CASE WHEN t.excluded = 0 AND (t.fiat_rate IS NOT NULL OR t.fiat_value IS NOT NULL) THEN 1 ELSE 0 END) AS priced_transactions,
            SUM(CASE WHEN t.excluded = 0 AND COALESCE(TRIM(t.note), '') != '' THEN 1 ELSE 0 END) AS noted_transactions
        FROM transactions t
        WHERE {tx_where}
        """,
        tx_params,
    ).fetchone()

    tagged_transactions = conn.execute(
        f"""
        SELECT COUNT(DISTINCT tt.transaction_id) AS count
        FROM transaction_tags tt
        JOIN transactions t ON t.id = tt.transaction_id
        WHERE {tx_where} AND t.excluded = 0
        """,
        tx_params,
    ).fetchone()["count"]

    journal_filters = ["je.profile_id = ?"]
    journal_params = [profile["id"]]
    if wallet:
        journal_filters.append("je.wallet_id = ?")
        journal_params.append(wallet["id"])
    journal_where = " AND ".join(journal_filters)
    journal_entries = conn.execute(
        f"SELECT COUNT(*) AS count FROM journal_entries je WHERE {journal_where}",
        journal_params,
    ).fetchone()["count"]

    quarantines = conn.execute(
        f"""
        SELECT COUNT(*) AS count
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        WHERE {tx_where}
        """,
        tx_params,
    ).fetchone()["count"]

    flow_by_asset = conn.execute(
        f"""
        SELECT
            t.asset,
            COUNT(*) AS tx_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_count,
            SUM(CASE WHEN t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN t.amount ELSE 0 END) AS inbound_amount,
            SUM(CASE WHEN t.direction = 'outbound' THEN t.amount ELSE 0 END) AS outbound_amount,
            SUM(t.fee) AS fee_amount
        FROM transactions t
        WHERE {tx_where} AND t.excluded = 0
        GROUP BY t.asset
        ORDER BY t.asset ASC
        """,
        tx_params,
    ).fetchall()

    flow_by_wallet = conn.execute(
        f"""
        SELECT
            w.label AS wallet,
            t.asset,
            COUNT(*) AS tx_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN 1 ELSE 0 END) AS inbound_count,
            SUM(CASE WHEN t.direction = 'outbound' THEN 1 ELSE 0 END) AS outbound_count,
            SUM(CASE WHEN t.direction = 'inbound' THEN t.amount ELSE 0 END) AS inbound_amount,
            SUM(CASE WHEN t.direction = 'outbound' THEN t.amount ELSE 0 END) AS outbound_amount,
            SUM(t.fee) AS fee_amount,
            MIN(t.occurred_at) AS first_at,
            MAX(t.occurred_at) AS last_at
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {tx_where} AND t.excluded = 0
        GROUP BY w.label, t.asset
        ORDER BY w.label ASC, t.asset ASC
        """,
        tx_params,
    ).fetchall()

    quarantine_rows = conn.execute(
        f"""
        SELECT jq.reason, COUNT(*) AS count
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        WHERE {tx_where}
        GROUP BY jq.reason
        ORDER BY count DESC, jq.reason ASC
        """,
        tx_params,
    ).fetchall()

    transaction_rows = conn.execute(
        f"""
        SELECT
            t.id AS row_id,
            t.occurred_at,
            w.label AS wallet,
            COALESCE(t.external_id, '') AS transaction_id,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_currency,
            t.fiat_rate,
            t.fiat_value,
            t.fiat_rate_exact,
            t.fiat_value_exact,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            COALESCE(t.counterparty, '') AS counterparty
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {tx_where} AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        tx_params,
    ).fetchall()
    tags_by_tx = _tags_by_transaction(conn, tx_where, tx_params)
    attachments_by_tx = _attachment_entries_by_transaction(conn, tx_where, tx_params)
    journals_current = _journals_current(conn, profile["id"])
    journal_values = (
        _transaction_journal_values(conn, profile, wallet=wallet) if journals_current else {}
    )
    transfer_legs = _self_transfer_legs_by_transaction(
        conn, profile, journals_current=journals_current
    )
    transactions = []
    for row in transaction_rows:
        entries = attachments_by_tx.get(row["row_id"], [])
        journal = journal_values.get(row["row_id"], {})
        references = [entry["url"] for entry in entries if entry.get("url")]
        transactions.append(
            {
                **dict(row),
                "cost_basis": journal.get("cost_basis"),
                "gain_loss": journal.get("gain_loss"),
                "transfer": transfer_legs.get(row["row_id"], ""),
                "tags": tags_by_tx.get(row["row_id"], ""),
                "references": "\n".join(references),
                "attachments": "\n".join(entry["display_name"] for entry in entries),
                "attachments_list": entries,
            }
        )

    pair_filters = ["p.profile_id = ?", "p.deleted_at IS NULL"]
    pair_params = [profile["id"]]
    if wallet:
        pair_filters.append("(tout.wallet_id = ? OR tin.wallet_id = ?)")
        pair_params.extend([wallet["id"], wallet["id"]])
    pair_where = " AND ".join(pair_filters)
    transfer_pairs = conn.execute(
        f"""
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.swap_fee_msat,
            COALESCE(p.swap_fee_kind, '') AS swap_fee_kind,
            COALESCE(p.notes, '') AS notes,
            p.created_at,
            tout.occurred_at AS out_occurred_at,
            COALESCE(tout.external_id, '') AS out_transaction_id,
            wout.label AS out_wallet,
            tout.asset AS out_asset,
            -- Split cross-asset pairs cross only `out_amount`; the swap fee is
            -- measured against that portion, so the Transfers & Swaps sheet must
            -- report it too (NULL out_amount on whole/same-asset pairs).
            COALESCE(p.out_amount, tout.amount) AS out_amount,
            tout.fee AS out_fee,
            tin.occurred_at AS in_occurred_at,
            COALESCE(tin.external_id, '') AS in_transaction_id,
            win.label AS in_wallet,
            tin.asset AS in_asset,
            tin.amount AS in_amount,
            tin.fee AS in_fee
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE {pair_where}
        ORDER BY
            MIN(tout.occurred_at, tin.occurred_at) ASC,
            p.created_at ASC,
            p.id ASC
        """,
        pair_params,
    ).fetchall()

    direct_payout_filters = ["p.profile_id = ?", "p.deleted_at IS NULL"]
    direct_payout_params = [profile["id"]]
    if wallet:
        direct_payout_filters.append("tout.wallet_id = ?")
        direct_payout_params.append(wallet["id"])
    direct_payout_where = " AND ".join(direct_payout_filters)
    direct_swap_payouts = conn.execute(
        f"""
        SELECT
            p.id,
            p.kind,
            p.policy,
            p.payout_asset,
            p.payout_amount,
            p.payout_occurred_at,
            p.payout_external_id,
            p.counterparty,
            p.swap_fee_msat,
            COALESCE(p.swap_fee_kind, '') AS swap_fee_kind,
            COALESCE(p.notes, '') AS notes,
            p.created_at,
            tout.occurred_at AS out_occurred_at,
            COALESCE(tout.external_id, '') AS out_transaction_id,
            wout.label AS out_wallet,
            tout.asset AS out_asset,
            COALESCE(p.out_amount, tout.amount) AS out_amount,
            tout.fee AS out_fee
        FROM direct_swap_payouts p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        WHERE {direct_payout_where}
        ORDER BY
            COALESCE(p.payout_occurred_at, tout.occurred_at) ASC,
            p.created_at ASC,
            p.id ASC
        """,
        direct_payout_params,
    ).fetchall()

    return {
        "summary": summary,
        "tagged_transactions": tagged_transactions,
        "journal_entries": journal_entries,
        "quarantines": quarantines,
        "flow_by_asset": flow_by_asset,
        "flow_by_wallet": flow_by_wallet,
        "quarantine_rows": quarantine_rows,
        "transactions": transactions,
        "transfer_pairs": transfer_pairs,
        "direct_swap_payouts": direct_swap_payouts,
    }


def _summary_rollups(balance_rows, capital_rows):
    return {
        "holdings": {
            "cost_basis": float(sum(float(row["cost_basis"]) for row in balance_rows)),
            "market_value": float(sum(float(row["market_value"]) for row in balance_rows)),
            "unrealized_pnl": float(sum(float(row["unrealized_pnl"]) for row in balance_rows)),
        },
        "realized": {
            "proceeds": float(sum(float(row["proceeds"]) for row in capital_rows)),
            "cost_basis": float(sum(float(row["cost_basis"]) for row in capital_rows)),
            "gain_loss": float(sum(float(row["gain_loss"]) for row in capital_rows)),
        },
    }


def _summary_flow_rows(rows):
    return [
        {
            "asset": row["asset"],
            "tx_count": int(row["tx_count"] or 0),
            "inbound_count": int(row["inbound_count"] or 0),
            "outbound_count": int(row["outbound_count"] or 0),
            "inbound_amount": float(msat_to_btc(row["inbound_amount"] or 0)),
            "inbound_amount_sat": _msat_to_sat(row["inbound_amount"] or 0),
            "inbound_amount_msat": int(row["inbound_amount"] or 0),
            "outbound_amount": float(msat_to_btc(row["outbound_amount"] or 0)),
            "outbound_amount_sat": _msat_to_sat(row["outbound_amount"] or 0),
            "outbound_amount_msat": int(row["outbound_amount"] or 0),
            "fee_amount": float(msat_to_btc(row["fee_amount"] or 0)),
            "fee_amount_sat": _msat_to_sat(row["fee_amount"] or 0),
            "fee_amount_msat": int(row["fee_amount"] or 0),
        }
        for row in rows
    ]


def _msat_to_sat(value):
    msat = int(value or 0)
    return msat / 1000.0


def _summary_wallet_flow_rows(rows):
    return [
        {
            "wallet": row["wallet"],
            "asset": row["asset"],
            "tx_count": int(row["tx_count"] or 0),
            "inbound_count": int(row["inbound_count"] or 0),
            "outbound_count": int(row["outbound_count"] or 0),
            "inbound_amount": float(msat_to_btc(row["inbound_amount"] or 0)),
            "inbound_amount_sat": _msat_to_sat(row["inbound_amount"] or 0),
            "inbound_amount_msat": int(row["inbound_amount"] or 0),
            "outbound_amount": float(msat_to_btc(row["outbound_amount"] or 0)),
            "outbound_amount_sat": _msat_to_sat(row["outbound_amount"] or 0),
            "outbound_amount_msat": int(row["outbound_amount"] or 0),
            "fee_amount": float(msat_to_btc(row["fee_amount"] or 0)),
            "fee_amount_sat": _msat_to_sat(row["fee_amount"] or 0),
            "fee_amount_msat": int(row["fee_amount"] or 0),
            "first_transaction_at": row["first_at"],
            "last_transaction_at": row["last_at"],
        }
        for row in rows
    ]


def _build_summary_context(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    hooks.require_processed_journals(conn, profile)

    scope_wallets = _scope_wallets(conn, workspace["id"], profile["id"], hooks, wallet=wallet)
    portfolio_rows = report_portfolio_summary(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        portfolio_rows = [row for row in portfolio_rows if row["wallet"] == wallet["label"]]
    balance_rows = _aggregate_balance_rows_from_portfolio(portfolio_rows)

    capital_rows = report_capital_gains(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        capital_rows = [row for row in capital_rows if row["wallet"] == wallet["label"]]

    query_rows = _report_query_rows(conn, profile, wallet=wallet)
    summary = query_rows["summary"]
    rollups = _summary_rollups(balance_rows, capital_rows)

    return {
        "workspace": workspace,
        "profile": profile,
        "wallet": wallet,
        "scope_wallets": scope_wallets,
        "portfolio_rows": portfolio_rows,
        "balance_rows": balance_rows,
        "capital_rows": capital_rows,
        "query_rows": query_rows,
        "summary": summary,
        "rollups": rollups,
    }


def _build_full_report_context(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    if history_limit is not None and int(history_limit) < 0:
        raise AppError("--history-limit must be zero or positive", code="validation")
    hooks.require_processed_journals(conn, profile)

    scope_wallets = _scope_wallets(conn, workspace["id"], profile["id"], hooks, wallet=wallet)
    portfolio_rows = report_portfolio_summary(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        portfolio_rows = [row for row in portfolio_rows if row["wallet"] == wallet["label"]]
    balance_rows = _aggregate_balance_rows_from_portfolio(portfolio_rows)

    capital_rows = report_capital_gains(conn, workspace["id"], profile["id"], hooks)
    if wallet:
        capital_rows = [row for row in capital_rows if row["wallet"] == wallet["label"]]
    history_rows = report_balance_history(
        conn,
        workspace["id"],
        profile["id"],
        hooks,
        interval="month",
        wallet_ref=wallet["id"] if wallet else None,
    )
    if history_limit is not None and int(history_limit) > 0:
        history_rows = history_rows[-int(history_limit) :]

    query_rows = _report_query_rows(conn, profile, wallet=wallet)
    summary = query_rows["summary"]
    rollups = _summary_rollups(balance_rows, capital_rows)
    title_scope = wallet["label"] if wallet else profile["label"]

    return {
        "workspace": workspace,
        "profile": profile,
        "wallet": wallet,
        "scope_wallets": scope_wallets,
        "portfolio_rows": portfolio_rows,
        "balance_rows": balance_rows,
        "capital_rows": capital_rows,
        "history_rows": history_rows,
        "query_rows": query_rows,
        "summary": summary,
        "rollups": rollups,
        "generated_at": hooks.now_iso(),
        "title": f"Kassiber Report - {title_scope}",
        "pdf_title": f"Kassiber PDF Report - {title_scope}",
    }


def report_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    context = _build_summary_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]

    return {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "wallet": wallet["label"] if wallet else None,
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "tax_long_term_days": int(profile["tax_long_term_days"] or 0),
        "gains_algorithm": profile["gains_algorithm"],
        "last_processed_at": profile["last_processed_at"],
        "processed_tx_count": int(profile["last_processed_tx_count"] or 0),
        "metrics": {
            "wallets_in_scope": len(scope_wallets),
            "assets_in_scope": int(summary["asset_count"] or 0),
            "active_transactions": int(summary["active_transactions"] or 0),
            "excluded_transactions": int(summary["excluded_transactions"] or 0),
            "inbound_transactions": int(summary["inbound_transactions"] or 0),
            "outbound_transactions": int(summary["outbound_transactions"] or 0),
            "journal_entries": int(query_rows["journal_entries"] or 0),
            "quarantines": int(query_rows["quarantines"] or 0),
            "priced_transactions": int(summary["priced_transactions"] or 0),
            "transactions_with_notes": int(summary["noted_transactions"] or 0),
            "transactions_with_tags": int(query_rows["tagged_transactions"] or 0),
            "first_transaction_at": summary["first_transaction_at"],
            "last_transaction_at": summary["last_transaction_at"],
        },
        "holdings": rollups["holdings"],
        "realized": rollups["realized"],
        "asset_flow": _summary_flow_rows(query_rows["flow_by_asset"]),
        "wallet_flow": _summary_wallet_flow_rows(query_rows["flow_by_wallet"]),
        "transfer_pairs": _generic_report_transfer_pair_rows(context),
    }


def build_summary_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    context = _build_summary_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]

    title_scope = wallet["label"] if wallet else profile["label"]
    title = f"Kassiber Summary Report - {title_scope}"
    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Workspace", workspace["label"]),
                ("Profile", profile["label"]),
                ("Wallet scope", wallet["label"] if wallet else "All wallets"),
                ("Fiat currency", profile["fiat_currency"]),
                ("Tax country", profile["tax_country"]),
                ("Tax long-term days", profile["tax_long_term_days"]),
                ("Gains algorithm", profile["gains_algorithm"]),
                ("Last processed at", profile["last_processed_at"] or ""),
                ("Processed tx count", _report_count(profile["last_processed_tx_count"])),
            ]
        )
    )

    lines.extend(["", "Activity", "--------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Wallets in scope", _report_count(len(scope_wallets))),
                ("Assets in scope", _report_count(summary["asset_count"])),
                ("Transactions (active)", _report_count(summary["active_transactions"])),
                ("Transactions (excluded)", _report_count(summary["excluded_transactions"])),
                ("Inbound transactions", _report_count(summary["inbound_transactions"])),
                ("Outbound transactions", _report_count(summary["outbound_transactions"])),
                ("Journal entries", _report_count(query_rows["journal_entries"])),
                ("Quarantines", _report_count(query_rows["quarantines"])),
                ("Priced transactions", _report_count(summary["priced_transactions"])),
                ("Transactions with notes", _report_count(summary["noted_transactions"])),
                ("Transactions with tags", _report_count(query_rows["tagged_transactions"])),
                ("First transaction", summary["first_transaction_at"] or ""),
                ("Last transaction", summary["last_transaction_at"] or ""),
            ]
        )
    )

    lines.extend(["", "Financial Summary", "-----------------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Holdings cost basis", _report_fiat(rollups["holdings"]["cost_basis"])),
                ("Holdings market value", _report_fiat(rollups["holdings"]["market_value"])),
                ("Unrealized PnL", _report_fiat(rollups["holdings"]["unrealized_pnl"])),
                ("Realized proceeds", _report_fiat(rollups["realized"]["proceeds"])),
                ("Realized cost basis", _report_fiat(rollups["realized"]["cost_basis"])),
                ("Realized gain/loss", _report_fiat(rollups["realized"]["gain_loss"])),
            ]
        )
    )

    lines.extend(["", "Asset Flow", "----------"])
    asset_flow_rows = [
        [
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_asset"]
    ]
    if asset_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                asset_flow_rows,
                [6, 6, 6, 6, 14, 14, 14],
                align_right={1, 2, 3, 4, 5, 6},
            )
        )
    else:
        lines.append("No active transactions in scope.")
    return lines


def _summary_pdf_period(hooks: ReportHooks, start=None, end=None):
    end_dt = hooks.parse_iso_datetime(end, "end") if end else hooks.parse_iso_datetime(hooks.now_iso(), "now")
    if start:
        start_dt = hooks.parse_iso_datetime(start, "start")
    else:
        start_dt = end_dt.replace(month=1, day=1, hour=0, minute=0, second=0, microsecond=0)
    if start_dt > end_dt:
        raise AppError("--start must not be after --end", code="validation")
    return start_dt, end_dt


def _summary_pdf_period_label(hooks: ReportHooks, start_dt, end_dt):
    return f"{hooks.iso_z(start_dt)[:10]} to {hooks.iso_z(end_dt)[:10]}"


def _month_key(dt):
    return f"{dt.year:04d}-{dt.month:02d}"


def _summary_pdf_wallet_scope_is_all(conn, workspace_id, profile_id, hooks: ReportHooks, wallets):
    all_ids = {str(row["id"]) for row in hooks.list_wallets(conn, workspace_id, profile_id)}
    selected_ids = {str(row["id"]) for row in wallets}
    return selected_ids == all_ids


def _summary_pdf_tx_counts(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt):
    wallet_filter, wallet_params = _wallet_scope_sql("t.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT t.wallet_id, COUNT(*) AS tx_count
        FROM transactions t
        WHERE t.profile_id = ?
          AND {wallet_filter}
          AND t.excluded = 0
          AND t.occurred_at >= ?
          AND t.occurred_at <= ?
        GROUP BY t.wallet_id
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)],
    ).fetchall()
    return {row["wallet_id"]: int(row["tx_count"] or 0) for row in rows}


def _summary_pdf_portfolio_rows(conn, workspace_id, profile_id, hooks: ReportHooks, wallets, *, as_of=None):
    rows = report_portfolio_summary(conn, workspace_id, profile_id, hooks, as_of=as_of, include_wallet_id=True)
    if _summary_pdf_wallet_scope_is_all(conn, workspace_id, profile_id, hooks, wallets):
        return rows
    selected_ids = {str(row["id"]) for row in wallets}
    return [row for row in rows if str(row.get("wallet_id")) in selected_ids]


def _summary_pdf_wallet_holdings_from_portfolio(wallets, portfolio_rows, tx_counts):
    buckets = {
        str(row["id"]): {
            "wallet_id": row["id"],
            "wallet": row["label"],
            "scope": f"{row['kind']} / {row['chain'] or 'chain'}",
            "assets": set(),
            "asset_quantities": defaultdict(lambda: Decimal("0")),
            "quantity": Decimal("0"),
            "cost_basis": Decimal("0"),
            "market_value": Decimal("0"),
            "tx_count": tx_counts.get(row["id"], 0),
        }
        for row in wallets
    }
    for row in portfolio_rows:
        bucket = buckets.get(str(row.get("wallet_id")))
        if bucket is None:
            continue
        bucket["assets"].add(row["asset"])
        quantity = dec(row["quantity"])
        bucket["asset_quantities"][row["asset"]] += quantity
        bucket["quantity"] += quantity
        bucket["cost_basis"] += dec(row["cost_basis"])
        bucket["market_value"] += dec(row["market_value"])
    results = []
    for row in wallets:
        bucket = buckets[str(row["id"])]
        results.append(
            {
                "wallet_id": bucket["wallet_id"],
                "wallet": bucket["wallet"],
                "scope": bucket["scope"],
                "assets": sorted(bucket["assets"]),
                "asset_quantities": [
                    {"asset": asset, "quantity": float(quantity)}
                    for asset, quantity in sorted(bucket["asset_quantities"].items())
                    if quantity
                ],
                "quantity": float(bucket["quantity"]),
                "cost_basis": float(bucket["cost_basis"]),
                "market_value": float(bucket["market_value"]),
                "tx_count": bucket["tx_count"],
            }
        )
    return results


def _summary_pdf_balance_history_from_report(conn, workspace_id, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt):
    start_text = hooks.iso_z(start_dt)
    end_text = hooks.iso_z(end_dt)
    if _summary_pdf_wallet_scope_is_all(conn, workspace_id, profile_id, hooks, wallets):
        rows = report_balance_history(
            conn,
            workspace_id,
            profile_id,
            hooks,
            interval="month",
            start=start_text,
            end=end_text,
        )
    else:
        rows = []
        for wallet in wallets:
            rows.extend(
                report_balance_history(
                    conn,
                    workspace_id,
                    profile_id,
                    hooks,
                    interval="month",
                    start=start_text,
                    end=end_text,
                    wallet_ref=wallet["id"],
                )
            )
    buckets = defaultdict(
        lambda: {
            "quantity": Decimal("0"),
            "cumulative_cost_basis": Decimal("0"),
            "market_value": Decimal("0"),
        }
    )
    period_ends = {}
    for row in rows:
        key = row["period_start"]
        bucket = buckets[key]
        bucket["quantity"] += dec(row["quantity"])
        bucket["cumulative_cost_basis"] += dec(row["cumulative_cost_basis"])
        bucket["market_value"] += dec(row["market_value"])
        period_ends[key] = row["period_end"]
    results = []
    for period_start in sorted(buckets):
        period_end = period_ends[period_start]
        period_end_dt = hooks.parse_iso_datetime(period_end, "period_end")
        bucket = buckets[period_start]
        results.append(
            {
                "period_start": period_start,
                "period_end": period_end,
                "period": period_start[:7],
                "period_partial": end_dt < period_end_dt,
                "quantity": float(bucket["quantity"]),
                "cumulative_cost_basis": float(bucket["cumulative_cost_basis"]),
                "market_value": float(bucket["market_value"]),
            }
        )
    return results


def _summary_pdf_total_market_value_from_holdings(rows):
    return float(sum(dec(row["market_value"]) for row in rows))


def _summary_pdf_total_quantity_from_holdings(rows):
    return float(sum(dec(row["quantity"]) for row in rows))


def _summary_pdf_total_asset_quantities_from_holdings(rows):
    totals = defaultdict(lambda: Decimal("0"))
    for row in rows:
        for item in row.get("asset_quantities") or []:
            asset = str(item.get("asset") or "").strip().upper()
            if asset:
                totals[asset] += dec(item.get("quantity"))
    return [
        {"asset": asset, "quantity": float(quantity)}
        for asset, quantity in sorted(totals.items())
        if quantity
    ]


def _summary_pdf_int(value):
    try:
        return int(value or 0)
    except (TypeError, ValueError):
        return 0


def _summary_pdf_data_integrity(conn, profile, wallets, hooks: ReportHooks, start_dt, end_dt):
    internal_transfers = _summary_pdf_internal_transfers(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    wallet_filter, wallet_params = _wallet_scope_sql("t.wallet_id", wallets)
    tx_params = [profile["id"], *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)]
    tx_summary = conn.execute(
        f"""
        SELECT COUNT(*) AS total_transactions,
               SUM(CASE WHEN t.fiat_rate IS NOT NULL OR t.fiat_value IS NOT NULL THEN 1 ELSE 0 END)
                    AS priced_transactions
        FROM transactions t
        WHERE t.profile_id = ?
          AND {wallet_filter}
          AND t.excluded = 0
          AND t.occurred_at >= ?
          AND t.occurred_at <= ?
        """,
        tx_params,
    ).fetchone()
    quarantine_rows = conn.execute(
        f"""
        SELECT jq.reason, COUNT(*) AS count
        FROM journal_quarantines jq
        JOIN transactions t ON t.id = jq.transaction_id
        WHERE t.profile_id = ?
          AND {wallet_filter}
          AND t.excluded = 0
          AND t.occurred_at >= ?
          AND t.occurred_at <= ?
        GROUP BY jq.reason
        ORDER BY count DESC, jq.reason ASC
        """,
        tx_params,
    ).fetchall()
    current_tx_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    input_version = _summary_pdf_int(profile["journal_input_version"])
    processed_version = _summary_pdf_int(profile["last_processed_input_version"])
    last_processed_tx_count = _summary_pdf_int(profile["last_processed_tx_count"])
    journals_current = bool(
        profile["last_processed_at"]
        and _summary_pdf_int(current_tx_count) == last_processed_tx_count
        and input_version == processed_version
    )
    total_transactions = _summary_pdf_int(tx_summary["total_transactions"])
    priced_transactions = _summary_pdf_int(tx_summary["priced_transactions"])
    priced_percentage = (
        (priced_transactions / total_transactions) * 100.0
        if total_transactions
        else 100.0
    )
    quarantine_reasons = [
        {"reason": row["reason"], "count": _summary_pdf_int(row["count"])}
        for row in quarantine_rows
    ]
    return {
        "priced_transactions": priced_transactions,
        "total_transactions": total_transactions,
        "priced_percentage": priced_percentage,
        "quarantine_count": sum(row["count"] for row in quarantine_reasons),
        "quarantine_reasons": quarantine_reasons,
        "internal_transfers": internal_transfers,
        "journals": {
            "status": "current" if journals_current else ("stale" if profile["last_processed_at"] else "not_processed"),
            "current": journals_current,
            "last_processed_at": profile["last_processed_at"],
            "journal_input_version": input_version,
            "last_processed_input_version": processed_version,
            "last_processed_tx_count": last_processed_tx_count,
            "current_tx_count": _summary_pdf_int(current_tx_count),
        },
    }


def _summary_pdf_flow_periods(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt):
    wallet_filter, wallet_params = _wallet_scope_sql("t.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT t.occurred_at, t.direction, t.amount, t.fee, t.fiat_rate, t.fiat_value
        FROM transactions t
        WHERE t.profile_id = ?
          AND {wallet_filter}
          AND t.excluded = 0
          AND t.occurred_at >= ?
          AND t.occurred_at <= ?
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)],
    ).fetchall()
    buckets = defaultdict(lambda: {"inflow_volume": Decimal("0"), "outflow_volume": Decimal("0"), "fees_btc": Decimal("0"), "fees_fiat": Decimal("0")})
    totals = {"inflow": Decimal("0"), "outflow": Decimal("0"), "fees_btc": Decimal("0"), "fees_fiat": Decimal("0")}
    for row in rows:
        key = _month_key(hooks.parse_iso_datetime(row["occurred_at"], "occurred_at"))
        amount = msat_to_btc(row["amount"] or 0)
        fee = msat_to_btc(row["fee"] or 0)
        if row["fiat_value"] is not None:
            volume = dec(row["fiat_value"])
        elif row["fiat_rate"] is not None:
            volume = amount * dec(row["fiat_rate"])
        else:
            volume = Decimal("0")
        fee_fiat = fee * dec(row["fiat_rate"]) if row["fiat_rate"] is not None else Decimal("0")
        if row["direction"] == "inbound":
            buckets[key]["inflow_volume"] += volume
            totals["inflow"] += volume
        else:
            buckets[key]["outflow_volume"] += volume
            totals["outflow"] += volume
        buckets[key]["fees_btc"] += fee
        buckets[key]["fees_fiat"] += fee_fiat
        totals["fees_btc"] += fee
        totals["fees_fiat"] += fee_fiat
    periods = []
    cursor = _floor_to_interval(start_dt, "month")
    end_cap = _floor_to_interval(end_dt, "month")
    while cursor <= end_cap:
        key = _month_key(cursor)
        bucket = buckets[key]
        periods.append(
            {
                "period": key,
                "inflow_volume": float(bucket["inflow_volume"]),
                "outflow_volume": float(bucket["outflow_volume"]),
                "fees_btc": float(bucket["fees_btc"]),
                "fees_fiat": float(bucket["fees_fiat"]),
            }
        )
        cursor = _next_interval(cursor, "month")
    return periods, totals


def _summary_pdf_realized_periods(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt):
    wallet_filter, wallet_params = _wallet_scope_sql("je.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT je.occurred_at,
               COALESCE(je.proceeds, 0) AS proceeds,
               COALESCE(je.cost_basis, 0) AS cost_basis,
               COALESCE(je.gain_loss, 0) AS gain_loss
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND {wallet_filter}
          AND je.entry_type = 'disposal'
          AND COALESCE(t.taxability_override, 1) != 0
          AND COALESCE(je.at_category, '') != 'neu_swap'
          AND je.occurred_at >= ?
          AND je.occurred_at <= ?
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)],
    ).fetchall()
    buckets = defaultdict(lambda: {"proceeds": Decimal("0"), "cost_basis": Decimal("0"), "realized_pnl": Decimal("0"), "count": 0})
    total = {"proceeds": Decimal("0"), "cost_basis": Decimal("0"), "realized_pnl": Decimal("0"), "count": 0}
    for row in rows:
        key = _month_key(hooks.parse_iso_datetime(row["occurred_at"], "occurred_at"))
        bucket = buckets[key]
        bucket["proceeds"] += dec(row["proceeds"])
        bucket["cost_basis"] += dec(row["cost_basis"])
        bucket["realized_pnl"] += dec(row["gain_loss"])
        bucket["count"] += 1
        total["proceeds"] += dec(row["proceeds"])
        total["cost_basis"] += dec(row["cost_basis"])
        total["realized_pnl"] += dec(row["gain_loss"])
        total["count"] += 1
    periods = []
    cursor = _floor_to_interval(start_dt, "month")
    end_cap = _floor_to_interval(end_dt, "month")
    while cursor <= end_cap:
        key = _month_key(cursor)
        bucket = buckets[key]
        periods.append(
            {
                "period": key,
                "proceeds": float(bucket["proceeds"]),
                "cost_basis": float(bucket["cost_basis"]),
                "realized_pnl": float(bucket["realized_pnl"]),
                "count": bucket["count"],
            }
        )
        cursor = _next_interval(cursor, "month")
    return periods, total


def _summary_pdf_rate_at(conn, pair, target_iso):
    row = conn.execute(
        """
        SELECT rate
        FROM rates_cache
        WHERE pair = ? AND timestamp <= ?
        ORDER BY timestamp DESC, fetched_at DESC
        LIMIT 1
        """,
        (pair, target_iso),
    ).fetchone()
    if row is None:
        return None
    return float(row["rate"])


def _summary_pdf_benchmark(conn, hooks: ReportHooks, start_dt, end_dt, fiat_currency):
    if not fiat_currency:
        return None
    pair = f"BTC-{fiat_currency.upper()}"
    start_rate = _summary_pdf_rate_at(conn, pair, hooks.iso_z(start_dt))
    end_rate = _summary_pdf_rate_at(conn, pair, hooks.iso_z(end_dt))
    if start_rate is None or end_rate is None or start_rate <= 0:
        return {"pair": pair, "start_rate": start_rate, "end_rate": end_rate, "change_pct": None}
    change_pct = (end_rate - start_rate) / start_rate * 100.0
    return {"pair": pair, "start_rate": start_rate, "end_rate": end_rate, "change_pct": change_pct}


def _summary_pdf_top_movements(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt, *, limit=5):
    wallet_filter, wallet_params = _wallet_scope_sql("t.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT t.occurred_at, t.direction, t.asset, t.amount, t.fiat_value, t.fiat_rate,
               t.counterparty, t.description, w.label AS wallet
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE t.profile_id = ?
          AND {wallet_filter}
          AND t.excluded = 0
          AND t.occurred_at >= ?
          AND t.occurred_at <= ?
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)],
    ).fetchall()
    ranked = []
    for row in rows:
        amount = msat_to_btc(row["amount"] or 0)
        if row["fiat_value"] is not None:
            fiat = dec(row["fiat_value"])
        elif row["fiat_rate"] is not None:
            fiat = amount * dec(row["fiat_rate"])
        else:
            fiat = Decimal("0")
        ranked.append({
            "occurred_at": row["occurred_at"],
            "wallet": row["wallet"],
            "direction": row["direction"],
            "asset": row["asset"],
            "quantity": float(amount),
            "fiat_value": float(fiat),
            "counterparty": row["counterparty"] or row["description"] or "",
        })
    ranked.sort(key=lambda r: abs(r["fiat_value"]), reverse=True)
    return ranked[:limit]


def _summary_pdf_top_disposals(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt, *, limit=5):
    wallet_filter, wallet_params = _wallet_scope_sql("je.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT je.occurred_at, je.asset, je.quantity,
               COALESCE(je.proceeds, 0) AS proceeds,
               COALESCE(je.cost_basis, 0) AS cost_basis,
               COALESCE(je.gain_loss, 0) AS gain_loss,
               w.label AS wallet
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND {wallet_filter}
          AND je.entry_type = 'disposal'
          AND COALESCE(t.taxability_override, 1) != 0
          AND COALESCE(je.at_category, '') != 'neu_swap'
          AND je.occurred_at >= ?
          AND je.occurred_at <= ?
        ORDER BY ABS(je.gain_loss) DESC, je.occurred_at DESC
        LIMIT ?
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt), limit],
    ).fetchall()
    return [
        {
            "occurred_at": row["occurred_at"],
            "wallet": row["wallet"],
            "asset": row["asset"],
            "quantity": float(msat_to_btc(row["quantity"] or 0)),
            "proceeds": float(dec(row["proceeds"])),
            "cost_basis": float(dec(row["cost_basis"])),
            "gain_loss": float(dec(row["gain_loss"])),
        }
        for row in rows
    ]


def _summary_pdf_internal_transfers(conn, profile_id, wallets, hooks: ReportHooks, start_dt, end_dt):
    wallet_filter, wallet_params = _wallet_scope_sql("tout.wallet_id", wallets)
    # A multi-pair component (whirlpool 1->N review) repeats the SAME out leg
    # on every pair row, so aggregate over distinct out transactions — one
    # spend is one internal transfer, whatever the number of receipt legs.
    rows = conn.execute(
        f"""
        SELECT COUNT(*) AS count,
               COALESCE(SUM(COALESCE(fiat_value, 0)), 0) AS fiat_volume,
               COALESCE(SUM(amount), 0) AS amount_msat
        FROM (
            SELECT DISTINCT tout.id, tout.fiat_value, tout.amount
            FROM transaction_pairs p
            JOIN transactions tout ON tout.id = p.out_transaction_id
            JOIN transactions tin ON tin.id = p.in_transaction_id
            WHERE p.profile_id = ?
              AND p.deleted_at IS NULL
              AND p.policy = 'carrying-value'
              AND tout.asset = tin.asset
              AND {wallet_filter}
              AND tout.occurred_at >= ?
              AND tout.occurred_at <= ?
        )
        """,
        [profile_id, *wallet_params, hooks.iso_z(start_dt), hooks.iso_z(end_dt)],
    ).fetchone()
    return {
        "count": _summary_pdf_int(rows["count"]),
        "fiat_volume": float(dec(rows["fiat_volume"])),
        "btc_volume": float(msat_to_btc(rows["amount_msat"] or 0)),
    }


def _summary_pdf_holding_age(conn, profile_id, wallets, hooks: ReportHooks, end_dt):
    wallet_filter, wallet_params = _wallet_scope_sql("je.wallet_id", wallets)
    rows = conn.execute(
        f"""
        SELECT je.occurred_at, je.quantity
        FROM journal_entries je
        WHERE je.profile_id = ?
          AND {wallet_filter}
          AND je.entry_type = 'acquisition'
          AND je.occurred_at <= ?
        ORDER BY je.occurred_at ASC
        """,
        [profile_id, *wallet_params, hooks.iso_z(end_dt)],
    ).fetchall()
    if not rows:
        return {"weighted_days": None, "oldest_acquisition": None, "acquisition_count": 0}
    weighted_seconds = Decimal("0")
    total_qty = Decimal("0")
    oldest = rows[0]["occurred_at"]
    for row in rows:
        qty = msat_to_btc(row["quantity"] or 0)
        if qty <= 0:
            continue
        acquired_at = hooks.parse_iso_datetime(row["occurred_at"], "occurred_at")
        seconds = Decimal(str((end_dt - acquired_at).total_seconds()))
        weighted_seconds += qty * seconds
        total_qty += qty
    if total_qty <= 0:
        return {"weighted_days": None, "oldest_acquisition": oldest, "acquisition_count": len(rows)}
    weighted_days = float(weighted_seconds / total_qty / Decimal("86400"))
    return {
        "weighted_days": weighted_days,
        "oldest_acquisition": oldest,
        "acquisition_count": len(rows),
    }


def build_summary_pdf_report_data(
    conn,
    workspace_ref,
    profile_ref,
    hooks: ReportHooks,
    *,
    start=None,
    end=None,
    wallet_refs=None,
    include_snapshot=False,
):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    start_dt, end_dt = _summary_pdf_period(hooks, start=start, end=end)
    wallets = _resolve_wallet_scope_refs(conn, workspace["id"], profile["id"], hooks, wallet_refs=wallet_refs)
    generated_at = hooks.now_iso()
    portfolio_rows = _summary_pdf_portfolio_rows(
        conn,
        workspace["id"],
        profile["id"],
        hooks,
        wallets,
        as_of=end_dt,
    )
    tx_counts = _summary_pdf_tx_counts(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    wallet_holdings = _summary_pdf_wallet_holdings_from_portfolio(wallets, portfolio_rows, tx_counts)
    holdings_totals = {
        "total_quantity": _summary_pdf_total_quantity_from_holdings(wallet_holdings),
        "asset_quantities": _summary_pdf_total_asset_quantities_from_holdings(wallet_holdings),
        "total_market_value": _summary_pdf_total_market_value_from_holdings(wallet_holdings),
    }
    history_rows = _summary_pdf_balance_history_from_report(conn, workspace["id"], profile["id"], wallets, hooks, start_dt, end_dt)
    flow_periods, flow_totals = _summary_pdf_flow_periods(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    realized_periods, realized_total = _summary_pdf_realized_periods(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    period_start_value = history_rows[0]["market_value"] if history_rows else 0.0
    period_end_value = history_rows[-1]["market_value"] if history_rows else 0.0
    end_cost_basis = history_rows[-1]["cumulative_cost_basis"] if history_rows else 0.0
    unrealized_pnl = period_end_value - end_cost_basis
    btc_stack_start = history_rows[0]["quantity"] if history_rows else 0.0
    btc_stack_end = history_rows[-1]["quantity"] if history_rows else 0.0
    data_integrity = _summary_pdf_data_integrity(conn, profile, wallets, hooks, start_dt, end_dt)
    benchmark = _summary_pdf_benchmark(conn, hooks, start_dt, end_dt, profile["fiat_currency"])
    top_movements = _summary_pdf_top_movements(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    top_disposals = _summary_pdf_top_disposals(conn, profile["id"], wallets, hooks, start_dt, end_dt)
    holding_age = _summary_pdf_holding_age(conn, profile["id"], wallets, hooks, end_dt)
    title = f"Kassiber Summary Report - {profile['label']}"
    snapshot = None
    if include_snapshot:
        snapshot_rows = _summary_pdf_portfolio_rows(conn, workspace["id"], profile["id"], hooks, wallets)
        snapshot_holdings = _summary_pdf_wallet_holdings_from_portfolio(wallets, snapshot_rows, tx_counts)
        snapshot_totals = {
            "total_quantity": _summary_pdf_total_quantity_from_holdings(snapshot_holdings),
            "asset_quantities": _summary_pdf_total_asset_quantities_from_holdings(snapshot_holdings),
            "total_market_value": _summary_pdf_total_market_value_from_holdings(snapshot_holdings),
        }
        snapshot = {
            "as_of": generated_at,
            "wallets": snapshot_holdings,
            "total_quantity": snapshot_totals["total_quantity"],
            "asset_quantities": snapshot_totals["asset_quantities"],
            "total_market_value": snapshot_totals["total_market_value"],
        }
    return {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "fiat_currency": profile["fiat_currency"],
        "generated_at": generated_at,
        "title": title,
        "timeframe": {
            "start": hooks.iso_z(start_dt),
            "end": hooks.iso_z(end_dt),
            "label": _summary_pdf_period_label(hooks, start_dt, end_dt),
        },
        "wallets": [{"id": row["id"], "label": row["label"]} for row in wallets],
        "data_integrity": data_integrity,
        "metrics": {
            "period_start_value": period_start_value,
            "period_end_value": period_end_value,
            "net_flow": float(flow_totals["inflow"] - flow_totals["outflow"]),
            "realized_pnl": float(realized_total["realized_pnl"]),
            "unrealized_pnl": unrealized_pnl,
            "end_cost_basis": end_cost_basis,
            "btc_stack_start": btc_stack_start,
            "btc_stack_end": btc_stack_end,
            "fees_btc": float(flow_totals["fees_btc"]),
            "fees_fiat": float(flow_totals["fees_fiat"]),
        },
        "benchmark": benchmark,
        "top_movements": top_movements,
        "top_disposals": top_disposals,
        "holding_age": holding_age,
        "balance_history": history_rows,
        "wallet_holdings": wallet_holdings,
        "holdings_totals": holdings_totals,
        "realized_pnl_periods": realized_periods,
        "flow_periods": flow_periods,
        "wallet_appendix": [
            {
                "wallet": row["wallet"],
                "scope": row["scope"],
                "tx_count": row["tx_count"],
                "assets": row["assets"],
                "asset_quantities": row["asset_quantities"],
                "end_quantity": row["quantity"],
                "end_market_value": row["market_value"],
            }
            for row in wallet_holdings
        ],
        "snapshot": snapshot,
    }


def _tax_summary_total_row(
    row_type,
    *,
    year=None,
    asset="",
    quantity: Decimal | None = None,
    proceeds=Decimal("0"),
    cost_basis=Decimal("0"),
    gain_loss=Decimal("0"),
):
    return {
        "row_type": row_type,
        "year": year,
        "asset": asset,
        "transaction_type": "",
        "capital_gains_type": "",
        "quantity": float(quantity) if quantity is not None else None,
        "quantity_msat": btc_to_msat(quantity) if quantity is not None else None,
        "proceeds": float(proceeds),
        "cost_basis": float(cost_basis),
        "gain_loss": float(gain_loss),
        "count": None,
        "total_swap_fee_msat": None,
        "total_swap_fee": None,
    }


_TAX_SUMMARY_ROW_KEYS = (
    "row_type",
    "year",
    "asset",
    "transaction_type",
    "capital_gains_type",
    "quantity",
    "quantity_msat",
    "proceeds",
    "cost_basis",
    "gain_loss",
    "count",
    "total_swap_fee_msat",
    "total_swap_fee",
)


def _tax_summary_detail_row(row):
    return _normalize_tax_summary_row({"row_type": "detail", **row})


def _normalize_tax_summary_row(row):
    normalized = dict(row)
    for key in _TAX_SUMMARY_ROW_KEYS:
        normalized.setdefault(key, None)
    return {key: normalized.get(key) for key in _TAX_SUMMARY_ROW_KEYS}


def _tax_summary_capital_gains_type(value):
    """Normalize capital_gains_type for tax-summary adjustment keys.

    NULL/blank summary rows and journal entries must share the same default
    (``short``) so non-reportable / neu_swap exclusions can match.
    """
    return str(value or "short").strip().lower() or "short"


def _non_reportable_tax_summary_adjustments(conn, profile_id):
    rows = conn.execute(
        """
        SELECT substr(je.occurred_at, 1, 4) AS year,
               je.asset,
               je.entry_type,
               t.kind AS transaction_kind,
               COALESCE(je.capital_gains_type, '') AS capital_gains_type,
               SUM(ABS(je.quantity)) AS quantity_msat,
               SUM(COALESCE(je.proceeds, 0)) AS proceeds,
               SUM(COALESCE(je.cost_basis, 0)) AS cost_basis,
               SUM(COALESCE(je.gain_loss, 0)) AS gain_loss
        FROM journal_entries je
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
          AND (
            (je.entry_type = 'disposal' AND je.at_category = 'neu_swap')
            OR (
              COALESCE(t.taxability_override, 1) = 0
              AND je.entry_type IN ('disposal', 'income')
            )
          )
        GROUP BY substr(je.occurred_at, 1, 4), je.asset, je.entry_type, t.kind, je.capital_gains_type
        """,
        (profile_id,),
    ).fetchall()
    adjustments = {}
    for row in rows:
        year = str(row["year"] or "")
        asset = str(row["asset"] or "")
        if not year.isdigit() or not asset:
            continue
        quantity_msat = int(row["quantity_msat"] or 0)
        if row["entry_type"] == "income":
            transaction_type = _TAX_SUMMARY_INCOME_TRANSACTION_TYPE_BY_KIND.get(
                str(row["transaction_kind"] or "").strip().lower(),
                "income",
            )
        else:
            transaction_type = "sell"
        capital_gains_type = _tax_summary_capital_gains_type(row["capital_gains_type"])
        key = (int(year), asset, transaction_type, capital_gains_type)
        adjustment = adjustments.setdefault(
            key,
            {
                "quantity": Decimal("0"),
                "quantity_msat": 0,
                "proceeds": Decimal("0"),
                "cost_basis": Decimal("0"),
                "gain_loss": Decimal("0"),
            },
        )
        adjustment["quantity"] += msat_to_btc(quantity_msat)
        adjustment["quantity_msat"] += quantity_msat
        adjustment["proceeds"] += dec(row["proceeds"])
        adjustment["cost_basis"] += dec(row["cost_basis"])
        adjustment["gain_loss"] += dec(row["gain_loss"])
    return adjustments


def _tax_summary_row_is_zero(row):
    quantity_msat = row.get("quantity_msat")
    if quantity_msat is None:
        quantity_msat = btc_to_msat(row.get("quantity"))
    if int(quantity_msat or 0) != 0:
        return False
    return all(
        abs(dec(row.get(key))) < Decimal("0.00000001")
        for key in ("proceeds", "cost_basis", "gain_loss")
    )


def _exclude_non_reportable_tax_summary_rows(conn, profile_id, rows):
    adjustments = _non_reportable_tax_summary_adjustments(conn, profile_id)
    if not adjustments:
        return [dict(row) for row in rows]

    adjusted_rows = []
    for row in rows:
        adjusted = dict(row)
        key = (
            int(adjusted["year"]),
            str(adjusted["asset"] or ""),
            str(adjusted["transaction_type"] or "").lower(),
            _tax_summary_capital_gains_type(adjusted["capital_gains_type"]),
        )
        adjustment = adjustments.pop(key, None)
        if adjustment is not None:
            quantity = dec(adjusted["quantity"]) - adjustment["quantity"]
            quantity_msat = (
                int(adjusted["quantity_msat"] or 0)
                - adjustment["quantity_msat"]
            )
            adjusted.update(
                {
                    "quantity": float(quantity),
                    "quantity_msat": quantity_msat,
                    "proceeds": float(
                        dec(adjusted["proceeds"]) - adjustment["proceeds"]
                    ),
                    "cost_basis": float(
                        dec(adjusted["cost_basis"]) - adjustment["cost_basis"]
                    ),
                    "gain_loss": float(
                        dec(adjusted["gain_loss"]) - adjustment["gain_loss"]
                    ),
                }
            )
        if not _tax_summary_row_is_zero(adjusted):
            adjusted_rows.append(adjusted)
    return adjusted_rows


def report_tax_summary(conn, workspace_ref, profile_ref, hooks: ReportHooks):
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    hooks.require_processed_journals(conn, profile)
    try:
        stored_rows = conn.execute(
            """
            SELECT year, asset, transaction_type, capital_gains_type, quantity,
                   proceeds, cost_basis, gain_loss
            FROM journal_tax_summary
            WHERE profile_id = ?
            """,
            (profile["id"],),
        ).fetchall()
        tax_summary_rows = [
            {
                "year": int(row["year"]),
                "asset": row["asset"],
                "transaction_type": row["transaction_type"],
                "capital_gains_type": row["capital_gains_type"],
                "quantity": float(msat_to_btc(row["quantity"])),
                "quantity_msat": int(row["quantity"]),
                "proceeds": row["proceeds"],
                "cost_basis": row["cost_basis"],
                "gain_loss": row["gain_loss"],
            }
            for row in stored_rows
        ]
    except sqlite3.OperationalError:
        tax_summary_rows = None
    if tax_summary_rows is None or (not tax_summary_rows and _profile_has_journal_entries(conn, profile["id"])):
        tax_summary_rows = hooks.build_ledger_state(conn, profile)["tax_summary"]
    taxable_summary_rows = _exclude_non_reportable_tax_summary_rows(
        conn,
        profile["id"],
        tax_summary_rows,
    )
    detail_rows = sorted(
        taxable_summary_rows,
        key=lambda row: (
            int(row["year"]),
            row["asset"],
            row["transaction_type"],
            row["capital_gains_type"],
        ),
    )
    swap_fee_rows = _swap_fee_summary_rows(conn, profile["id"])
    if not detail_rows and not swap_fee_rows:
        return []

    grouped_by_year = defaultdict(
        lambda: {
            "assets": set(),
            "quantity": Decimal("0"),
            "proceeds": Decimal("0"),
            "cost_basis": Decimal("0"),
            "gain_loss": Decimal("0"),
        }
    )
    grand = {
        "assets": set(),
        "quantity": Decimal("0"),
        "proceeds": Decimal("0"),
        "cost_basis": Decimal("0"),
        "gain_loss": Decimal("0"),
    }
    grouped_rows = defaultdict(list)
    for row in detail_rows:
        quantity = dec(row["quantity"])
        proceeds = dec(row["proceeds"])
        cost_basis = dec(row["cost_basis"])
        gain_loss = dec(row["gain_loss"])
        year = int(row["year"])
        grouped_rows[year].append(_tax_summary_detail_row(row))
        grouped_by_year[year]["assets"].add(row["asset"])
        grouped_by_year[year]["quantity"] += quantity
        grouped_by_year[year]["proceeds"] += proceeds
        grouped_by_year[year]["cost_basis"] += cost_basis
        grouped_by_year[year]["gain_loss"] += gain_loss
        grand["assets"].add(row["asset"])
        grand["quantity"] += quantity
        grand["proceeds"] += proceeds
        grand["cost_basis"] += cost_basis
        grand["gain_loss"] += gain_loss

    rows = []
    for year in sorted(grouped_rows):
        year_asset = next(iter(grouped_by_year[year]["assets"])) if len(grouped_by_year[year]["assets"]) == 1 else ""
        year_quantity = grouped_by_year[year]["quantity"] if year_asset else None
        rows.extend(grouped_rows[year])
        rows.append(
            _tax_summary_total_row(
                "year_total",
                year=year,
                asset=year_asset,
                quantity=year_quantity,
                proceeds=grouped_by_year[year]["proceeds"],
                cost_basis=grouped_by_year[year]["cost_basis"],
                gain_loss=grouped_by_year[year]["gain_loss"],
            )
        )
    rows.append(
        _tax_summary_total_row(
            "grand_total",
            asset=next(iter(grand["assets"])) if len(grand["assets"]) == 1 else "",
            quantity=grand["quantity"] if len(grand["assets"]) == 1 else None,
            proceeds=grand["proceeds"],
            cost_basis=grand["cost_basis"],
            gain_loss=grand["gain_loss"],
        )
    )
    rows.extend(swap_fee_rows)
    return [_normalize_tax_summary_row(row) for row in rows]


def _swap_fee_summary_rows(conn, profile_id):
    """Aggregate ``transaction_pairs.swap_fee_msat`` per tax year and per
    grand total, returning rows shaped to slot into the tax-summary list.

    Surfaces the "what actually left your custody" line that's invisible
    to the per-asset capital-gains breakdown above. For carrying-value
    swaps the principal does not leave the user's custody at all — only
    the fee delta does — so the user sees a separate "Swap fees"
    section with the annual totals.
    """
    rows = conn.execute(
        """
        SELECT p.kind,
               p.policy,
               p.swap_fee_msat,
               t_out.asset AS out_asset,
               t_in.asset AS in_asset,
               substr(t_out.occurred_at, 1, 4) AS year
        FROM transaction_pairs p
        JOIN transactions t_out ON t_out.id = p.out_transaction_id
        JOIN transactions t_in ON t_in.id = p.in_transaction_id
        WHERE p.profile_id = ?
          AND p.deleted_at IS NULL
          AND p.swap_fee_msat IS NOT NULL
        UNION ALL
        SELECT p.kind,
               p.policy,
               p.swap_fee_msat,
               t_out.asset AS out_asset,
               p.payout_asset AS in_asset,
               substr(COALESCE(p.payout_occurred_at, t_out.occurred_at), 1, 4) AS year
        FROM direct_swap_payouts p
        JOIN transactions t_out ON t_out.id = p.out_transaction_id
        WHERE p.profile_id = ?
          AND p.deleted_at IS NULL
          AND p.swap_fee_msat IS NOT NULL
        """,
        (profile_id, profile_id),
    ).fetchall()
    if not rows:
        return []

    per_year = defaultdict(lambda: {"count": 0, "total_msat": 0})
    grand = {"count": 0, "total_msat": 0}
    for row in rows:
        try:
            same_asset = normalize_asset_code(row["out_asset"]) == normalize_asset_code(row["in_asset"])
        except (TypeError, ValueError):
            same_asset = False
        if same_asset and str(row["kind"] or "").strip().lower() not in SWAP_FEE_PAIR_KINDS:
            continue
        year_str = row["year"] or ""
        if not year_str.isdigit():
            continue
        year = int(year_str)
        fee = int(row["swap_fee_msat"] or 0)
        per_year[year]["count"] += 1
        per_year[year]["total_msat"] += fee
        grand["count"] += 1
        grand["total_msat"] += fee

    output = []
    for year in sorted(per_year):
        bucket = per_year[year]
        output.append(
            {
                "row_type": "swap_fees_year",
                "year": year,
                "count": bucket["count"],
                "total_swap_fee_msat": bucket["total_msat"],
                "total_swap_fee": float(msat_to_btc(bucket["total_msat"])),
            }
        )
    output.append(
        {
            "row_type": "swap_fees_total",
            "count": grand["count"],
            "total_swap_fee_msat": grand["total_msat"],
            "total_swap_fee": float(msat_to_btc(grand["total_msat"])),
        }
    )
    return output


def _require_austrian_e1kv_profile(profile):
    tax_country = str(profile["tax_country"] or "").strip().lower()
    if tax_country != "at":
        raise AppError(
            "Austrian E 1kv export requires an Austrian tax profile",
            code="validation",
            hint="Use `profiles set --tax-country at --fiat-currency EUR` and re-run `journals process` first.",
            details={"tax_country": profile["tax_country"]},
        )
    fiat_currency = str(profile["fiat_currency"] or "").strip().upper()
    if fiat_currency != "EUR":
        raise AppError(
            "Austrian E 1kv export requires EUR journal amounts",
            code="validation",
            hint="Use an Austrian profile with --fiat-currency EUR and re-run `journals process`.",
            details={"fiat_currency": profile["fiat_currency"]},
        )


def _normalize_tax_year(year):
    if year is None:
        raise AppError("--year is required for Austrian E 1kv export", code="validation")
    try:
        normalized = int(year)
    except (TypeError, ValueError) as exc:
        raise AppError("--year must be a four-digit tax year", code="validation") from exc
    if normalized < 2009 or normalized > 2100:
        raise AppError("--year must be a plausible four-digit tax year", code="validation")
    return normalized


def _eur_cents(value):
    if value is None:
        return None
    rounded = dec(value).quantize(EUR_CENT, rounding=ROUND_HALF_UP)
    return int(rounded * 100)


def _eur_from_cents(cents):
    return Decimal(int(cents or 0)) / Decimal("100")


def _xlsx_eur_from_cents(cents):
    if cents is None:
        return None
    return float(_eur_from_cents(cents))


def _report_eur_cents(cents):
    return _report_fiat(_eur_from_cents(cents))


def _at_regime_from_category(category):
    if not category:
        return ""
    if str(category).startswith("neu_"):
        return "neu"
    if str(category).startswith("alt_"):
        return "alt"
    if str(category).startswith("income_"):
        return "income"
    return ""


def _austrian_e1kv_form_amount(row, kennzahl):
    gain_loss = dec(row["gain_loss"] or 0)
    if str(row["entry_type"]) == "income":
        return gain_loss
    if kennzahl == 176:
        return abs(gain_loss)
    if kennzahl in {172, 174, 801}:
        return gain_loss
    return Decimal("0")


def _austrian_kennzahl_form(kennzahl):
    return AUSTRIAN_KENNZAHL_FORM_BY_CODE.get(kennzahl, "")


def _austrian_kennzahl_form_section(kennzahl):
    return AUSTRIAN_KENNZAHL_FORM_SECTION_BY_CODE.get(kennzahl, "")


def _austrian_e1kv_detail_row(row):
    category = row["at_category"]
    kennzahl = kennzahl_for_disposal_category(category)
    quantity_msat = abs(int(row["quantity"] or 0))
    quantity = msat_to_btc(quantity_msat)
    proceeds = dec(row["proceeds"] or 0)
    cost_basis = dec(row["cost_basis"] or 0)
    gain_loss = dec(row["gain_loss"] or 0)
    income = gain_loss if str(row["entry_type"]) == "income" else Decimal("0")
    price_basis = income if str(row["entry_type"]) == "income" else proceeds
    price = price_basis / quantity if quantity else None
    form_amount = _austrian_e1kv_form_amount(row, kennzahl)
    occurred_at = str(row["occurred_at"])
    note = row["transaction_note"] or row["description"] or ""
    return {
        "tax_year": int(occurred_at[:4]),
        "date": occurred_at[:10],
        "tx_id": row["transaction_external_id"] or row["transaction_id"],
        "transaction_id": row["transaction_id"],
        "wallet": row["wallet"],
        "asset": row["asset"],
        "kind": row["transaction_kind"] or row["entry_type"],
        "entry_type": row["entry_type"],
        "at_category": category,
        "at_category_label": AUSTRIAN_E1KV_CATEGORY_LABELS.get(category, ""),
        "at_regime": _at_regime_from_category(category),
        "qty_msat": quantity_msat,
        "quantity": float(quantity),
        "price_eur_cents": _eur_cents(price),
        "cost_basis_eur_cents": _eur_cents(cost_basis),
        "proceeds_eur_cents": _eur_cents(proceeds),
        "gain_loss_eur_cents": _eur_cents(gain_loss),
        "income_eur_cents": _eur_cents(income),
        "form_amount_eur_cents": _eur_cents(form_amount),
        "holding_period_days": None,
        "kennzahl": kennzahl,
        "stored_kennzahl": row["at_kennzahl"],
        "form": _austrian_kennzahl_form(kennzahl),
        "form_section": _austrian_kennzahl_form_section(kennzahl),
        "note": note,
    }


def _austrian_e1kv_rows(conn, profile, tax_year):
    where = [
        "je.profile_id = ?",
        "je.at_category IS NOT NULL",
        "je.at_category != 'neu_swap'",
        "je.entry_type NOT IN ('fee', 'transfer_fee')",
        "COALESCE(t.taxability_override, 1) != 0",
    ]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        where.append("substr(je.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    rows = conn.execute(
        f"""
        SELECT
            je.occurred_at,
            je.transaction_id,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl,
            w.label AS wallet,
            t.external_id AS transaction_external_id,
            t.kind AS transaction_kind,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        params,
    ).fetchall()
    return [_austrian_e1kv_detail_row(row) for row in rows]


def _austrian_e1kv_quarantines(conn, profile, tax_year):
    where = ["jq.profile_id = ?"]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        where.append("substr(t.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    return [
        {"reason": row["reason"], "count": int(row["count"] or 0)}
        for row in conn.execute(
            f"""
            SELECT jq.reason, COUNT(*) AS count
            FROM journal_quarantines jq
            JOIN transactions t ON t.id = jq.transaction_id
            WHERE {' AND '.join(where)}
            GROUP BY jq.reason
            ORDER BY count DESC, jq.reason ASC
            """,
            params,
        ).fetchall()
    ]


def _austrian_e1kv_transaction_rows(conn, profile, tax_year):
    where = ["t.profile_id = ?", "t.excluded = 0"]
    params: list[Any] = [profile["id"]]
    if tax_year is not None:
        where.append("substr(t.occurred_at, 1, 4) = ?")
        params.append(str(tax_year))
    rows = conn.execute(
        f"""
        SELECT
            t.id AS transaction_id,
            t.external_id,
            t.occurred_at,
            w.label AS wallet,
            t.direction,
            t.kind,
            t.asset,
            t.amount,
            t.fee,
            t.fiat_value,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            GROUP_CONCAT(tags.code, ', ') AS tags
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN transaction_tags tt ON tt.transaction_id = t.id
        LEFT JOIN tags ON tags.id = tt.tag_id
        WHERE {' AND '.join(where)}
        GROUP BY t.id
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        params,
    ).fetchall()
    return [
        {
            "transaction_id": row["transaction_id"],
            "external_id": row["external_id"] or "",
            "occurred_at": row["occurred_at"],
            "wallet": row["wallet"],
            "direction": row["direction"],
            "kind": row["kind"] or "",
            "asset": row["asset"],
            "amount": float(msat_to_btc(row["amount"] or 0)),
            "amount_msat": int(row["amount"] or 0),
            "fee": float(msat_to_btc(row["fee"] or 0)),
            "fee_msat": int(row["fee"] or 0),
            "fiat_value": float(dec(row["fiat_value"] or 0)),
            "description": row["description"] or "",
            "note": row["note"] or "",
            "tags": row["tags"] or "",
        }
        for row in rows
    ]


def _austrian_e1kv_summary_rows(rows):
    totals = defaultdict(lambda: {"amount": 0, "count": 0})
    for row in rows:
        kennzahl = row["kennzahl"]
        if kennzahl is None:
            continue
        totals[kennzahl]["amount"] += int(row["form_amount_eur_cents"] or 0)
        totals[kennzahl]["count"] += 1

    codes = [
        *AUSTRIAN_E1KV_SUPPORTED_KENNZAHL_ORDER,
        *AUSTRIAN_OUTSIDE_E1KV_KENNZAHL_ORDER,
    ]
    for code in sorted(code for code in totals if code not in codes):
        codes.append(code)

    return [
        {
            "kennzahl": code,
            "label": AUSTRIAN_E1KV_KENNZAHL_LABELS.get(code, ""),
            "form": _austrian_kennzahl_form(code),
            "form_section": _austrian_kennzahl_form_section(code),
            "row_count": totals[code]["count"],
            "amount_eur_cents": totals[code]["amount"],
        }
        for code in codes
    ]


def _austrian_e1kv_kennzahl_totals(summary_rows):
    return {
        str(row["kennzahl"]): {
            "label": row["label"],
            "form": row.get("form", ""),
            "form_section": row.get("form_section", ""),
            "row_count": row["row_count"],
            "amount_eur_cents": row["amount_eur_cents"],
        }
        for row in summary_rows
    }


def build_austrian_kennzahl_summary(conn, profile, tax_year):
    _require_austrian_e1kv_profile(profile)
    normalized_year = _normalize_tax_year(tax_year)
    return _austrian_e1kv_summary_rows(
        _austrian_e1kv_rows(conn, profile, normalized_year)
    )


def _austrian_tax_empty_section(section_id):
    metadata = AUSTRIAN_TAX_SECTION_METADATA[section_id]
    return {
        "section_id": section_id,
        "label": metadata["label"],
        "law": metadata["law"],
        "supported": bool(metadata["supported"]),
        "status": "supported" if metadata["supported"] else "not_modelled",
        "kennzahlen": list(metadata["kennzahlen"]),
        "totals": {
            "row_count": 0,
            "quantity_msat": 0,
            "amount_eur_cents": 0,
            "proceeds_eur_cents": 0,
            "cost_basis_eur_cents": 0,
            "gain_loss_eur_cents": 0,
            "income_eur_cents": 0,
        },
        "detail_rows": [],
    }


def _austrian_tax_section_id(row):
    category = row["at_category"]
    if category in {"neu_gain", "neu_loss", "neu_swap"}:
        return "1.1"
    if category == "alt_spekulation":
        # Taxable Spekulation (E 1 KZ 801) — not the non-taxable Alt bucket.
        return "1.3"
    if category == "income_capital_yield":
        return "2.1"
    if category == "income_general":
        return "2.2"
    if category == "alt_taxfree":
        return "3.1"
    return None


def _austrian_tax_sections(rows):
    sections = {
        section_id: _austrian_tax_empty_section(section_id)
        for section_id in AUSTRIAN_TAX_SECTION_ORDER
    }
    for row in rows:
        section_id = _austrian_tax_section_id(row)
        if section_id is None:
            continue
        section = sections[section_id]
        totals = section["totals"]
        section["detail_rows"].append(row)
        totals["row_count"] += 1
        totals["quantity_msat"] += int(row["qty_msat"] or 0)
        totals["amount_eur_cents"] += int(row["form_amount_eur_cents"] or 0)
        totals["proceeds_eur_cents"] += int(row["proceeds_eur_cents"] or 0)
        totals["cost_basis_eur_cents"] += int(row["cost_basis_eur_cents"] or 0)
        totals["gain_loss_eur_cents"] += int(row["gain_loss_eur_cents"] or 0)
        totals["income_eur_cents"] += int(row["income_eur_cents"] or 0)
    return sections


def _austrian_section_title(section_id, section):
    law = f" ({section['law']})" if section["law"] else ""
    return f"{section_id}. {section['label']}{law}"


def _austrian_section_amount(section):
    return _report_eur_cents(section["totals"]["amount_eur_cents"])


def _austrian_unsupported_section_lines(section_id, section):
    title = _austrian_section_title(section_id, section)
    return [
        title,
        "-" * len(title),
        "Status: not modelled in Kassiber yet; placeholder total is 0.00 EUR.",
        "Amount: 0.00 EUR",
        "",
    ]


def _austrian_disposal_split(rows):
    split = {
        "gains_proceeds": 0,
        "gains_cost_basis": 0,
        "gains_amount": 0,
        "losses_proceeds": 0,
        "losses_cost_basis": 0,
        "losses_amount": 0,
    }
    for row in rows:
        if row["kennzahl"] == 176:
            split["losses_proceeds"] += int(row["proceeds_eur_cents"] or 0)
            split["losses_cost_basis"] += int(row["cost_basis_eur_cents"] or 0)
            split["losses_amount"] += int(row["form_amount_eur_cents"] or 0)
        elif row["kennzahl"] == 174:
            split["gains_proceeds"] += int(row["proceeds_eur_cents"] or 0)
            split["gains_cost_basis"] += int(row["cost_basis_eur_cents"] or 0)
            split["gains_amount"] += int(row["form_amount_eur_cents"] or 0)
    return split


def _austrian_e1kv_overview_entries(report):
    sections = report["sections"]
    split_11 = _austrian_disposal_split(sections["1.1"]["detail_rows"])
    entries = []

    def heading(text):
        entries.append(("heading", text))

    def section(text):
        entries.append(("section", text))

    def amount(label, cents, total=False):
        entries.append(("amount", label, int(cents or 0), bool(total)))

    heading("1. Steuerpflichtige Einkünfte aus dem Handel mit Kryptowährungen")
    section("1.1. Steuerpflichtige Einkünfte aus dem An- und Verkauf von Kryptowährungen")
    amount("Veräußerungspreis", split_11["gains_proceeds"])
    amount("Anschaffungskosten", split_11["gains_cost_basis"])
    amount("Veräußerungsgewinn", split_11["gains_amount"], total=True)
    amount("Veräußerungspreis", split_11["losses_proceeds"])
    amount("Anschaffungskosten", split_11["losses_cost_basis"])
    amount("Veräußerungsverlust", split_11["losses_amount"], total=True)
    section("1.2. Steuerpflichtige Einkünfte aus Margin, Derivaten und Futures")
    amount("Gewinne aus Margin, Derivaten und Futures", 0, total=True)
    amount("Verluste aus Margin, Derivaten und Futures", 0, total=True)
    section("1.3. Steuerpflichtige Einkünfte aus Spekulationsgeschäften (§ 31 EStG)")
    amount(
        "Summe Spekulationseinkünfte",
        sections["1.3"]["totals"]["amount_eur_cents"],
        total=True,
    )

    heading("2. Steuerpflichtige laufende Einkünfte")
    section("2.1. Einkünfte aus der Überlassung von Kryptowährungen")
    amount("Summe laufende Einkünfte", sections["2.1"]["totals"]["amount_eur_cents"], total=True)
    section("2.2. Einkünfte aus Leistungen zur Transaktionsverarbeitung")
    amount("Summe laufende Einkünfte", sections["2.2"]["totals"]["amount_eur_cents"], total=True)

    heading("3. Nicht steuerbare Einkünfte")
    section("3.1. Nicht steuerbare Einkünfte aus Altvermögen außerhalb der Spekulationsfrist")
    amount("Summe nicht steuerbare Einkünfte", sections["3.1"]["totals"]["amount_eur_cents"], total=True)
    section("3.2. Nicht steuerbare Einkünfte gem. § 27b Abs 2 Z 2 Satz 2 EStG")
    amount("Summe nicht steuerbare Einkünfte", 0, total=True)
    section("3.3. Nicht steuerbare Steuergebühren und Rückerstattungen")
    amount("Summe entrichtete Steuergebühren", 0, total=True)
    amount("Summe Rückerstattungen", 0, total=True)

    heading("4. Sonstige Ein- und Ausgänge")
    section("4.1. Eingegangene Spenden/Trinkgeld")
    amount("Summe Spenden/Trinkgeld", 0, total=True)
    section("4.2. Ausgegangene Spenden/Schenkungen")
    amount("Summe Spenden", 0, total=True)
    amount("Summe Schenkungen", 0, total=True)
    section("4.3. Gestohlene, gehackte und verlorene Coins")
    amount("Summe gestohlen/gehackt/Betrug", 0, total=True)
    amount("Summe Verlust", 0, total=True)
    section("4.4. Mining (kommerziell)")
    amount("Summe Mining", 0, total=True)
    section("4.5. Minting")
    amount("Summe Minting", 0, total=True)
    return entries


def _austrian_e1kv_assumptions(rows):
    assumptions = [
        {
            "code": "AT-E1KV-FOREIGN-SELF-CUSTODY",
            "severity": "review",
            "message": AUSTRIAN_E1KV_SELF_CUSTODY_ASSUMPTION,
        },
        {
            "code": "AT-E1KV-DETAIL-LIMITATION",
            "severity": "review",
            "message": AUSTRIAN_E1KV_DETAIL_LIMITATION,
        },
        {
            "code": "AT-E1KV-KENNZAHL-REPROCESS",
            "severity": "review",
            "message": AUSTRIAN_E1KV_REPROCESS_HINT,
        },
    ]
    if any(str(row["asset"]).upper() == "LBTC" for row in rows):
        assumptions.append(
            {
                "code": "AT-002",
                "severity": "review",
                "message": "L-BTC is treated as Kryptowaehrung like BTC for this report period.",
            }
        )
    if any(str(row["kind"]).lower() == "routing_income" for row in rows):
        assumptions.append(
            {
                "code": "AT-001",
                "severity": "review",
                "message": "Lightning routing fees are treated as laufende Einkuenfte at fair market value.",
            }
        )
    assumptions.append(
        {
            "code": "AT-REVIEW-GATE",
            "severity": "review",
            "message": AUSTRIAN_E1KV_REVIEW_GATE,
        }
    )
    return assumptions


def _austrian_e1kv_mismatches(rows):
    mismatches = []
    for row in rows:
        stored = row["stored_kennzahl"]
        current = row["kennzahl"]
        if stored is not None and current is not None and int(stored) != int(current):
            mismatches.append(
                {
                    "tx_id": row["tx_id"],
                    "at_category": row["at_category"],
                    "stored_kennzahl": stored,
                    "export_kennzahl": current,
                }
            )
    return mismatches


def _austrian_e1kv_mismatch_table_rows(report):
    return [
        [
            row["tx_id"],
            AUSTRIAN_E1KV_CATEGORY_LABELS.get(row["at_category"], row["at_category"]),
            row["stored_kennzahl"],
            row["export_kennzahl"],
        ]
        for row in report["data_quality"]["kennzahl_mismatches"]
    ]


def _austrian_kennzahl_table_rows(summary_rows):
    return [
        [
            str(row["kennzahl"]),
            row["label"],
            _report_count(row["row_count"]),
            _report_eur_cents(row["amount_eur_cents"]),
        ]
        for row in summary_rows
    ]


def report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    _require_austrian_e1kv_profile(profile)
    hooks.require_processed_journals(conn, profile)
    normalized_year = _normalize_tax_year(tax_year)
    rows = _austrian_e1kv_rows(conn, profile, normalized_year)
    quarantines = _austrian_e1kv_quarantines(conn, profile, normalized_year)
    summary_rows = _austrian_e1kv_summary_rows(rows)
    return {
        "workspace": workspace["label"],
        "profile": profile["label"],
        "tax_year": normalized_year,
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "form": "E 1kv",
        "form_section": AUSTRIAN_E1KV_FORM_SECTION,
        "review_gate": AUSTRIAN_E1KV_REVIEW_GATE,
        "assumptions": _austrian_e1kv_assumptions(rows),
        "summary_rows": summary_rows,
        "kennzahl_totals": _austrian_e1kv_kennzahl_totals(summary_rows),
        "section_order": list(AUSTRIAN_TAX_SECTION_ORDER),
        "sections": _austrian_tax_sections(rows),
        "rows": rows,
        "data_quality": {
            "quarantines": quarantines,
            "kennzahl_mismatches": _austrian_e1kv_mismatches(rows),
        },
    }


def _build_austrian_e1kv_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    scope = str(report["tax_year"])
    title = f"Kassiber Austrian E 1kv / Steuerbericht - {report['profile']} ({scope})"
    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Workspace", report["workspace"]),
                ("Profile", report["profile"]),
                ("Tax year", scope),
                ("Fiat currency", report["fiat_currency"]),
                ("Tax country", report["tax_country"]),
                ("Form", report["form"]),
                ("Section", report["form_section"]),
            ]
        )
    )

    lines.extend(["", "Hinweise zum Berichtsumfang", "---------------------------"])
    for assumption in report["assumptions"]:
        lines.append(f"{assumption['code']}: {assumption['message']}")

    e1kv_rows = [row for row in report["summary_rows"] if row.get("form") == "E 1kv"]
    outside_rows = [row for row in report["summary_rows"] if row.get("form") != "E 1kv"]
    lines.extend(["", "E 1kv Kennzahlen", "-----------------"])
    lines.extend(
        _markdown_table_lines(
            ["KZ", "Description", "Rows", "Amount EUR"],
            _austrian_kennzahl_table_rows(e1kv_rows),
        )
    )
    if outside_rows:
        lines.extend(["", "Other Austrian Kennzahlen", "-------------------------"])
        lines.extend(
            _markdown_table_lines(
                ["Form", "KZ", "Description", "Rows", "Amount EUR"],
                [
                    [
                        row.get("form") or "Other",
                        *_austrian_kennzahl_table_rows([row])[0],
                    ]
                    for row in outside_rows
                ],
            )
        )

    lines.extend(["", "I. Übersicht", "------------"])
    for entry in _austrian_e1kv_overview_entries(report):
        if entry[0] == "heading":
            lines.extend(["", entry[1], "-" * len(entry[1])])
        elif entry[0] == "section":
            lines.extend(["", entry[1]])
        else:
            _kind, label, cents, _is_total = entry
            lines.extend(_report_kv_lines([(label, f"{_report_eur_cents(cents)} EUR")], label_width=54))

    lines.extend(["", "II. Detail Sections", "-------------------"])
    for spec in _austrian_e1kv_section_table_specs(report):
        lines.extend(["", spec["title"], "-" * len(spec["title"])])
        if spec["rows"]:
            lines.extend(
                _markdown_table_lines(
                    spec["headers"],
                    [
                        _austrian_e1kv_render_table_row(row, spec["row_format_names"])
                        for row in spec["rows"]
                    ],
                )
            )
        else:
            lines.append("No rows in scope.")
        if spec["total_rows"]:
            lines.append("")
            lines.extend(
                _report_kv_lines(
                    [
                        (label, f"{_austrian_e1kv_render_cell(value, 'money')} EUR")
                        for label, value in spec["total_rows"]
                    ],
                    label_width=54,
                )
            )

    lines.extend(["", "Data Quality", "------------"])
    quarantines = report["data_quality"]["quarantines"]
    mismatches = report["data_quality"]["kennzahl_mismatches"]
    if quarantines:
        lines.append("Quarantined transactions remain outside this export:")
        lines.extend(
            hooks.format_table(
                ["Reason", "Count"],
                [[row["reason"], _report_count(row["count"])] for row in quarantines],
                [32, 8],
                align_right={1},
            )
        )
    else:
        lines.append("No quarantined transactions in scope.")
    if mismatches:
        lines.append("Some rows had stale stored Kennzahlen; the export used the current category mapping.")
        lines.extend(
            _markdown_table_lines(
                ["Tx ID", "Category", "Stored KZ", "Export KZ"],
                _austrian_e1kv_mismatch_table_rows(report),
            )
        )
    else:
        lines.append("Stored Kennzahlen match the current export mapping.")

    lines.extend(["", "Review Footer", "-------------", AUSTRIAN_E1KV_REVIEW_GATE])
    return title, lines, report


def build_austrian_e1kv_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, tax_year=None):
    _, lines, _ = _build_austrian_e1kv_report_lines(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        tax_year=tax_year,
    )
    return lines


def export_austrian_e1kv_pdf_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, tax_year=None):
    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    _, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    transaction_rows = _austrian_e1kv_transaction_rows(conn, profile, report["tax_year"])
    from ..austrian_pdf_report import write_austrian_e1kv_pdf

    written = dict(
        write_austrian_e1kv_pdf(
            file_path,
            report=report,
            profile=dict(profile),
            portfolio_rows=report_portfolio_summary(conn, workspace_ref, profile_ref, hooks),
            transaction_rows=transaction_rows,
            section_specs=_austrian_e1kv_section_table_specs(report),
            generated_at=hooks.now_iso(),
        )
    )
    written["tax_year"] = report["tax_year"]
    written["form"] = report["form"]
    written["assumptions"] = report["assumptions"]
    written["sections"] = [
        "steuerpflichtige_gesamtuebersicht",
        "steuerpflichtige_detailuebersicht",
        "steuerfreie_gesamtuebersicht",
        "bestandsuebersicht",
        "besonderheiten",
        "erlaeuterungen",
        "transaktionsuebersicht",
        "steuerformulare",
        "faq",
    ]
    written["transactions"] = len(transaction_rows)
    return written


AUSTRIAN_E1KV_XLSX_SHEETS = (
    "Übersicht",
    "1.1.",
    "1.2.",
    "1.3.",
    "2.1.",
    "2.2.",
    "3.1.",
    "3.2.",
    "3.3.",
    "4.1.",
    "4.2.",
    "4.3.",
    "4.4.",
    "4.5.",
    "Erläuterungen zum Steuerreport",
)

AUSTRIAN_E1KV_XLSX_TITLES = {
    "1.1": "1.1. Steuerpflichtige Einkünfte aus dem An- und Verkauf von Kryptowährungen gem. § 27b Abs 3 EStG",
    "1.2": "1.2. Steuerpflichtige Einkünfte aus Margin, Derivaten und Futures",
    "1.3": "1.3. Steuerpflichtige Einkünfte aus Spekulationsgeschäften mit Kryptowährungen (§ 31 EStG)",
    "2.1": "2.1. Steuerpflichtige laufende Einkünfte aus der Überlassung von Kryptowährungen",
    "2.2": "2.2. Steuerpflichtige laufende Einkünfte aus Leistungen zur Transaktionsverarbeitung",
    "3.1": "3.1. Nicht steuerbare Einkünfte aus Altvermögen außerhalb der Spekulationsfrist",
    "3.2": "3.2. Nicht steuerbare Einkünfte gem. § 27b Abs 2 Z 2 Satz 2 EStG",
    "3.3": "3.3. Nicht steuerbare Steuergebühren und Rückerstattungen",
    "4.1": "4.1. Eingegangene Spenden/Trinkgeld",
    "4.2": "4.2. Ausgegangene Spenden/Schenkungen",
    "4.3": "4.3. Gestohlene, gehackte und verlorene Coins",
    "4.4": "4.4. Mining (kommerziell)",
    "4.5": "4.5. Minting",
}

AUSTRIAN_E1KV_XLSX_DISPOSAL_HEADERS = (
    "Börse",
    "Anlage",
    "Anzahl",
    "Erwerbsdatum",
    "Verkaufsdatum",
    "Kauf/Eingang bei",
    "Typ",
    "Kostenbasis in EUR",
    "Erlös in EUR",
    "Gewinn/Verlust in EUR",
)
AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS = (
    "Börse",
    "Anlage",
    "Anzahl",
    "Erwerbsdatum",
    "Verkaufsdatum",
    "Kauf/Eingang bei",
    "Haltedauer in Tagen",
    "Typ",
    "Kostenbasis in EUR",
    "Erlös in EUR",
    "Gewinn/Verlust in EUR",
)
AUSTRIAN_E1KV_XLSX_INCOME_HEADERS = (
    "Börse",
    "Typ",
    "Datum des Eingangs",
    "Anzahl",
    "Währung",
    "Hinweis",
    "Wert in EUR zum Zeitpunkt des Eingangs",
)
AUSTRIAN_E1KV_XLSX_MARGIN_HEADERS = (
    "Börse",
    "Typ",
    "Datum",
    "Anzahl",
    "Währung",
    "Gesamt",
)
AUSTRIAN_E1KV_XLSX_FEE_HEADERS = (
    "Börse",
    "Datum der Gebühr",
    "Anzahl",
    "Währung",
    "Erlös in EUR",
    "Anzahl",
    "Währung",
    "Hinweis",
)
AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS = (
    "Börse",
    "Typ",
    "Datum der Auszahlung",
    "Anzahl",
    "Währung",
    "Hinweis",
    "Kostenbasis in EUR",
    "Wert bei Auszahlung in EUR",
)


def _austrian_e1kv_xlsx_formats(workbook):
    return {
        "overview_title": workbook.add_format(
            {"bold": True, "font_size": 16, "valign": "vcenter"}
        ),
        "overview_group": workbook.add_format(
            {"bold": True, "font_size": 12, "valign": "vcenter"}
        ),
        "overview_section": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter"}
        ),
        "overview_label": workbook.add_format({"font_size": 11, "valign": "vcenter"}),
        "overview_money": workbook.add_format(
            {"font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "overview_currency": workbook.add_format({"font_size": 11, "valign": "vcenter"}),
        "overview_total_label": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter"}
        ),
        "overview_total_money": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "detail_title": workbook.add_format(
            {"bold": True, "font_size": 13, "align": "center", "valign": "vcenter", "text_wrap": True}
        ),
        "header": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "top", "text_wrap": True}
        ),
        "text": workbook.add_format({"font_size": 11, "valign": "top", "text_wrap": True}),
        "int": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0"}),
        "quantity": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "0.00000000"}
        ),
        "money": workbook.add_format(
            {"font_size": 11, "valign": "top", "num_format": "#,##0.00"}
        ),
        "total_label": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "text_wrap": True}
        ),
        "total_money": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "vcenter", "num_format": "#,##0.00"}
        ),
        "explanation_title": workbook.add_format(
            {"bold": True, "font_size": 16, "valign": "vcenter"}
        ),
        "explanation_heading": workbook.add_format(
            {"bold": True, "font_size": 12, "valign": "top"}
        ),
        "explanation_text": workbook.add_format(
            {"font_size": 11, "valign": "top", "text_wrap": True}
        ),
    }


def _xlsx_write_value(worksheet, row_index, column_index, value, cell_format):
    if value is None or value == "":
        worksheet.write_blank(row_index, column_index, None, cell_format)
    elif isinstance(value, bool):
        worksheet.write_boolean(row_index, column_index, value, cell_format)
    elif isinstance(value, (int, float, Decimal)):
        worksheet.write_number(row_index, column_index, float(value), cell_format)
    else:
        worksheet.write_string(row_index, column_index, str(value), cell_format)


def _austrian_e1kv_xlsx_hint(row):
    tx_id = str(row.get("tx_id") or "")
    note = str(row.get("note") or "")
    return tx_id or note


def _austrian_e1kv_xlsx_category(row):
    return row.get("at_category_label") or row.get("at_category") or row.get("kind") or ""


def _austrian_e1kv_xlsx_disposal_values(row, include_holding_days=False):
    values = [
        row.get("wallet") or "",
        row.get("asset") or "",
        row.get("quantity"),
        "",
        row.get("date") or "",
        "",
    ]
    if include_holding_days:
        values.append(row.get("holding_period_days"))
    values.extend(
        [
            _austrian_e1kv_xlsx_category(row),
            _xlsx_eur_from_cents(row.get("cost_basis_eur_cents")),
            _xlsx_eur_from_cents(row.get("proceeds_eur_cents")),
            _xlsx_eur_from_cents(row.get("gain_loss_eur_cents")),
        ]
    )
    return values


def _austrian_e1kv_xlsx_income_values(row):
    return [
        row.get("wallet") or "",
        row.get("kind") or row.get("entry_type") or "",
        row.get("date") or "",
        row.get("quantity"),
        row.get("asset") or "",
        _austrian_e1kv_xlsx_hint(row),
        _xlsx_eur_from_cents(row.get("form_amount_eur_cents")),
    ]


def _austrian_e1kv_render_cell(value, format_name):
    if value is None or value == "":
        return ""
    if format_name == "money":
        return _report_fiat(Decimal(str(value)))
    if format_name == "quantity":
        return f"{Decimal(str(value)):.8f}"
    if format_name == "int":
        return str(int(value))
    return str(value)


def _austrian_e1kv_render_table_row(values, format_names):
    return [
        _austrian_e1kv_render_cell(
            value,
            format_names[index] if index < len(format_names) else "text",
        )
        for index, value in enumerate(values)
    ]


def _austrian_e1kv_csv_filename(index, stem):
    return f"{int(index):02d}_{stem}.csv"


def _austrian_e1kv_section_table_specs(report):
    sections = report["sections"]
    split_11 = _austrian_disposal_split(sections["1.1"]["detail_rows"])
    disposal_formats = ("text", "text", "quantity", "text", "text", "text", "text", "money", "money", "money")
    holding_formats = (
        "text",
        "text",
        "quantity",
        "text",
        "text",
        "text",
        "int",
        "text",
        "money",
        "money",
        "money",
    )
    income_formats = ("text", "text", "text", "quantity", "text", "text", "money")
    margin_formats = ("text", "text", "text", "quantity", "text", "money")
    fee_formats = ("text", "text", "quantity", "text", "money", "quantity", "text", "text")
    outgoing_formats = ("text", "text", "text", "quantity", "text", "text", "money", "money")
    return [
        {
            "sheet_name": "1.1.",
            "filename": _austrian_e1kv_csv_filename(1, "1.1"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.1"],
            "headers": AUSTRIAN_E1KV_XLSX_DISPOSAL_HEADERS,
            "rows": [_austrian_e1kv_xlsx_disposal_values(row) for row in sections["1.1"]["detail_rows"]],
            "row_format_names": disposal_formats,
            "total_rows": [
                ("Summe Einkünfte realisierten Wertsteigerungen", _xlsx_eur_from_cents(split_11["gains_amount"])),
                ("Summe realisierte Wertverluste", _xlsx_eur_from_cents(split_11["losses_amount"])),
            ],
            "value_column": 9,
            "column_widths": (18, 12, 14, 15, 15, 18, 24, 18, 18, 20),
        },
        {
            "sheet_name": "1.2.",
            "filename": _austrian_e1kv_csv_filename(2, "1.2"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.2"],
            "headers": AUSTRIAN_E1KV_XLSX_MARGIN_HEADERS,
            "rows": [],
            "row_format_names": margin_formats,
            "total_rows": [
                ("Summe Gewinne aus Margin, Derivaten, Futures", 0.0),
                ("Summe Verluste aus Margin, Derivaten, Futures", 0.0),
            ],
            "value_column": 5,
            "column_widths": (18, 22, 16, 14, 12, 18),
        },
        {
            "sheet_name": "1.3.",
            "filename": _austrian_e1kv_csv_filename(3, "1.3"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["1.3"],
            "headers": AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS,
            "rows": [
                _austrian_e1kv_xlsx_disposal_values(row, include_holding_days=True)
                for row in sections["1.3"]["detail_rows"]
            ],
            "row_format_names": holding_formats,
            "total_rows": [
                (
                    "Summe Spekulationseinkünfte",
                    _xlsx_eur_from_cents(sections["1.3"]["totals"]["amount_eur_cents"]),
                )
            ],
            "value_column": 10,
            "column_widths": (18, 12, 14, 15, 15, 18, 16, 24, 18, 18, 20),
        },
        *[
            {
                "sheet_name": f"{section_id}.",
                "filename": _austrian_e1kv_csv_filename(index, section_id),
                "title": AUSTRIAN_E1KV_XLSX_TITLES[section_id],
                "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
                "rows": [_austrian_e1kv_xlsx_income_values(row) for row in sections[section_id]["detail_rows"]],
                "row_format_names": income_formats,
                "total_rows": [
                    (
                        "Summe laufende Einkünfte",
                        _xlsx_eur_from_cents(sections[section_id]["totals"]["amount_eur_cents"]),
                    )
                ],
                "value_column": 6,
                "column_widths": (18, 24, 18, 14, 12, 36, 24),
            }
            for index, section_id in ((4, "2.1"), (5, "2.2"))
        ],
        {
            "sheet_name": "3.1.",
            "filename": _austrian_e1kv_csv_filename(6, "3.1"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.1"],
            "headers": AUSTRIAN_E1KV_XLSX_HOLDING_HEADERS,
            "rows": [
                _austrian_e1kv_xlsx_disposal_values(row, include_holding_days=True)
                for row in sections["3.1"]["detail_rows"]
            ],
            "row_format_names": holding_formats,
            "total_rows": [
                ("Summe nicht steuerbare Einkünfte", _xlsx_eur_from_cents(sections["3.1"]["totals"]["amount_eur_cents"]))
            ],
            "value_column": 10,
            "column_widths": (18, 12, 14, 15, 15, 18, 16, 24, 18, 18, 20),
        },
        {
            "sheet_name": "3.2.",
            "filename": _austrian_e1kv_csv_filename(7, "3.2"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.2"],
            "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
            "rows": [],
            "row_format_names": income_formats,
            "total_rows": [("Summe nicht steuerbare Einkünfte", 0.0)],
            "value_column": 6,
            "column_widths": (18, 24, 18, 14, 12, 36, 24),
        },
        {
            "sheet_name": "3.3.",
            "filename": _austrian_e1kv_csv_filename(8, "3.3"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["3.3"],
            "headers": AUSTRIAN_E1KV_XLSX_FEE_HEADERS,
            "rows": [],
            "row_format_names": fee_formats,
            "total_rows": [
                ("Summe entrichtete Steuergebühren", 0.0),
                ("Summe der Rückerstattungen", 0.0),
            ],
            "value_column": 4,
            "column_widths": (18, 18, 14, 12, 18, 14, 12, 32),
        },
        {
            "sheet_name": "4.1.",
            "filename": _austrian_e1kv_csv_filename(9, "4.1"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.1"],
            "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
            "rows": [],
            "row_format_names": income_formats,
            "total_rows": [("Summe Spenden/Trinkgeld", 0.0)],
            "value_column": 6,
            "column_widths": (18, 24, 18, 14, 12, 36, 24),
        },
        {
            "sheet_name": "4.2.",
            "filename": _austrian_e1kv_csv_filename(10, "4.2"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.2"],
            "headers": AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS,
            "rows": [],
            "row_format_names": outgoing_formats,
            "total_rows": [("Summe Spenden", 0.0), ("Summe Schenkungen", 0.0)],
            "value_column": 7,
            "column_widths": (18, 22, 18, 14, 12, 36, 18, 24),
        },
        {
            "sheet_name": "4.3.",
            "filename": _austrian_e1kv_csv_filename(11, "4.3"),
            "title": AUSTRIAN_E1KV_XLSX_TITLES["4.3"],
            "headers": AUSTRIAN_E1KV_XLSX_OUTGOING_HEADERS,
            "rows": [],
            "row_format_names": outgoing_formats,
            "total_rows": [("Summe gestohlen/gehackt/Betrug", 0.0), ("Summe Verlust", 0.0)],
            "value_column": 7,
            "column_widths": (18, 22, 18, 14, 12, 36, 18, 24),
        },
        *[
            {
                "sheet_name": f"{section_id}.",
                "filename": _austrian_e1kv_csv_filename(index, section_id),
                "title": AUSTRIAN_E1KV_XLSX_TITLES[section_id],
                "headers": AUSTRIAN_E1KV_XLSX_INCOME_HEADERS,
                "rows": [],
                "row_format_names": income_formats,
                "total_rows": [(total_label, 0.0)],
                "value_column": 6,
                "column_widths": (18, 24, 18, 14, 12, 36, 24),
            }
            for index, section_id, total_label in ((12, "4.4", "Summe Mining"), (13, "4.5", "Summe Minting"))
        ],
    ]


def _austrian_e1kv_xlsx_write_total_row(worksheet, row_index, label, value, value_column, formats):
    if value_column > 0:
        worksheet.merge_range(row_index, 0, row_index, value_column - 1, label, formats["total_label"])
    else:
        worksheet.write_string(row_index, 0, label, formats["total_label"])
    _xlsx_write_value(worksheet, row_index, value_column, value, formats["total_money"])


def _austrian_e1kv_xlsx_write_detail_sheet(
    workbook,
    sheet_name,
    title,
    headers,
    rows,
    row_format_names,
    total_rows,
    value_column,
    formats,
    column_widths,
):
    worksheet = workbook.add_worksheet(sheet_name)
    worksheet.set_landscape()
    worksheet.fit_to_pages(1, 0)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    last_column = len(headers) - 1
    widths = [column_widths[i] if i < len(column_widths) else 16 for i in range(len(headers))]
    worksheet.set_row(0, 31)
    worksheet.merge_range(0, 0, 0, last_column, title, formats["detail_title"])
    header_lines = max((_estimate_wrapped_lines(header, widths[i]) for i, header in enumerate(headers)), default=1)
    worksheet.set_row(1, max(34, header_lines * 15 + 6))
    for column_index, header in enumerate(headers):
        worksheet.set_column(column_index, column_index, widths[column_index])
        worksheet.write_string(1, column_index, header, formats["header"])

    row_index = 2
    if rows:
        for values in rows:
            lines = max(
                (_estimate_wrapped_lines(values[i], widths[i]) for i in range(min(len(values), len(widths)))),
                default=1,
            )
            worksheet.set_row(row_index, max(22, lines * 15 + 4))
            for column_index, value in enumerate(values):
                format_name = row_format_names[column_index] if column_index < len(row_format_names) else "text"
                _xlsx_write_value(worksheet, row_index, column_index, value, formats[format_name])
            row_index += 1
    else:
        worksheet.set_row(row_index, 20)
        for column_index in range(len(headers)):
            worksheet.write_blank(row_index, column_index, None, formats["text"])
        row_index += 1

    row_index += 1
    total_label_width = sum(widths[:value_column]) if value_column > 0 else (widths[0] if widths else 16)
    for label, value in total_rows:
        worksheet.set_row(row_index, max(22, _estimate_wrapped_lines(label, total_label_width) * 15 + 4))
        _austrian_e1kv_xlsx_write_total_row(
            worksheet,
            row_index,
            label,
            value,
            value_column,
            formats,
        )
        row_index += 1
    return worksheet


def _austrian_e1kv_xlsx_write_overview(report, workbook, formats):
    worksheet = workbook.add_worksheet("Übersicht")
    worksheet.set_column(0, 0, 72)
    worksheet.set_column(1, 1, 20)
    worksheet.set_column(2, 2, 8)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    worksheet.set_row(0, 28)
    worksheet.merge_range(0, 0, 0, 2, "I. Übersicht", formats["overview_title"])
    row_index = 2
    for entry in _austrian_e1kv_overview_entries(report):
        if entry[0] == "heading":
            worksheet.set_row(row_index, max(25, _estimate_wrapped_lines(entry[1], 100) * 15 + 6))
            worksheet.merge_range(row_index, 0, row_index, 2, entry[1], formats["overview_group"])
            row_index += 2
        elif entry[0] == "section":
            worksheet.set_row(row_index, max(25, _estimate_wrapped_lines(entry[1], 100) * 15 + 6))
            worksheet.merge_range(row_index, 0, row_index, 2, entry[1], formats["overview_section"])
            row_index += 1
        else:
            _kind, label, cents, total = entry
            worksheet.set_row(row_index, max(22, _estimate_wrapped_lines(label, 72) * 15 + 4))
            label_format = formats["overview_total_label"] if total else formats["overview_label"]
            value_format = formats["overview_total_money"] if total else formats["overview_money"]
            worksheet.write_string(row_index, 0, label, label_format)
            worksheet.write_number(row_index, 1, float(_eur_from_cents(cents)), value_format)
            worksheet.write_string(row_index, 2, "EUR", formats["overview_currency"])
            row_index += 1
    return worksheet


def _austrian_e1kv_xlsx_write_explanations(report, workbook, formats):
    worksheet = workbook.add_worksheet("Erläuterungen zum Steuerreport")
    worksheet.set_column(0, 0, 105)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)

    rows = [
        ("Erläuterungen zum Steuerreport", "explanation_title"),
        ("", "explanation_text"),
        ("Berichtsumfang", "explanation_heading"),
        (
            "Dieser XLSX-Export ist als jährliche Arbeitsunterlage für die österreichische "
            "E 1kv-Prüfung aufgebaut. Die Übersicht fasst die Beträge zusammen; die "
            "nummerierten Blätter enthalten die dazugehörigen Detailzeilen oder explizite "
            "Null-Platzhalter für noch nicht modellierte Bereiche.",
            "explanation_text",
        ),
        ("Prüfung", "explanation_heading"),
        (report["review_gate"], "explanation_text"),
        ("Aktuelle Kassiber-Annahmen", "explanation_heading"),
    ]
    rows.extend((f"{row['code']}: {row['message']}", "explanation_text") for row in report["assumptions"])
    rows.extend(
        [
            ("Datenqualität", "explanation_heading"),
            (
                f"Quarantäne-Gründe im Jahr: {len(report['data_quality']['quarantines'])}; "
                f"abweichende gespeicherte Kennzahlen: {len(report['data_quality']['kennzahl_mismatches'])}. "
                "Quarantinierte Transaktionen bleiben außerhalb dieser Arbeitsmappe, bis sie aufgelöst sind.",
                "explanation_text",
            ),
        ]
    )
    mismatches = _austrian_e1kv_mismatch_table_rows(report)
    if mismatches:
        rows.append(("Kennzahl-Abweichungen", "explanation_heading"))
        rows.append(("Transaktion | Kategorie | gespeichert | Export", "explanation_text"))
        rows.extend(
            (
                f"{tx_id} | {category} | {stored_kennzahl} | {export_kennzahl}",
                "explanation_text",
            )
            for tx_id, category, stored_kennzahl, export_kennzahl in mismatches
        )
    rows.extend(
        [
            ("Nicht modellierte Blätter", "explanation_heading"),
            (
                "Margin/Derivate/Futures, Steuergebühren, "
                "Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting "
                "werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür "
                "noch keine strukturierten Steuerereignisse speichert.",
                "explanation_text",
            ),
        ]
    )
    for row_index, (text, format_name) in enumerate(rows):
        if format_name.endswith("heading") or format_name.endswith("title"):
            worksheet.set_row(row_index, 26)
        elif text:
            worksheet.set_row(row_index, max(20, _estimate_wrapped_lines(text, 105) * 15 + 6))
        else:
            worksheet.set_row(row_index, 12)
        if text:
            worksheet.write_string(row_index, 0, text, formats[format_name])
        else:
            worksheet.write_blank(row_index, 0, None, formats[format_name])
    return worksheet


def _austrian_e1kv_xlsx_write_section_sheets(report, workbook, formats):
    for spec in _austrian_e1kv_section_table_specs(report):
        _austrian_e1kv_xlsx_write_detail_sheet(
            workbook,
            spec["sheet_name"],
            spec["title"],
            spec["headers"],
            spec["rows"],
            spec["row_format_names"],
            spec["total_rows"],
            spec["value_column"],
            formats,
            spec["column_widths"],
        )


def export_austrian_e1kv_xlsx_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, tax_year=None):
    import xlsxwriter

    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_properties(
        {
            "title": f"Kassiber Austrian E 1kv Report - {report['profile']} ({report['tax_year']})",
            "subject": "Austrian E 1kv cryptocurrency tax handoff",
            "author": "Kassiber",
            "comments": AUSTRIAN_E1KV_REVIEW_GATE,
        }
    )
    formats = _austrian_e1kv_xlsx_formats(workbook)
    _austrian_e1kv_xlsx_write_overview(report, workbook, formats)
    _austrian_e1kv_xlsx_write_section_sheets(report, workbook, formats)
    _austrian_e1kv_xlsx_write_explanations(report, workbook, formats)
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "form": report["form"],
        "tax_year": report["tax_year"],
        "sheets": list(AUSTRIAN_E1KV_XLSX_SHEETS),
        "rows": len(report["rows"]),
        "summary_rows": len(report["summary_rows"]),
    }


def _write_csv_rows(path, rows):
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerows(rows)


def _austrian_e1kv_overview_csv_rows(report):
    rows = [["I. Übersicht"]]
    for entry in _austrian_e1kv_overview_entries(report):
        if entry[0] == "heading":
            rows.extend([[], [entry[1]]])
        elif entry[0] == "section":
            rows.append([entry[1]])
        else:
            _kind, label, cents, _total = entry
            rows.append([label, _report_eur_cents(cents), "EUR"])
    return rows


def _austrian_e1kv_detail_csv_rows(spec):
    rows = [[spec["title"]], list(spec["headers"])]
    if spec["rows"]:
        rows.extend(
            _austrian_e1kv_render_table_row(row, spec["row_format_names"])
            for row in spec["rows"]
        )
    else:
        rows.append(["No rows in scope."])
    rows.append([])
    total_width = max(len(spec["headers"]), spec["value_column"] + 1)
    for label, value in spec["total_rows"]:
        total_row = [""] * total_width
        total_row[0] = label
        total_row[spec["value_column"]] = _austrian_e1kv_render_cell(value, "money")
        rows.append(total_row)
    return rows


def _austrian_e1kv_explanation_csv_rows(report):
    rows = [
        ["Erläuterungen zum Steuerreport"],
        [],
        ["Berichtsumfang"],
        [
            "Dieser CSV-Bundle-Export ist als jährliche Arbeitsunterlage für die "
            "österreichische E 1kv-Prüfung aufgebaut. Die Übersicht fasst die "
            "Beträge zusammen; die nummerierten Dateien enthalten Detailzeilen "
            "oder explizite Null-Platzhalter für noch nicht modellierte Bereiche."
        ],
        ["Prüfung"],
        [report["review_gate"]],
        ["Aktuelle Kassiber-Annahmen"],
    ]
    rows.extend([[f"{row['code']}: {row['message']}"] for row in report["assumptions"]])
    rows.extend(
        [
            ["Datenqualität"],
            [
                f"Quarantäne-Gründe im Jahr: {len(report['data_quality']['quarantines'])}; "
                f"abweichende gespeicherte Kennzahlen: {len(report['data_quality']['kennzahl_mismatches'])}. "
                "Quarantinierte Transaktionen bleiben außerhalb dieses Bundles, bis sie aufgelöst sind."
            ],
        ]
    )
    mismatches = _austrian_e1kv_mismatch_table_rows(report)
    if mismatches:
        rows.extend(
            [
                ["Kennzahl-Abweichungen"],
                ["Transaktion", "Kategorie", "Gespeicherte KZ", "Export KZ"],
                *mismatches,
            ]
        )
    rows.extend(
        [
            ["Nicht modellierte Dateien"],
            [
                "Margin/Derivate/Futures, Steuergebühren, "
                "Spenden/Schenkungen, verlorene Coins, kommerzielles Mining und Minting "
                "werden heute als leere Nullabschnitte dargestellt, weil Kassiber dafür "
                "noch keine strukturierten Steuerereignisse speichert."
            ],
        ]
    )
    return rows


def export_austrian_e1kv_csv_bundle(conn, workspace_ref, profile_ref, dir_path, hooks: ReportHooks, tax_year=None):
    report = report_austrian_e1kv(conn, workspace_ref, profile_ref, hooks, tax_year=tax_year)
    directory = Path(dir_path).expanduser()
    directory.mkdir(parents=True, exist_ok=True)

    files = []

    def write_file(sheet_name, filename, rows):
        path = directory / filename
        _write_csv_rows(path, rows)
        files.append(
            {
                "sheet": sheet_name,
                "file": str(path.resolve()),
                "bytes": path.stat().st_size,
                "rows": max(len(rows) - 1, 0),
            }
        )

    write_file(
        "Übersicht",
        _austrian_e1kv_csv_filename(0, "uebersicht"),
        _austrian_e1kv_overview_csv_rows(report),
    )
    for spec in _austrian_e1kv_section_table_specs(report):
        write_file(spec["sheet_name"], spec["filename"], _austrian_e1kv_detail_csv_rows(spec))
    write_file(
        "Erläuterungen zum Steuerreport",
        _austrian_e1kv_csv_filename(99, "erlaeuterungen_zum_steuerreport"),
        _austrian_e1kv_explanation_csv_rows(report),
    )
    return {
        "dir": str(directory.resolve()),
        "form": report["form"],
        "tax_year": report["tax_year"],
        "sheets": list(AUSTRIAN_E1KV_XLSX_SHEETS),
        "files": files,
        "rows": len(report["rows"]),
        "summary_rows": len(report["summary_rows"]),
    }


def _row_get(row, key, default=""):
    if isinstance(row, Mapping):
        return row.get(key, default)
    try:
        return row[key]
    except (IndexError, KeyError, TypeError):
        return default


def _overview_row(group, field, value):
    return {"group": group, "field": field, "value": "" if value is None else value}


def _generic_report_overview_rows(context):
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]
    holdings = rollups["holdings"]
    realized = rollups["realized"]

    return [
        _overview_row("Report metadata", "Generated at", context["generated_at"]),
        _overview_row("Report metadata", "Workspace", workspace["label"]),
        _overview_row("Report metadata", "Profile", profile["label"]),
        _overview_row("Report metadata", "Wallet scope", wallet["label"] if wallet else "All wallets"),
        _overview_row("Report metadata", "Fiat currency", profile["fiat_currency"]),
        _overview_row("Report metadata", "Tax country", profile["tax_country"]),
        _overview_row("Report metadata", "Tax long-term days", int(profile["tax_long_term_days"] or 0)),
        _overview_row("Report metadata", "Gains algorithm", profile["gains_algorithm"]),
        _overview_row("Report metadata", "Last processed at", profile["last_processed_at"] or ""),
        _overview_row("Report metadata", "Processed tx count", int(profile["last_processed_tx_count"] or 0)),
        _overview_row("Executive summary", "Wallets in scope", len(scope_wallets)),
        _overview_row("Executive summary", "Assets in scope", int(summary["asset_count"] or 0)),
        _overview_row("Executive summary", "Transactions active", int(summary["active_transactions"] or 0)),
        _overview_row("Executive summary", "Transactions excluded", int(summary["excluded_transactions"] or 0)),
        _overview_row("Executive summary", "Inbound transactions", int(summary["inbound_transactions"] or 0)),
        _overview_row("Executive summary", "Outbound transactions", int(summary["outbound_transactions"] or 0)),
        _overview_row("Executive summary", "Journal entries", int(query_rows["journal_entries"] or 0)),
        _overview_row("Executive summary", "Quarantines", int(query_rows["quarantines"] or 0)),
        _overview_row("Executive summary", "Priced transactions", int(summary["priced_transactions"] or 0)),
        _overview_row("Executive summary", "Transactions with notes", int(summary["noted_transactions"] or 0)),
        _overview_row("Executive summary", "Transactions with tags", int(query_rows["tagged_transactions"] or 0)),
        _overview_row("Executive summary", "First transaction", summary["first_transaction_at"] or ""),
        _overview_row("Executive summary", "Last transaction", summary["last_transaction_at"] or ""),
        _overview_row("Financial summary", "Holdings cost basis", holdings["cost_basis"]),
        _overview_row("Financial summary", "Holdings market value", holdings["market_value"]),
        _overview_row("Financial summary", "Unrealized PnL", holdings["unrealized_pnl"]),
        _overview_row("Financial summary", "Realized proceeds", realized["proceeds"]),
        _overview_row("Financial summary", "Realized cost basis", realized["cost_basis"]),
        _overview_row("Financial summary", "Realized gain/loss", realized["gain_loss"]),
    ]


def _generic_report_wallet_rows(context):
    return [
        {
            "wallet": _row_get(row, "label"),
            "kind": _row_get(row, "kind"),
            "chain": _row_get(row, "chain"),
            "network": _row_get(row, "network"),
            "backend": _row_get(row, "backend"),
            "gap_limit": _row_get(row, "gap_limit"),
        }
        for row in context["scope_wallets"]
    ]


def _generic_report_asset_flow_rows(context):
    return _summary_flow_rows(context["query_rows"]["flow_by_asset"])


def _generic_report_wallet_flow_rows(context):
    return _summary_wallet_flow_rows(context["query_rows"]["flow_by_wallet"])


def _pair_swap_fee_msat(row):
    """Return the persisted swap fee, or 0 when the pair stores NULL.

    Same-asset carrying-value transfers intentionally leave ``swap_fee_msat``
    NULL (they are not swaps). Inventing ``out_amount - in_amount`` would
    report network/amount deltas as swap fees and disagree with tax-summary,
    which only counts rows where ``swap_fee_msat IS NOT NULL``.
    """
    raw_fee_msat = row["swap_fee_msat"]
    if raw_fee_msat is None:
        return 0
    return int(raw_fee_msat or 0)


def _generic_report_transfer_pair_rows(context):
    rows = []
    for row in context["query_rows"]["transfer_pairs"]:
        out_amount_msat = int(row["out_amount"] or 0)
        in_amount_msat = int(row["in_amount"] or 0)
        out_fee_msat = int(row["out_fee"] or 0)
        in_fee_msat = int(row["in_fee"] or 0)
        out_asset = row["out_asset"]
        in_asset = row["in_asset"]
        swap_fee_msat = _pair_swap_fee_msat(row)
        rows.append(
            {
                "pair_id": row["id"],
                "pair_type": "transfer" if out_asset == in_asset else "swap",
                "kind": row["kind"],
                "policy": row["policy"],
                "swap_fee": float(msat_to_btc(swap_fee_msat)),
                "swap_fee_msat": swap_fee_msat,
                "swap_fee_kind": row["swap_fee_kind"],
                "out_occurred_at": row["out_occurred_at"],
                "out_wallet": row["out_wallet"],
                "out_transaction_id": row["out_transaction_id"],
                "out_asset": out_asset,
                "out_amount": float(msat_to_btc(out_amount_msat)),
                "out_amount_msat": out_amount_msat,
                "out_fee": float(msat_to_btc(out_fee_msat)),
                "out_fee_msat": out_fee_msat,
                "in_occurred_at": row["in_occurred_at"],
                "in_wallet": row["in_wallet"],
                "in_transaction_id": row["in_transaction_id"],
                "in_asset": in_asset,
                "in_amount": float(msat_to_btc(in_amount_msat)),
                "in_amount_msat": in_amount_msat,
                "in_fee": float(msat_to_btc(in_fee_msat)),
                "in_fee_msat": in_fee_msat,
                "notes": row["notes"],
                "created_at": row["created_at"],
            }
        )
    for row in context["query_rows"].get("direct_swap_payouts", []):
        out_amount_msat = int(row["out_amount"] or 0)
        payout_amount_msat = int(row["payout_amount"] or 0)
        out_fee_msat = int(row["out_fee"] or 0)
        swap_fee_msat = int(row["swap_fee_msat"] or 0)
        rows.append(
            {
                "pair_id": row["id"],
                "pair_type": "direct_swap_payout",
                "kind": row["kind"],
                "policy": row["policy"],
                "swap_fee": float(msat_to_btc(swap_fee_msat)),
                "swap_fee_msat": swap_fee_msat,
                "swap_fee_kind": row["swap_fee_kind"],
                "out_occurred_at": row["out_occurred_at"],
                "out_wallet": row["out_wallet"],
                "out_transaction_id": row["out_transaction_id"],
                "out_asset": row["out_asset"],
                "out_amount": float(msat_to_btc(out_amount_msat)),
                "out_amount_msat": out_amount_msat,
                "out_fee": float(msat_to_btc(out_fee_msat)),
                "out_fee_msat": out_fee_msat,
                "in_occurred_at": row["payout_occurred_at"] or row["out_occurred_at"],
                "in_wallet": row["counterparty"] or "external",
                "in_transaction_id": row["payout_external_id"] or "",
                "in_asset": row["payout_asset"],
                "in_amount": float(msat_to_btc(payout_amount_msat)),
                "in_amount_msat": payout_amount_msat,
                "in_fee": 0.0,
                "in_fee_msat": 0,
                "notes": row["notes"],
                "created_at": row["created_at"],
            }
        )
    return rows


def _generic_report_balance_rows(context):
    rows = []
    for row in context["balance_rows"]:
        quantity_msat = btc_to_msat(row["quantity"])
        rows.append(
            {
                "account": row["account"],
                "asset": row["asset"],
                "quantity": row["quantity"],
                "quantity_sat": _msat_to_sat(quantity_msat),
                "quantity_msat": quantity_msat,
                "cost_basis": row["cost_basis"],
                "market_value": row["market_value"],
                "unrealized_pnl": row["unrealized_pnl"],
            }
        )
    return rows


def _generic_report_portfolio_rows(context):
    return [dict(row) for row in context["portfolio_rows"]]


def _generic_report_capital_summary_rows(context):
    grouped = {}
    for row in context["capital_rows"]:
        key = (row["wallet"], row["asset"])
        bucket = grouped.setdefault(
            key,
            {
                "wallet": row["wallet"],
                "asset": row["asset"],
                "rows": 0,
                "proceeds": 0.0,
                "cost_basis": 0.0,
                "gain_loss": 0.0,
            },
        )
        bucket["rows"] += 1
        bucket["proceeds"] += float(row["proceeds"])
        bucket["cost_basis"] += float(row["cost_basis"])
        bucket["gain_loss"] += float(row["gain_loss"])
    return [grouped[key] for key in sorted(grouped)]


def _generic_report_capital_detail_rows(context):
    rows = []
    for row in context["capital_rows"]:
        rows.append(
            {
                "occurred_at": row["occurred_at"],
                "wallet": row["wallet"],
                "transaction_id": row["transaction_id"],
                "entry_type": row["entry_type"],
                "asset": row["asset"],
                "quantity": row["quantity"],
                "quantity_msat": row["quantity_msat"],
                "proceeds": row["proceeds"],
                "cost_basis": row["cost_basis"],
                "gain_loss": row["gain_loss"],
                "description": row["description"],
                "at_category": row.get("at_category", ""),
                "at_kennzahl": row.get("at_kennzahl", ""),
            }
        )
    return rows


def _generic_report_balance_history_rows(context):
    return [dict(row) for row in context["history_rows"]]


def _generic_report_data_quality_rows(context):
    return [
        {"reason": row["reason"], "count": int(row["count"] or 0)}
        for row in context["query_rows"]["quarantine_rows"]
    ]


def _optional_decimal(value):
    """Return ``Decimal(value)`` or ``None`` when the value is absent, so a real
    ``0`` price/value stays distinct from a missing one."""
    if value is None or value == "":
        return None
    return dec(value)


def _optional_decimal_pref(row, exact_key, legacy_key):
    """Prefer the exact decimal string over the lossy float column, mirroring the
    transaction editor's canonical view so the tax ledger shows the precise
    recorded price/value users are meant to verify."""
    value = _optional_decimal(row.get(exact_key))
    if value is None:
        value = _optional_decimal(row.get(legacy_key))
    return value


def _generic_report_transaction_rows(context):
    rows = []
    for row in context["query_rows"]["transactions"]:
        amount_msat = int(row["amount"] or 0)
        fee_msat = int(row["fee"] or 0)
        amount_btc = msat_to_btc(amount_msat)
        fee_btc = msat_to_btc(fee_msat)
        # `fiat_price` is the recorded execution/market price, `fiat_value` the
        # recorded cash leg. Exchange imports fold fiat service fees into
        # `fiat_value` (e.g. Coinfinity/Strike buy = cash paid incl. fee), so
        # FMV — the market value of the BTC amount — is amount * price when a
        # price is recorded, with the fee-adjusted cash value kept in its own
        # column. `fiat_fee` prices the crypto-denominated network fee; a fiat
        # service fee shows as the fiat_value − fmv difference.
        price = _optional_decimal_pref(row, "fiat_rate_exact", "fiat_rate")
        value = _optional_decimal_pref(row, "fiat_value_exact", "fiat_value")
        if price is None and value is not None:
            if amount_btc:
                price = value / amount_btc
            elif fee_btc:
                # Fee-only rows (zero amount, crypto fee, recorded value): the
                # recorded value prices the fee, mirroring the tax normalizer,
                # so FMV correctly reads 0 and fiat_fee carries the value.
                price = value / fee_btc
        fmv = amount_btc * price if price is not None else value
        fiat_fee = fee_btc * price if price is not None else None
        rows.append(
            {
                "occurred_at": row["occurred_at"],
                "wallet": row["wallet"],
                "transaction_id": row["transaction_id"],
                "direction": row["direction"],
                "asset": row["asset"],
                "amount": float(amount_btc),
                "amount_msat": amount_msat,
                "fee": float(fee_btc),
                "fee_msat": fee_msat,
                "fiat_currency": row.get("fiat_currency") or "",
                "fiat_price": float(price) if price is not None else None,
                "fiat_value": float(value) if value is not None else None,
                "fmv": float(fmv) if fmv is not None else None,
                "fiat_fee": float(fiat_fee) if fiat_fee is not None else None,
                "cost_basis": row.get("cost_basis"),
                "gain_loss": row.get("gain_loss"),
                "description": row["description"],
                "note": row.get("note", ""),
                "counterparty": row.get("counterparty", ""),
                "transfer": row.get("transfer", ""),
                "tags": row.get("tags", ""),
                "references": row.get("references", ""),
                "attachments": row.get("attachments", ""),
                "attachments_list": row.get("attachments_list", []),
            }
        )
    return rows


def _generic_report_section_specs(context):
    return [
        {
            "sheet_name": "Overview",
            "title": "Overview",
            "headers": ("group", "field", "value"),
            "rows": _generic_report_overview_rows(context),
        },
        {
            "sheet_name": "Wallets",
            "title": "Wallet Inventory",
            "headers": ("wallet", "kind", "chain", "network", "backend", "gap_limit"),
            "rows": _generic_report_wallet_rows(context),
        },
        {
            "sheet_name": "Asset Flow",
            "title": "Asset Flow Summary",
            "headers": (
                "asset",
                "tx_count",
                "inbound_count",
                "outbound_count",
                "inbound_amount",
                "inbound_amount_sat",
                "inbound_amount_msat",
                "outbound_amount",
                "outbound_amount_sat",
                "outbound_amount_msat",
                "fee_amount",
                "fee_amount_sat",
                "fee_amount_msat",
            ),
            "rows": _generic_report_asset_flow_rows(context),
        },
        {
            "sheet_name": "Wallet Metrics",
            "title": "Wallet Transaction Metrics",
            "headers": (
                "wallet",
                "asset",
                "tx_count",
                "inbound_count",
                "outbound_count",
                "inbound_amount",
                "inbound_amount_sat",
                "inbound_amount_msat",
                "outbound_amount",
                "outbound_amount_sat",
                "outbound_amount_msat",
                "fee_amount",
                "fee_amount_sat",
                "fee_amount_msat",
                "first_transaction_at",
                "last_transaction_at",
            ),
            "rows": _generic_report_wallet_flow_rows(context),
        },
        {
            "sheet_name": "Transfers & Swaps",
            "title": "Reviewed Transfers and Swaps",
            "headers": (
                "pair_id",
                "pair_type",
                "kind",
                "policy",
                "swap_fee",
                "swap_fee_msat",
                "swap_fee_kind",
                "out_occurred_at",
                "out_wallet",
                "out_transaction_id",
                "out_asset",
                "out_amount",
                "out_amount_msat",
                "out_fee",
                "out_fee_msat",
                "in_occurred_at",
                "in_wallet",
                "in_transaction_id",
                "in_asset",
                "in_amount",
                "in_amount_msat",
                "in_fee",
                "in_fee_msat",
                "notes",
                "created_at",
            ),
            "rows": _generic_report_transfer_pair_rows(context),
        },
        {
            "sheet_name": "Balance Sheet",
            "title": "Balance Sheet",
            "headers": (
                "account",
                "asset",
                "quantity",
                "quantity_sat",
                "quantity_msat",
                "cost_basis",
                "market_value",
                "unrealized_pnl",
            ),
            "rows": _generic_report_balance_rows(context),
        },
        {
            "sheet_name": "Portfolio",
            "title": "Portfolio Summary",
            "headers": (
                "wallet",
                "account",
                "asset",
                "quantity",
                "quantity_sat",
                "quantity_msat",
                "avg_cost",
                "cost_basis",
                "market_value",
                "unrealized_pnl",
            ),
            "rows": _generic_report_portfolio_rows(context),
        },
        {
            "sheet_name": "Capital Summary",
            "title": "Capital Gains Summary",
            "headers": ("wallet", "asset", "rows", "proceeds", "cost_basis", "gain_loss"),
            "rows": _generic_report_capital_summary_rows(context),
        },
        {
            "sheet_name": "Capital Detail",
            "title": "Capital Gains Detail",
            "headers": (
                "occurred_at",
                "wallet",
                "transaction_id",
                "entry_type",
                "asset",
                "quantity",
                "quantity_msat",
                "proceeds",
                "cost_basis",
                "gain_loss",
                "description",
                "at_category",
                "at_kennzahl",
            ),
            "rows": _generic_report_capital_detail_rows(context),
        },
        {
            "sheet_name": "Balance History",
            "title": "Balance History",
            "headers": ("period_start", "period_end", "asset", "quantity", "cumulative_cost_basis", "market_value"),
            "rows": _generic_report_balance_history_rows(context),
        },
        {
            "sheet_name": "Data Quality",
            "title": "Data Quality",
            "headers": ("reason", "count"),
            "rows": _generic_report_data_quality_rows(context),
        },
        {
            "sheet_name": "Transactions",
            "title": "Transactions",
            "headers": TRANSACTIONS_EXPORT_HEADERS,
            "rows": _generic_report_transaction_rows(context),
        },
    ]


def _report_column_label(header):
    return (
        header.replace("_", " ")
        .title()
        .replace(" Id", " ID")
        .replace(" Msat", " msat")
        .replace(" Sat", " sat")
        .replace(" Pnl", " PnL")
        .replace("Fmv", "FMV")
    )


def _generic_report_csv_rows(context, sections):
    rows = [[context["title"]], []]
    for spec in sections:
        headers = list(spec["headers"])
        rows.append([spec["title"]])
        rows.append([_report_column_label(header) for header in headers])
        if spec["rows"]:
            rows.extend([[row.get(header, "") for header in headers] for row in spec["rows"]])
        else:
            rows.append(["No rows in scope."])
        rows.append([])
    return rows


def _generic_report_xlsx_formats(workbook):
    return {
        "title": workbook.add_format({"bold": True, "font_size": 14, "valign": "vcenter"}),
        "header": workbook.add_format(
            {"bold": True, "font_size": 11, "valign": "top", "text_wrap": True, "bg_color": "#EFEFEF", "bottom": 1}
        ),
        "text": workbook.add_format({"font_size": 11, "valign": "top", "text_wrap": True}),
        "int": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0"}),
        "quantity": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "0.00000000"}),
        "money": workbook.add_format({"font_size": 11, "valign": "top", "num_format": "#,##0.00"}),
        "link": workbook.add_format(
            {"font_size": 11, "valign": "top", "text_wrap": True, "font_color": "#185FA5", "underline": 1}
        ),
    }


GENERIC_REPORT_QUANTITY_FIELDS = {
    "amount",
    "fee",
    "fee_amount",
    "in_amount",
    "in_fee",
    "inbound_amount",
    "out_amount",
    "out_fee",
    "outbound_amount",
    "quantity",
}
GENERIC_REPORT_INT_FIELDS = {
    "amount_msat",
    "count",
    "fee_msat",
    "fee_amount_msat",
    "gap_limit",
    "in_amount_msat",
    "in_fee_msat",
    "inbound_amount_msat",
    "inbound_count",
    "out_amount_msat",
    "out_fee_msat",
    "outbound_amount_msat",
    "outbound_count",
    "quantity_msat",
    "rows",
    "tx_count",
}
GENERIC_REPORT_MONEY_FIELDS = {
    "avg_cost",
    "cost_basis",
    "cumulative_cost_basis",
    "fiat_fee",
    "fiat_price",
    "fiat_value",
    "fmv",
    "gain_loss",
    "market_value",
    "proceeds",
    "unrealized_pnl",
}


def _generic_report_xlsx_format_name(header, value):
    if value in (None, ""):
        return "text"
    if header in GENERIC_REPORT_INT_FIELDS:
        return "int"
    if header in GENERIC_REPORT_MONEY_FIELDS:
        return "money"
    if header in GENERIC_REPORT_QUANTITY_FIELDS or header.endswith("_sat"):
        return "quantity"
    return "text"


def _generic_report_column_width(header, rows):
    label_width = len(_report_column_label(header)) + 2
    sample_width = label_width
    for row in rows[:50]:
        value = row.get(header, "")
        sample_width = max(sample_width, min(len(str(value)), 36))
    return max(10, min(sample_width + 2, 38))


def _estimate_wrapped_lines(text, width_chars):
    """Estimate how many wrapped lines a string occupies in a cell that wide."""
    if text in (None, ""):
        return 1
    width = max(1, int(width_chars) - 1)
    total = 0
    for segment in str(text).split("\n"):
        total += max(1, -(-len(segment.rstrip()) // width))
    return max(1, total)


def _row_height_for_lines(lines):
    return max(16, lines * 15 + 4)


def _generic_report_xlsx_write_sheet(workbook, spec, formats):
    worksheet = workbook.add_worksheet(spec["sheet_name"])
    worksheet.set_landscape()
    worksheet.fit_to_pages(1, 0)
    worksheet.set_margins(left=0.35, right=0.35, top=0.5, bottom=0.5)
    headers = list(spec["headers"])
    rows = spec["rows"]
    last_column = max(len(headers) - 1, 0)
    widths = [_generic_report_column_width(header, rows) for header in headers]
    worksheet.set_row(0, 28)
    worksheet.merge_range(0, 0, 0, last_column, spec["title"], formats["title"])
    header_lines = max(
        (_estimate_wrapped_lines(_report_column_label(header), widths[index]) for index, header in enumerate(headers)),
        default=1,
    )
    worksheet.set_row(1, max(24, header_lines * 15 + 4))
    for column_index, header in enumerate(headers):
        worksheet.set_column(column_index, column_index, widths[column_index])
        worksheet.write_string(1, column_index, _report_column_label(header), formats["header"])
    worksheet.freeze_panes(2, 0)

    row_index = 2
    if rows:
        for row in rows:
            lines = 1
            for column_index, header in enumerate(headers):
                if header == "attachments" and "attachments_list" in row:
                    _write_attachments_cell(worksheet, row_index, column_index, row, formats)
                    lines = max(lines, len(row.get("attachments_list") or []) or 1)
                    continue
                value = row.get(header, "")
                format_name = _generic_report_xlsx_format_name(header, value)
                if format_name == "text":
                    lines = max(lines, _estimate_wrapped_lines(value, widths[column_index]))
                _xlsx_write_value(worksheet, row_index, column_index, value, formats[format_name])
            worksheet.set_row(row_index, _row_height_for_lines(lines))
            row_index += 1
    else:
        worksheet.write_string(row_index, 0, "No rows in scope.", formats["text"])
        row_index += 1
    worksheet.autofilter(1, 0, max(row_index - 1, 1), last_column)
    return worksheet


def _write_attachments_cell(worksheet, row_index, column_index, row, formats):
    """Render the attachments cell: a single URL becomes a clickable link shown
    behind its name; multiple attachments are listed one per line (Excel allows
    only one hyperlink per cell, so the per-link clickables live on Evidence)."""
    entries = row.get("attachments_list") or []
    if not entries:
        worksheet.write_blank(row_index, column_index, None, formats["text"])
        return
    url_entries = [entry for entry in entries if entry.get("url")]
    if len(entries) == 1 and url_entries:
        entry = url_entries[0]
        worksheet.write_url(row_index, column_index, entry["url"], formats["link"], entry["display_name"])
        return
    worksheet.write_string(
        row_index, column_index, "\n".join(entry["display_name"] for entry in entries), formats["text"]
    )


def export_csv_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    context = _build_full_report_context(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        wallet_ref=wallet_ref,
        history_limit=history_limit,
    )
    sections = _generic_report_section_specs(context)
    rows = _generic_report_csv_rows(context, sections)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    _write_csv_rows(path, rows)
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "title": context["title"],
        "wallet": wallet_ref or "",
        "sections": [spec["sheet_name"] for spec in sections],
        "rows": sum(len(spec["rows"]) for spec in sections),
    }


_VERIFY_ADD_ENTRY_TYPES = ("acquisition", "income", "transfer_in")
_VERIFY_SUB_ENTRY_TYPES = ("disposal", "fee", "transfer_fee", "transfer_out")


def _verification_quarantine_detail(detail_json):
    """Render a quarantine's detail_json as a compact one-line string."""
    if not detail_json:
        return ""
    import json

    try:
        data = json.loads(detail_json)
    except (ValueError, TypeError):
        return str(detail_json)[:200]
    if isinstance(data, dict):
        if not data:
            return ""
        return "; ".join(f"{key}={value}" for key, value in data.items())[:200]
    return str(data)[:200]


def _build_verification_data(conn, profile, hooks: ReportHooks):
    """Gather the profile-scope ledger and per-asset Kassiber aggregates that
    drive the self-verifying ``Control`` / ``Acquisitions`` / ``Disposals``
    sheets. Reconciliation is per asset across the whole profile (Bitcoin
    accounting is pooled per asset across wallets), so the wallet filter on the
    rest of the report does not apply here."""
    workspace_id = profile["workspace_id"]
    profile_id = profile["id"]

    ledger_rows = conn.execute(
        """
        SELECT
            je.occurred_at,
            w.label AS wallet,
            je.transaction_id AS internal_tx_id,
            COALESCE(NULLIF(t.external_id, ''), je.transaction_id) AS transaction_id,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            COALESCE(je.pricing_source_kind, '') AS pricing_source,
            COALESCE(je.pricing_quality, '') AS pricing_quality,
            CASE
                WHEN COALESCE(t.taxability_override, 1) != 0
                     AND COALESCE(je.at_category, '') != 'neu_swap'
                THEN 1 ELSE 0
            END AS taxable
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ?
        ORDER BY je.occurred_at ASC, je.created_at ASC, je.id ASC
        """,
        (profile_id,),
    ).fetchall()

    ledger_tags = _tags_by_transaction(conn, "t.profile_id = ?", [profile_id])

    acquisitions = []
    disposals = []
    for row in ledger_rows:
        magnitude = abs(int(row["quantity"]))
        common = {
            "occurred_at": row["occurred_at"],
            "wallet": row["wallet"],
            "transaction_id": row["transaction_id"],
            "asset": row["asset"],
            "entry_type": row["entry_type"],
            "quantity_msat": magnitude,
            "quantity": float(msat_to_btc(magnitude)),
            "description": row["description"],
            "tags": ledger_tags.get(row["internal_tx_id"], ""),
            "pricing_source": row["pricing_source"],
            "pricing_quality": row["pricing_quality"],
            "taxable": int(row["taxable"]),
        }
        if row["entry_type"] in _VERIFY_ADD_ENTRY_TYPES:
            common["fiat_value"] = float(row["fiat_value"] or 0.0)
            common["gain_loss"] = float(row["gain_loss"] or 0.0)
            acquisitions.append(common)
        elif row["entry_type"] in _VERIFY_SUB_ENTRY_TYPES:
            common["proceeds"] = float(row["proceeds"] or 0.0)
            common["cost_basis"] = float(row["cost_basis"] or 0.0)
            common["gain_loss"] = float(row["gain_loss"] or 0.0)
            common["gain_loss_kassiber"] = float(row["gain_loss"] or 0.0)
            disposals.append(common)

    quarantines = conn.execute(
        """
        SELECT
            jq.reason,
            jq.detail_json,
            COALESCE(NULLIF(t.external_id, ''), jq.transaction_id) AS transaction_id,
            t.occurred_at,
            t.asset,
            t.amount,
            COALESCE(t.description, '') AS description
        FROM journal_quarantines jq
        LEFT JOIN transactions t ON t.id = jq.transaction_id
        WHERE jq.profile_id = ?
        ORDER BY t.occurred_at ASC, jq.transaction_id ASC
        """,
        (profile_id,),
    ).fetchall()
    quarantine_rows = []
    for row in quarantines:
        amount_msat = row["amount"]
        quarantine_rows.append(
            {
                "occurred_at": row["occurred_at"] or "",
                "transaction_id": row["transaction_id"],
                "asset": row["asset"] or "",
                "amount": float(msat_to_btc(amount_msat)) if amount_msat is not None else "",
                "reason": row["reason"],
                "detail": _verification_quarantine_detail(row["detail_json"]),
                "description": row["description"],
            }
        )

    evidence = conn.execute(
        """
        SELECT
            t.occurred_at,
            w.label AS wallet,
            COALESCE(t.external_id, '') AS transaction_id,
            t.asset,
            a.attachment_type AS kind,
            COALESCE(a.label, '') AS label,
            COALESCE(a.original_filename, '') AS filename,
            COALESCE(a.source_url, '') AS url
        FROM attachments a
        JOIN transactions t ON t.id = a.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE a.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, a.created_at ASC
        """,
        (profile_id,),
    ).fetchall()
    attachments = []
    for row in evidence:
        entry = _attachment_entry(row)
        attachments.append(
            {
                "occurred_at": row["occurred_at"] or "",
                "wallet": row["wallet"],
                "transaction_id": row["transaction_id"],
                "asset": row["asset"] or "",
                "type": entry["kind"],
                "name": entry["display_name"],
                "url": entry["url"],
                "reference": entry["reference"],
            }
        )

    portfolio_rows = report_portfolio_summary(conn, workspace_id, profile_id, hooks)
    capital_rows = report_capital_gains(conn, workspace_id, profile_id, hooks)

    fiat_currency = profile["fiat_currency"]
    rate_provenance = {}
    for asset in {row["asset"] for row in portfolio_rows}:
        pair = core_rates.transaction_rate_pair(asset, fiat_currency)
        source, timestamp = "transaction price (no market quote)", ""
        if pair is not None:
            try:
                cached_rate = core_rates.get_latest_rate(conn, pair)
                source = cached_rate.get("source") or source
                timestamp = cached_rate.get("timestamp") or ""
            except AppError as exc:
                if exc.code != "not_found":
                    raise
            except sqlite3.OperationalError:
                # Rates cache table may not exist yet (older DBs); fall back to
                # the "no market quote" provenance defaults set above.
                pass
        rate_provenance[asset] = (source, timestamp)

    holdings_by_asset = {}
    for row in portfolio_rows:
        bucket = holdings_by_asset.setdefault(
            row["asset"],
            {"quantity": 0.0, "cost_basis": 0.0, "market_value": 0.0, "unrealized_pnl": 0.0},
        )
        bucket["quantity"] += float(row["quantity"])
        bucket["cost_basis"] += float(row["cost_basis"])
        bucket["market_value"] += float(row["market_value"])
        bucket["unrealized_pnl"] += float(row["unrealized_pnl"])

    realized_by_asset = {}
    for row in capital_rows:
        realized_by_asset[row["asset"]] = realized_by_asset.get(row["asset"], 0.0) + float(row["gain_loss"])

    asset_rows = []
    for asset in sorted(set(holdings_by_asset) | set(realized_by_asset)):
        holding = holdings_by_asset.get(
            asset, {"quantity": 0.0, "cost_basis": 0.0, "market_value": 0.0, "unrealized_pnl": 0.0}
        )
        quantity = holding["quantity"]
        cost_basis = holding["cost_basis"]
        market_value = holding["market_value"]
        rate = market_value / quantity if quantity else 0.0
        avg_cost = cost_basis / quantity if quantity else 0.0
        realized = realized_by_asset.get(asset, 0.0)
        rate_source, rate_timestamp = rate_provenance.get(asset, ("", ""))
        asset_rows.append(
            {
                "asset": asset,
                "rate": rate,
                "rate_source": rate_source,
                "rate_as_of": rate_timestamp,
                "quantity": quantity,
                "holdings_qty_kassiber": quantity,
                "cost_basis": cost_basis,
                "cost_basis_kassiber": cost_basis,
                "avg_cost_kassiber": avg_cost,
                "market_value": market_value,
                "market_value_kassiber": market_value,
                "unrealized_pnl": holding["unrealized_pnl"],
                "unrealized_kassiber": holding["unrealized_pnl"],
                "realized_gain": realized,
                "realized_gain_kassiber": realized,
            }
        )

    return {
        "acquisitions": acquisitions,
        "disposals": disposals,
        "asset_rows": asset_rows,
        "quarantines": quarantine_rows,
        "attachments": attachments,
    }


def export_xlsx_report(
    conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None, history_limit=None, verify=True
):
    import xlsxwriter

    from . import report_verify

    context = _build_full_report_context(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        wallet_ref=wallet_ref,
        history_limit=history_limit,
    )
    sections = _generic_report_section_specs(context)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)

    workbook = xlsxwriter.Workbook(str(path))
    if verify:
        # Verification-sheet formulas should recompute when the file is opened.
        workbook.set_calc_mode("auto")
    workbook.set_properties(
        {
            "title": context["title"],
            "subject": "Kassiber full report export",
            "author": "Kassiber",
            "comments": "Generated from the same processed report context as the generic PDF export.",
        }
    )
    formats = _generic_report_xlsx_formats(workbook)
    for spec in sections:
        _generic_report_xlsx_write_sheet(workbook, spec, formats)

    verify_sheets = []
    if verify:
        import kassiber

        profile = context["profile"]
        verification = _build_verification_data(conn, profile, hooks)
        run_metadata = {
            "generated_at": context["generated_at"],
            "kassiber_version": getattr(kassiber, "__version__", ""),
            "lot_method": profile["gains_algorithm"],
            "fiat_currency": profile["fiat_currency"],
            "tax_country": profile["tax_country"],
            "last_processed_at": profile["last_processed_at"] or "",
            "processed_tx_count": int(profile["last_processed_tx_count"] or 0),
            "wallet_scope": context["wallet"]["label"] if context["wallet"] else "All wallets",
        }
        verify_sheets = report_verify.augment_workbook(
            workbook,
            gains_algorithm=profile["gains_algorithm"],
            tax_country=profile["tax_country"],
            fiat_currency=profile["fiat_currency"],
            wallet_scope_label=context["wallet"]["label"] if context["wallet"] else None,
            run_metadata=run_metadata,
            acquisitions=verification["acquisitions"],
            disposals=verification["disposals"],
            asset_rows=verification["asset_rows"],
            quarantines=verification["quarantines"],
            attachments=verification["attachments"],
        )
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "title": context["title"],
        "wallet": wallet_ref or "",
        "sheets": [spec["sheet_name"] for spec in sections] + verify_sheets,
        "verified": bool(verify),
        "rows": sum(len(spec["rows"]) for spec in sections),
    }


TRANSACTIONS_EXPORT_HEADERS = (
    "occurred_at",
    "wallet",
    "transaction_id",
    "direction",
    "asset",
    "amount",
    "amount_msat",
    "fee",
    "fee_msat",
    "fiat_currency",
    "fiat_price",
    "fiat_value",
    "fmv",
    "fiat_fee",
    "cost_basis",
    "gain_loss",
    "description",
    "note",
    "counterparty",
    "transfer",
    "tags",
    "references",
    "attachments",
)


def _transactions_export_context(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None):
    """Build the single-sheet Transactions export spec (wallet-scope aware).

    Reuses the same row builder + columns as the full report's Transactions
    sheet — including note, counterparty, tags, and the linked-file/URL
    attachments — so the standalone export matches the report."""
    workspace, profile = _resolve_report_scope(conn, workspace_ref, profile_ref, hooks)
    wallet = hooks.resolve_wallet(conn, profile["id"], wallet_ref) if wallet_ref else None
    query_rows = _report_query_rows(conn, profile, wallet=wallet)
    rows = _generic_report_transaction_rows({"query_rows": query_rows})
    title_scope = wallet["label"] if wallet else profile["label"]
    spec = {
        "sheet_name": "Transactions",
        "title": "Transactions",
        "headers": TRANSACTIONS_EXPORT_HEADERS,
        "rows": rows,
    }
    return {"title": f"Kassiber Transactions - {title_scope}", "spec": spec, "wallet": wallet}


def export_transactions_csv_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None):
    context = _transactions_export_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    spec = context["spec"]
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    _write_csv_rows(path, _generic_report_csv_rows({"title": context["title"]}, [spec]))
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "title": context["title"],
        "wallet": wallet_ref or "",
        "rows": len(spec["rows"]),
    }


def export_transactions_xlsx_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None):
    import xlsxwriter

    context = _transactions_export_context(conn, workspace_ref, profile_ref, hooks, wallet_ref=wallet_ref)
    spec = context["spec"]
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_properties(
        {
            "title": context["title"],
            "subject": "Kassiber transactions export",
            "author": "Kassiber",
            "comments": "Transaction ledger with notes, tags, and linked evidence.",
        }
    )
    formats = _generic_report_xlsx_formats(workbook)
    _generic_report_xlsx_write_sheet(workbook, spec, formats)
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "title": context["title"],
        "wallet": wallet_ref or "",
        "sheets": [spec["sheet_name"]],
        "rows": len(spec["rows"]),
    }


def build_pdf_report_lines(conn, workspace_ref, profile_ref, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    context = _build_full_report_context(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        wallet_ref=wallet_ref,
        history_limit=history_limit,
    )
    workspace = context["workspace"]
    profile = context["profile"]
    wallet = context["wallet"]
    scope_wallets = context["scope_wallets"]
    portfolio_rows = context["portfolio_rows"]
    balance_rows = context["balance_rows"]
    capital_rows = context["capital_rows"]
    history_rows = context["history_rows"]
    query_rows = context["query_rows"]
    summary = context["summary"]
    rollups = context["rollups"]
    holdings_cost_basis = rollups["holdings"]["cost_basis"]
    holdings_market_value = rollups["holdings"]["market_value"]
    holdings_unrealized = rollups["holdings"]["unrealized_pnl"]
    realized_proceeds = rollups["realized"]["proceeds"]
    realized_cost_basis = rollups["realized"]["cost_basis"]
    realized_gain_loss = rollups["realized"]["gain_loss"]

    title = context["pdf_title"]

    lines = [title, "=" * len(title), ""]
    lines.extend(
        _report_kv_lines(
            [
                ("Generated at", context["generated_at"]),
                ("Workspace", workspace["label"]),
                ("Profile", profile["label"]),
                ("Wallet scope", wallet["label"] if wallet else "All wallets"),
                ("Fiat currency", profile["fiat_currency"]),
                ("Tax country", profile["tax_country"]),
                ("Tax long-term days", profile["tax_long_term_days"]),
                ("Gains algorithm", profile["gains_algorithm"]),
                ("Last processed at", profile["last_processed_at"] or ""),
                ("Processed tx count", _report_count(profile["last_processed_tx_count"])),
            ]
        )
    )

    lines.extend(["", "Executive Summary", "-----------------"])
    lines.extend(
        _report_kv_lines(
            [
                ("Wallets in scope", _report_count(len(scope_wallets))),
                ("Assets in scope", _report_count(summary["asset_count"])),
                ("Transactions (active)", _report_count(summary["active_transactions"])),
                ("Transactions (excluded)", _report_count(summary["excluded_transactions"])),
                ("Inbound transactions", _report_count(summary["inbound_transactions"])),
                ("Outbound transactions", _report_count(summary["outbound_transactions"])),
                ("Journal entries", _report_count(query_rows["journal_entries"])),
                ("Quarantines", _report_count(query_rows["quarantines"])),
                ("Priced transactions", _report_count(summary["priced_transactions"])),
                ("Transactions with notes", _report_count(summary["noted_transactions"])),
                ("Transactions with tags", _report_count(query_rows["tagged_transactions"])),
                ("First transaction", summary["first_transaction_at"] or ""),
                ("Last transaction", summary["last_transaction_at"] or ""),
                ("Holdings cost basis", _report_fiat(holdings_cost_basis)),
                ("Holdings market value", _report_fiat(holdings_market_value)),
                ("Unrealized PnL", _report_fiat(holdings_unrealized)),
                ("Realized proceeds", _report_fiat(realized_proceeds)),
                ("Realized cost basis", _report_fiat(realized_cost_basis)),
                ("Realized gain/loss", _report_fiat(realized_gain_loss)),
            ]
        )
    )

    lines.extend(["", "Wallet Inventory", "----------------"])
    wallet_table_rows = [
        [
            row["label"],
            row["kind"],
            row["chain"],
            row["network"],
            row["backend"],
            row["gap_limit"],
        ]
        for row in scope_wallets
    ]
    if wallet_table_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Kind", "Chain", "Network", "Backend", "Gap"],
                wallet_table_rows,
                [18, 12, 8, 10, 12, 5],
            )
        )
    else:
        lines.append("No wallets in scope.")

    lines.extend(["", "Asset Flow Summary", "------------------"])
    asset_flow_rows = [
        [
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_asset"]
    ]
    if asset_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                asset_flow_rows,
                [6, 6, 6, 6, 14, 14, 14],
                align_right={1, 2, 3, 4, 5, 6},
            )
        )
    else:
        lines.append("No active transactions in scope.")

    lines.extend(["", "Wallet Transaction Metrics", "--------------------------"])
    wallet_flow_rows = [
        [
            row["wallet"],
            row["asset"],
            _report_count(row["tx_count"]),
            _report_count(row["inbound_count"]),
            _report_count(row["outbound_count"]),
            _report_btc(msat_to_btc(row["inbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["outbound_amount"] or 0)),
            _report_btc(msat_to_btc(row["fee_amount"] or 0)),
        ]
        for row in query_rows["flow_by_wallet"]
    ]
    if wallet_flow_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Asset", "Tx", "In", "Out", "Inbound", "Outbound", "Fees"],
                wallet_flow_rows,
                [18, 6, 6, 6, 6, 14, 14, 14],
                align_right={2, 3, 4, 5, 6, 7},
            )
        )
    else:
        lines.append("No wallet transaction metrics available.")

    lines.extend(["", "Balance Sheet", "-------------"])
    balance_table_rows = [
        [
            row["account"],
            row["asset"],
            _report_btc(row["quantity"]),
            _report_fiat(row["cost_basis"]),
            _report_fiat(row["market_value"]),
            _report_fiat(row["unrealized_pnl"]),
        ]
        for row in balance_rows
    ]
    if balance_table_rows:
        lines.extend(
            hooks.format_table(
                ["Bucket", "Asset", "Quantity", "Cost Basis", "Market Value", "Unrealized"],
                balance_table_rows,
                [16, 6, 14, 14, 14, 14],
                align_right={2, 3, 4, 5},
            )
        )
    else:
        lines.append("No current holdings in scope.")

    lines.extend(["", "Portfolio Summary", "-----------------"])
    portfolio_table_rows = [
        [
            row["wallet"],
            row["account"],
            row["asset"],
            _report_btc(row["quantity"]),
            _report_fiat(row["avg_cost"]),
            _report_fiat(row["cost_basis"]),
            _report_fiat(row["market_value"]),
            _report_fiat(row["unrealized_pnl"]),
        ]
        for row in portfolio_rows
    ]
    if portfolio_table_rows:
        lines.extend(
            hooks.format_table(
                ["Wallet", "Bucket", "Asset", "Quantity", "Avg Cost", "Cost Basis", "Market", "Unreal."],
                portfolio_table_rows,
                [16, 12, 6, 12, 12, 12, 12, 12],
                align_right={3, 4, 5, 6, 7},
            )
        )
    else:
        lines.append("No portfolio rows available.")

    lines.extend(["", "Capital Gains Summary", "---------------------"])
    if capital_rows:
        grouped_capital = {}
        for row in capital_rows:
            key = (row["wallet"], row["asset"])
            bucket = grouped_capital.setdefault(
                key,
                {
                    "wallet": row["wallet"],
                    "asset": row["asset"],
                    "count": 0,
                    "proceeds": 0.0,
                    "cost_basis": 0.0,
                    "gain_loss": 0.0,
                },
            )
            bucket["count"] += 1
            bucket["proceeds"] += float(row["proceeds"])
            bucket["cost_basis"] += float(row["cost_basis"])
            bucket["gain_loss"] += float(row["gain_loss"])
        lines.extend(
            hooks.format_table(
                ["Wallet", "Asset", "Rows", "Proceeds", "Cost Basis", "Gain/Loss"],
                [
                    [
                        bucket["wallet"],
                        bucket["asset"],
                        _report_count(bucket["count"]),
                        _report_fiat(bucket["proceeds"]),
                        _report_fiat(bucket["cost_basis"]),
                        _report_fiat(bucket["gain_loss"]),
                    ]
                    for bucket in grouped_capital.values()
                ],
                [16, 6, 6, 14, 14, 14],
                align_right={2, 3, 4, 5},
            )
        )
        lines.extend(["", "Capital Gains Detail", "--------------------"])
        detail_rows = [
            [
                row["occurred_at"][:10],
                row["wallet"],
                row["asset"],
                _report_btc(row["quantity"]),
                _report_fiat(row["proceeds"]),
                _report_fiat(row["cost_basis"]),
                _report_fiat(row["gain_loss"]),
            ]
            for row in capital_rows
        ]
        lines.extend(
            hooks.format_table(
                ["Date", "Wallet", "Asset", "Qty", "Proceeds", "Basis", "Gain/Loss"],
                detail_rows,
                [10, 16, 6, 12, 12, 12, 12],
                align_right={3, 4, 5, 6},
            )
        )
    else:
        lines.append("No realized disposals in scope.")

    lines.extend(["", "Balance History", "---------------"])
    if history_rows:
        lines.extend(
            hooks.format_table(
                ["Period Start", "Asset", "Quantity", "Cost Basis", "Market Value"],
                [
                    [
                        row["period_start"][:10],
                        row["asset"],
                        _report_btc(row["quantity"]),
                        _report_fiat(row["cumulative_cost_basis"]),
                        _report_fiat(row["market_value"]),
                    ]
                    for row in history_rows
                ],
                [12, 6, 14, 14, 14],
                align_right={2, 3, 4},
            )
        )
    else:
        lines.append("No balance history rows available.")

    lines.extend(["", "Data Quality", "------------"])
    if query_rows["quarantine_rows"]:
        lines.extend(
            hooks.format_table(
                ["Reason", "Count"],
                [[row["reason"], _report_count(row["count"])] for row in query_rows["quarantine_rows"]],
                [28, 10],
                align_right={1},
            )
        )
    else:
        lines.append("No quarantined transactions.")

    lines.extend(["", "Transactions", "------------"])
    if query_rows["transactions"]:
        lines.extend(
            hooks.format_table(
                ["Date", "Wallet", "Dir", "Asset", "Amount", "Fee", "Description"],
                [
                    [
                        row["occurred_at"][:10],
                        row["wallet"],
                        row["direction"][:3],
                        row["asset"],
                        _report_btc(msat_to_btc(row["amount"] or 0)),
                        _report_btc(msat_to_btc(row["fee"] or 0)),
                        row["description"],
                    ]
                    for row in query_rows["transactions"]
                ],
                [10, 14, 3, 6, 12, 12, 28],
                align_right={4, 5},
            )
        )
    else:
        lines.append("No transactions in scope.")

    return title, lines


def export_pdf_report(conn, workspace_ref, profile_ref, file_path, hooks: ReportHooks, wallet_ref=None, history_limit=None):
    title, lines = build_pdf_report_lines(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        wallet_ref=wallet_ref,
        history_limit=history_limit,
    )
    written = dict(hooks.write_text_pdf(file_path, title, lines))
    written["wallet"] = wallet_ref or ""
    return written


def export_summary_pdf_report(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: ReportHooks,
    *,
    start=None,
    end=None,
    wallet_refs=None,
    include_snapshot=False,
):
    from ..summary_pdf_report import write_summary_pdf

    report = build_summary_pdf_report_data(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        start=start,
        end=end,
        wallet_refs=wallet_refs,
        include_snapshot=include_snapshot,
    )
    written = dict(write_summary_pdf(file_path, report))
    written.update(
        {
            "timeframe": report["timeframe"],
            "wallets": report["wallets"],
            "snapshot": bool(report.get("snapshot")),
            "data_integrity": report["data_integrity"],
            "wallet_holdings": report["wallet_holdings"],
            "holdings_totals": report["holdings_totals"],
            "metrics": report["metrics"],
            "benchmark": report.get("benchmark"),
            "top_movements": report.get("top_movements") or [],
            "top_disposals": report.get("top_disposals") or [],
            "holding_age": report.get("holding_age"),
            "snapshot_totals": (
                {
                    "total_quantity": report["snapshot"]["total_quantity"],
                    "asset_quantities": report["snapshot"]["asset_quantities"],
                    "total_market_value": report["snapshot"]["total_market_value"],
                }
                if report.get("snapshot")
                else None
            ),
            "snapshot_wallets": report["snapshot"]["wallets"] if report.get("snapshot") else None,
            "balance_history": report["balance_history"],
        }
    )
    return written


def export_exit_tax_pdf_report(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: ReportHooks,
    *,
    departure_date=None,
    destination=None,
):
    report = report_exit_tax(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        departure_date=departure_date,
        destination=destination,
    )
    lines = format_exit_tax_lines(report)
    title = f"Exit-tax estimate — {report['profile']} ({report['departureDate']})"
    written = dict(hooks.write_text_pdf(file_path, title, lines))
    written.update(
        {
            "scope": "exit_tax",
            "format": "pdf",
            "departure_date": report["departureDate"],
            "destination": report["destination"],
        }
    )
    return written


def _exit_tax_xlsx_specs(report):
    totals = report["totals"]
    ccy = report["fiatCurrency"]

    def _btc(sats):
        return float(Decimal(int(sats)) / Decimal("100000000"))

    overview = [
        {"field": "Profile", "value": report["profile"]},
        {"field": "Jurisdiction", "value": report["jurisdictionCode"]},
        {"field": "Departure date", "value": report["departureDate"]},
        {"field": "Destination", "value": report["destination"]},
        {"field": "Collection timing", "value": totals["collectionTiming"]},
        {"field": "Accounting method", "value": report["method"]},
        {"field": f"Neubestand gain ({ccy})", "value": totals["neuGain"]},
        {"field": f"Taxable gain ({ccy})", "value": totals["taxableGain"]},
        {"field": "Rate", "value": totals["estimatedTaxRate"]},
        {"field": f"Estimated exit tax ({ccy})", "value": totals["estimatedTax"]},
        {"field": f"Altbestand market value — excluded ({ccy})", "value": totals["altMarketValue"]},
    ]
    lots = [
        {
            "asset": lot["asset"],
            "regime": lot["regime"],
            "quantity": _btc(lot["quantitySats"]),
            "cost_basis": lot["costBasis"],
            "market_value": lot["marketValue"],
            "gain_loss": lot["gain"],
            "taxable": "yes" if lot["taxable"] else "no",
            "kennzahl": lot["kennzahl"],
        }
        for lot in report["lots"]
    ]
    holdings = [
        {
            "asset": holding["asset"],
            "wallet": holding["wallet"],
            "quantity": _btc(holding["quantitySats"]),
            "market_value": holding["marketValue"],
        }
        for holding in report["walletHoldings"]
    ]
    notes = [{"note": note} for note in report["assumptions"]]
    notes.append({"note": report["reviewGate"]})
    return [
        {
            "sheet_name": "Overview",
            "title": f"Exit-tax estimate — {report['profile']}",
            "headers": ["field", "value"],
            "rows": overview,
        },
        {
            "sheet_name": "Deemed disposal",
            "title": "Deemed disposal at fair market value",
            "headers": ["asset", "regime", "quantity", "cost_basis", "market_value", "gain_loss", "taxable", "kennzahl"],
            "rows": lots,
        },
        {
            "sheet_name": "Wallet holdings",
            "title": "Wallet holdings (context)",
            "headers": ["asset", "wallet", "quantity", "market_value"],
            "rows": holdings,
        },
        {
            "sheet_name": "Assumptions",
            "title": "Assumptions & Steuerberater review gate",
            "headers": ["note"],
            "rows": notes,
        },
    ]


def export_exit_tax_xlsx_report(
    conn,
    workspace_ref,
    profile_ref,
    file_path,
    hooks: ReportHooks,
    *,
    departure_date=None,
    destination=None,
):
    import xlsxwriter

    report = report_exit_tax(
        conn,
        workspace_ref,
        profile_ref,
        hooks,
        departure_date=departure_date,
        destination=destination,
    )
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    workbook = xlsxwriter.Workbook(str(path))
    workbook.set_properties(
        {
            "title": f"Kassiber Exit-Tax Estimate — {report['profile']} ({report['departureDate']})",
            "subject": "Wegzugsbesteuerung deemed-disposal estimate",
            "author": "Kassiber",
            "comments": report["reviewGate"],
        }
    )
    formats = _generic_report_xlsx_formats(workbook)
    specs = _exit_tax_xlsx_specs(report)
    for spec in specs:
        _generic_report_xlsx_write_sheet(workbook, spec, formats)
    workbook.close()
    return {
        "file": str(path.resolve()),
        "bytes": path.stat().st_size,
        "scope": "exit_tax",
        "format": "xlsx",
        "departure_date": report["departureDate"],
        "destination": report["destination"],
        "sheets": [spec["sheet_name"] for spec in specs],
        "rows": len(report["lots"]),
    }


__all__ = [
    "DEFAULT_BALANCE_HISTORY_INTERVAL",
    "INTERVAL_CHOICES",
    "ReportHooks",
    "build_austrian_e1kv_report_lines",
    "build_exit_tax_report_lines",
    "compute_deemed_disposal",
    "export_exit_tax_pdf_report",
    "export_exit_tax_xlsx_report",
    "format_exit_tax_lines",
    "report_exit_tax",
    "build_pdf_report_lines",
    "build_summary_pdf_report_data",
    "export_austrian_e1kv_csv_bundle",
    "export_austrian_e1kv_pdf_report",
    "export_austrian_e1kv_xlsx_report",
    "export_csv_report",
    "export_pdf_report",
    "export_summary_pdf_report",
    "export_transactions_csv_report",
    "export_transactions_xlsx_report",
    "export_xlsx_report",
    "latest_market_rates_for_profile",
    "latest_transaction_rates_for_profile",
    "report_austrian_e1kv",
    "report_balance_history",
    "report_balance_sheet",
    "report_capital_gains",
    "report_journal_entries",
    "report_portfolio_summary",
    "build_summary_report_lines",
    "report_summary",
    "report_tax_summary",
]
