"""Project catalog and legacy layout migration helpers.

The project catalog is intentionally boring: it stores only routing metadata
needed to find a project container. SQLCipher passphrases, verifier material,
descriptors, backend tokens, xpubs, wallet config, accounting rows, and chat
history stay out of this file.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import sqlite3
import tempfile
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .db import (
    DEFAULT_ATTACHMENTS_DIRNAME,
    DEFAULT_CONFIG_DIRNAME,
    DEFAULT_DATA_DIRNAME,
    DEFAULT_DATA_ROOT,
    DEFAULT_DB_FILENAME,
    DEFAULT_EXPORTS_DIRNAME,
    DEFAULT_STATE_ROOT,
    LEGACY_DATA_ROOT,
    LEGACY_DB_FILENAME,
    LEGACY_XDG_DATA_ROOT,
    resolve_database_path,
    resolve_effective_state_root,
)
from .errors import AppError
from .secrets.sqlcipher import looks_like_plaintext_sqlite


CATALOG_SCHEMA_VERSION = 1
DEFAULT_PROJECT_ID = "default"
DEFAULT_PROJECT_NAME = "Default"
DEFAULT_PROJECTS_DIRNAME = "projects"
PROJECT_CATALOG_FILENAME = "projects.json"
PROJECT_MIGRATION_MARKER = "legacy-project-migration.json"
MIGRATION_REPORTS_DIRNAME = "migration-reports"
LEGACY_MIGRATION_BACKUP_PREFIX = "pre-project-migration-"


WORKSPACE_SPLIT_POLICY: dict[str, Any] = {
    "schema_version": 1,
    "policy": "legacy_app_wide_to_project_containers",
    "workspace_profile_scoped_tables": [
        "workspaces",
        "profiles",
        "accounts",
        "wallets",
        "transactions",
        "tags",
        "wallet_utxos",
        "wallet_utxo_refreshes",
        "transaction_edit_events",
        "journal_entries",
        "journal_quarantines",
        "journal_tax_summary",
        "journal_account_holdings",
        "journal_wallet_holdings",
        "journal_quantity_postings",
        "journal_quantity_issues",
        "journal_quantity_balances",
        "journal_custody_decisions",
        "custody_authored_evidence_snapshots",
        "custody_components",
        "custody_component_legs",
        "custody_component_allocations",
        "custody_component_evidence_commitments",
        "custody_gap_reviews",
        "custody_gap_review_relation_sets",
        "custody_gap_review_transactions",
        "filed_report_snapshots",
        "custody_filed_report_impacts",
        "custody_filed_report_impact_resolutions",
        "transaction_pairs",
        "direct_swap_payouts",
        "transaction_pair_dismissals",
        "loan_legs",
        "swap_matching_rules",
        "saved_views",
        "ai_chat_sessions",
        "bip329_labels",
        "lightning_node_syncs",
        "lightning_node_records",
        "freshness_source_states",
        "freshness_jobs",
        "attachments",
        "btcpay_provenance_records",
        "external_documents",
        "commercial_links",
        "source_funds_sources",
        "source_funds_links",
        "source_funds_cases",
        "source_funds_recipients",
    ],
    "relationship_tables_following_parent_rows": [
        "transaction_tags",
        "transaction_edit_fields",
        "ai_chat_messages",
        "external_document_attachments",
        "source_funds_link_attachments",
        "source_funds_source_attachments",
    ],
    "project_shared_tables_copied_to_each_split_project": [
        "settings",
        "backends",
        "ai_providers",
        "ai_provider_secret_refs",
        "rates_cache",
        "rates_checked_minutes",
        "transaction_graph_cache",
        "source_funds_snapshots",
    ],
    "filesystem_policy": {
        "attachments": (
            "copied into the target project container; attachment rows remain "
            "profile-scoped and orphan cleanup may prune unused files later"
        ),
        "exports": (
            "copied for single-project migration; for future split migration, "
            "copied to each staged project because exports are generated files "
            "outside SQLCipher, not accounting source of truth"
        ),
        "backends_env": (
            "copied per project as plaintext bootstrap/addressing metadata; "
            "secret-shaped entries must be lifted into the encrypted backends "
            "table before the file is considered sanitized"
        ),
        "settings_json": (
            "regenerated or rewritten per project with project-local paths only"
        ),
        "logs": "not migrated because Kassiber logs are RAM-only unless explicitly exported",
    },
}


@dataclass(frozen=True)
class ProjectEntry:
    id: str
    name: str
    path: str
    encrypted: bool
    last_opened_at: str | None = None

    @property
    def root(self) -> Path:
        return Path(self.path).expanduser()

    @property
    def data_root(self) -> Path:
        return self.root / DEFAULT_DATA_DIRNAME

    @property
    def database(self) -> Path:
        return resolve_database_path(self.data_root)

    def to_catalog_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "path": str(self.root),
            "encrypted": bool(self.encrypted),
            "last_opened_at": self.last_opened_at,
        }


@dataclass(frozen=True)
class LegacyLayout:
    data_root: Path
    state_root: Path
    database: Path


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _chmod_best_effort(path: Path, mode: int) -> None:
    try:
        path.chmod(mode)
    except OSError:
        return


def default_state_root() -> Path:
    return Path(DEFAULT_STATE_ROOT).expanduser()


def projects_root(state_root: str | Path | None = None) -> Path:
    root = Path(state_root).expanduser() if state_root is not None else default_state_root()
    return root / DEFAULT_PROJECTS_DIRNAME


def catalog_path(state_root: str | Path | None = None) -> Path:
    root = Path(state_root).expanduser() if state_root is not None else default_state_root()
    return root / DEFAULT_CONFIG_DIRNAME / PROJECT_CATALOG_FILENAME


def project_root_for_id(
    project_id: str,
    *,
    state_root: str | Path | None = None,
) -> Path:
    return projects_root(state_root) / sanitize_project_id(project_id)


def sanitize_project_id(value: str) -> str:
    slug = re.sub(r"[^a-zA-Z0-9_.-]+", "-", value.strip().lower()).strip(".-")
    if not slug:
        raise AppError(
            "project id must contain at least one letter or number",
            code="validation",
            retryable=False,
        )
    if slug in {".", ".."}:
        raise AppError("invalid project id", code="validation", retryable=False)
    return slug[:80]


def project_id_from_name(name: str) -> str:
    return sanitize_project_id(name)


def _empty_catalog() -> dict[str, Any]:
    return {
        "schema_version": CATALOG_SCHEMA_VERSION,
        "selected_project_id": None,
        "projects": [],
    }


def _entry_from_dict(raw: dict[str, Any]) -> ProjectEntry | None:
    project_id = raw.get("id")
    name = raw.get("name")
    path = raw.get("path")
    if not isinstance(project_id, str) or not project_id.strip():
        return None
    if not isinstance(name, str) or not name.strip():
        return None
    if not isinstance(path, str) or not path.strip():
        return None
    last_opened = raw.get("last_opened_at")
    return ProjectEntry(
        id=sanitize_project_id(project_id),
        name=name.strip(),
        path=str(Path(path).expanduser()),
        encrypted=bool(raw.get("encrypted")),
        last_opened_at=last_opened if isinstance(last_opened, str) else None,
    )


def load_catalog(path: str | Path | None = None) -> dict[str, Any]:
    cpath = Path(path).expanduser() if path is not None else catalog_path()
    if not cpath.exists():
        return _empty_catalog()
    try:
        loaded = json.loads(cpath.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError, TypeError, ValueError):
        return _empty_catalog()
    if not isinstance(loaded, dict):
        return _empty_catalog()
    projects = []
    for raw in loaded.get("projects") or []:
        if isinstance(raw, dict):
            entry = _entry_from_dict(raw)
            if entry is not None:
                projects.append(entry.to_catalog_dict())
    selected = loaded.get("selected_project_id")
    if not isinstance(selected, str) or not selected:
        selected = projects[0]["id"] if projects else None
    return {
        "schema_version": CATALOG_SCHEMA_VERSION,
        "selected_project_id": selected,
        "projects": projects,
    }


def write_catalog(catalog: dict[str, Any], path: str | Path | None = None) -> Path:
    cpath = Path(path).expanduser() if path is not None else catalog_path()
    cpath.parent.mkdir(parents=True, exist_ok=True)
    _chmod_best_effort(cpath.parent, 0o700)
    normalized = {
        "schema_version": CATALOG_SCHEMA_VERSION,
        "selected_project_id": catalog.get("selected_project_id"),
        "projects": [],
    }
    seen: set[str] = set()
    for raw in catalog.get("projects") or []:
        if not isinstance(raw, dict):
            continue
        entry = _entry_from_dict(raw)
        if entry is None or entry.id in seen:
            continue
        seen.add(entry.id)
        normalized["projects"].append(entry.to_catalog_dict())
    fd, tmp_name = tempfile.mkstemp(
        prefix=f"{cpath.name}.",
        suffix=".tmp",
        dir=str(cpath.parent),
        text=True,
    )
    tmp_path = Path(tmp_name)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(normalized, indent=2, sort_keys=True) + "\n")
        _chmod_best_effort(tmp_path, 0o600)
        tmp_path.replace(cpath)
        _chmod_best_effort(cpath, 0o600)
    except Exception:
        try:
            tmp_path.unlink()
        except OSError:
            # Preserve the original write/replace error if cleanup also fails.
            pass
        raise
    return cpath


def _database_encrypted_at(data_root: Path) -> bool:
    db_path = resolve_database_path(data_root)
    return (
        db_path.exists()
        and db_path.stat().st_size > 0
        and not looks_like_plaintext_sqlite(db_path)
    )


def _entry_with_current_encryption(entry: ProjectEntry) -> ProjectEntry:
    return ProjectEntry(
        id=entry.id,
        name=entry.name,
        path=str(entry.root),
        encrypted=_database_encrypted_at(entry.data_root),
        last_opened_at=entry.last_opened_at,
    )


def list_projects(*, state_root: str | Path | None = None) -> list[ProjectEntry]:
    cpath = catalog_path(state_root)
    catalog = load_catalog(cpath)
    entries = []
    changed = False
    for raw in catalog.get("projects") or []:
        entry = _entry_from_dict(raw)
        if entry is None:
            continue
        refreshed = _entry_with_current_encryption(entry)
        if refreshed.encrypted != entry.encrypted:
            changed = True
        entries.append(refreshed)
    if changed:
        catalog["projects"] = [entry.to_catalog_dict() for entry in entries]
        write_catalog(catalog, cpath)
    return entries


def get_project(
    project_id: str,
    *,
    state_root: str | Path | None = None,
) -> ProjectEntry:
    wanted = sanitize_project_id(project_id)
    for entry in list_projects(state_root=state_root):
        if entry.id == wanted:
            return entry
    raise AppError(
        f"project {project_id!r} is not registered",
        code="project_not_found",
        details={"project_id": project_id},
        retryable=False,
    )


def selected_project(*, state_root: str | Path | None = None) -> ProjectEntry:
    ensure_default_project(state_root=state_root)
    cpath = catalog_path(state_root)
    catalog = load_catalog(cpath)
    selected = catalog.get("selected_project_id")
    if isinstance(selected, str) and selected:
        return get_project(selected, state_root=state_root)
    entries = list_projects(state_root=state_root)
    if entries:
        return entries[0]
    raise AppError(
        "no Kassiber project is registered",
        code="project_not_found",
        hint="Create a project with `kassiber projects create <name>`.",
        retryable=False,
    )


def set_selected_project(
    project_id: str,
    *,
    state_root: str | Path | None = None,
    last_opened_at: str | None = None,
) -> ProjectEntry:
    entry = get_project(project_id, state_root=state_root)
    cpath = catalog_path(state_root)
    catalog = load_catalog(cpath)
    catalog["selected_project_id"] = entry.id
    updated = []
    for raw in catalog.get("projects") or []:
        existing = _entry_from_dict(raw)
        if existing is None:
            continue
        if existing.id == entry.id:
            existing = ProjectEntry(
                id=existing.id,
                name=existing.name,
                path=str(existing.root),
                encrypted=_database_encrypted_at(existing.data_root),
                last_opened_at=last_opened_at or existing.last_opened_at,
            )
            entry = existing
        updated.append(existing.to_catalog_dict())
    catalog["projects"] = updated
    write_catalog(catalog, cpath)
    return entry


def _upsert_project(
    entry: ProjectEntry,
    *,
    state_root: str | Path | None = None,
    select: bool = False,
) -> ProjectEntry:
    cpath = catalog_path(state_root)
    catalog = load_catalog(cpath)
    entries = []
    replaced = False
    for raw in catalog.get("projects") or []:
        existing = _entry_from_dict(raw)
        if existing is None:
            continue
        if existing.id == entry.id:
            entries.append(entry.to_catalog_dict())
            replaced = True
        else:
            entries.append(existing.to_catalog_dict())
    if not replaced:
        entries.append(entry.to_catalog_dict())
    catalog["projects"] = entries
    if select or not catalog.get("selected_project_id"):
        catalog["selected_project_id"] = entry.id
    write_catalog(catalog, cpath)
    return entry


def create_project(
    name: str,
    *,
    project_id: str | None = None,
    root: str | Path | None = None,
    state_root: str | Path | None = None,
    encrypted: bool | None = None,
    select: bool = True,
    replace_existing: bool = True,
    allow_existing_database: bool = True,
) -> ProjectEntry:
    pid = sanitize_project_id(project_id or project_id_from_name(name))
    project_root = Path(root).expanduser() if root is not None else project_root_for_id(pid, state_root=state_root)
    if not replace_existing:
        catalog = load_catalog(catalog_path(state_root))
        for raw in catalog.get("projects") or []:
            existing = _entry_from_dict(raw) if isinstance(raw, dict) else None
            if existing is not None and existing.id == pid:
                raise AppError(
                    f"project {pid!r} already exists",
                    code="project_exists",
                    details={"project_id": pid, "project_root": str(existing.root)},
                    retryable=False,
                )
    database_path = project_root / DEFAULT_DATA_DIRNAME / DEFAULT_DB_FILENAME
    if (
        not allow_existing_database
        and database_path.exists()
        and database_path.stat().st_size > 0
    ):
        raise AppError(
            f"refusing to create a project over existing database at {database_path}",
            code="database_exists",
            details={"database": str(database_path)},
            retryable=False,
        )
    project_root.mkdir(parents=True, exist_ok=True)
    for dirname in (
        DEFAULT_DATA_DIRNAME,
        DEFAULT_CONFIG_DIRNAME,
        DEFAULT_EXPORTS_DIRNAME,
        DEFAULT_ATTACHMENTS_DIRNAME,
    ):
        (project_root / dirname).mkdir(parents=True, exist_ok=True)
    entry = ProjectEntry(
        id=pid,
        name=name.strip() or pid,
        path=str(project_root),
        encrypted=_database_encrypted_at(project_root / DEFAULT_DATA_DIRNAME)
        if encrypted is None
        else bool(encrypted),
        last_opened_at=None,
    )
    return _upsert_project(entry, state_root=state_root, select=select)


def ensure_default_project(*, state_root: str | Path | None = None) -> ProjectEntry:
    cpath = catalog_path(state_root)
    catalog = load_catalog(cpath)
    entries = catalog.get("projects") or []
    if entries:
        selected = catalog.get("selected_project_id")
        if isinstance(selected, str) and selected:
            return get_project(selected, state_root=state_root)
        first = _entry_from_dict(entries[0])
        if first is not None:
            catalog["selected_project_id"] = first.id
            write_catalog(catalog, cpath)
            return first

    migrated = migrate_legacy_default_layout_if_needed(state_root=state_root)
    if migrated is not None:
        return migrated
    return create_project(
        DEFAULT_PROJECT_NAME,
        project_id=DEFAULT_PROJECT_ID,
        state_root=state_root,
        select=True,
    )


def project_for_runtime(
    *,
    project_id: str | None = None,
    state_root: str | Path | None = None,
) -> ProjectEntry:
    if project_id:
        ensure_default_project(state_root=state_root)
        return get_project(project_id, state_root=state_root)
    return selected_project(state_root=state_root)


def mark_project_opened(
    project_id: str,
    *,
    data_root: str | Path | None = None,
    state_root: str | Path | None = None,
    select: bool = True,
) -> ProjectEntry:
    return refresh_project_metadata(
        project_id,
        data_root=data_root,
        state_root=state_root,
        last_opened_at=now_iso(),
        select=select,
    )


def refresh_project_metadata(
    project_id: str,
    *,
    data_root: str | Path | None = None,
    state_root: str | Path | None = None,
    last_opened_at: str | None = None,
    select: bool = False,
) -> ProjectEntry:
    if data_root is not None:
        entry = get_project(project_id, state_root=state_root)
        entry = ProjectEntry(
            id=entry.id,
            name=entry.name,
            path=str(entry.root),
            encrypted=_database_encrypted_at(Path(data_root).expanduser()),
            last_opened_at=last_opened_at or entry.last_opened_at,
        )
        return _upsert_project(entry, state_root=state_root, select=select)
    if last_opened_at is not None or select:
        return set_selected_project(project_id, state_root=state_root, last_opened_at=last_opened_at)
    entry = get_project(project_id, state_root=state_root)
    entry = _entry_with_current_encryption(entry)
    return _upsert_project(entry, state_root=state_root, select=False)


def project_metadata_for_data_root(data_root: str | Path) -> dict[str, Any] | None:
    data = Path(data_root).expanduser()
    for entry in list_projects():
        try:
            if entry.data_root.resolve() == data.resolve():
                return entry.to_catalog_dict()
        except OSError:
            continue
    return None


def _legacy_database_path() -> Path | None:
    layout = _legacy_layout()
    return layout.database if layout is not None else None


def _legacy_layout() -> LegacyLayout | None:
    candidate_data_roots = (
        Path(DEFAULT_DATA_ROOT).expanduser(),
        Path(LEGACY_XDG_DATA_ROOT).expanduser(),
        Path(LEGACY_DATA_ROOT).expanduser(),
    )
    for data_root in candidate_data_roots:
        for filename in (DEFAULT_DB_FILENAME, LEGACY_DB_FILENAME):
            database = data_root / filename
            if not database.exists():
                continue
            return LegacyLayout(
                data_root=data_root,
                state_root=Path(resolve_effective_state_root(data_root)).expanduser(),
                database=database,
            )
    return None


def _workspace_count_for_plaintext(db_path: Path) -> int | None:
    if not db_path.exists() or not looks_like_plaintext_sqlite(db_path):
        return None
    conn = sqlite3.connect(str(db_path))
    try:
        exists = conn.execute(
            """
            SELECT 1
            FROM sqlite_master
            WHERE type = 'table' AND name = 'workspaces'
            """
        ).fetchone()
        if not exists:
            return 0
        return int(conn.execute("SELECT COUNT(*) FROM workspaces").fetchone()[0])
    finally:
        conn.close()


def _write_migration_report(
    *,
    state_root: Path,
    status: str,
    source_state_root: Path,
    source_database: Path | None,
    target_project_root: Path,
    details: dict[str, Any] | None = None,
) -> Path:
    reports = state_root / DEFAULT_CONFIG_DIRNAME / MIGRATION_REPORTS_DIRNAME
    reports.mkdir(parents=True, exist_ok=True)
    path = reports / f"{now_iso().replace(':', '')}-legacy-project-migration.json"
    payload = {
        "schema_version": 1,
        "status": status,
        "created_at": now_iso(),
        "source": {
            "state_root": str(source_state_root),
            "database": str(source_database) if source_database else None,
        },
        "target": {
            "project_id": DEFAULT_PROJECT_ID,
            "project_root": str(target_project_root),
            "data_root": str(target_project_root / DEFAULT_DATA_DIRNAME),
        },
        "split_policy": WORKSPACE_SPLIT_POLICY,
        "details": details or {},
    }
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def _copy_if_exists(source: Path, target: Path) -> bool:
    if not source.exists():
        return False
    if source.is_dir():
        shutil.copytree(source, target, dirs_exist_ok=True)
    else:
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
    return True


def _backup_dir_for_legacy_migration(source_state: Path) -> Path:
    base = source_state / (
        LEGACY_MIGRATION_BACKUP_PREFIX
        + datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    )
    candidate = base
    suffix = 1
    while candidate.exists():
        suffix += 1
        candidate = Path(f"{base}-{suffix}")
    return candidate


def _move_if_exists(source: Path, backup_root: Path, source_state: Path) -> str | None:
    if not source.exists():
        return None
    try:
        rel = source.relative_to(source_state)
    except ValueError:
        rel = Path(source.name)
    target = backup_root / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(source), str(target))
    return str(target)


def _move_legacy_artifacts_aside(layout: LegacyLayout) -> dict[str, Any]:
    backup_root = _backup_dir_for_legacy_migration(layout.state_root)
    moved: dict[str, str] = {}
    for key, source in (
        ("database", layout.database),
        ("attachments", layout.state_root / DEFAULT_ATTACHMENTS_DIRNAME),
        ("exports", layout.state_root / DEFAULT_EXPORTS_DIRNAME),
        ("backends_env", layout.state_root / DEFAULT_CONFIG_DIRNAME / "backends.env"),
        ("settings_json", layout.state_root / DEFAULT_CONFIG_DIRNAME / "settings.json"),
    ):
        moved_to = _move_if_exists(source, backup_root, layout.state_root)
        if moved_to is not None:
            moved[key] = moved_to
    if not moved:
        return {"moved": {}, "backup_root": None}
    return {"moved": moved, "backup_root": str(backup_root)}


def _register_existing_default_project_after_legacy_recovery(
    layout: LegacyLayout,
    target_root: Path,
) -> ProjectEntry:
    _move_legacy_artifacts_aside(layout)
    return create_project(
        DEFAULT_PROJECT_NAME,
        project_id=DEFAULT_PROJECT_ID,
        root=target_root,
        encrypted=_database_encrypted_at(target_root / DEFAULT_DATA_DIRNAME),
        select=True,
    )


def migrate_legacy_default_layout_if_needed(
    *,
    state_root: str | Path | None = None,
) -> ProjectEntry | None:
    if state_root is not None:
        return None
    layout = _legacy_layout()
    if layout is None:
        return None
    source_state = layout.state_root
    source_db = layout.database

    target_root = project_root_for_id(DEFAULT_PROJECT_ID)
    target_db = target_root / DEFAULT_DATA_DIRNAME / DEFAULT_DB_FILENAME
    if target_db.exists():
        return _register_existing_default_project_after_legacy_recovery(
            layout,
            target_root,
        )

    workspace_count = _workspace_count_for_plaintext(source_db)

    staging_root = target_root.with_name(target_root.name + ".migrating")
    if staging_root.exists():
        if target_db.exists():
            shutil.rmtree(staging_root, ignore_errors=True)
            return _register_existing_default_project_after_legacy_recovery(
                layout,
                target_root,
            )
        shutil.rmtree(staging_root, ignore_errors=True)
    try:
        staging_root.mkdir(parents=True, exist_ok=False)
        for dirname in (
            DEFAULT_DATA_DIRNAME,
            DEFAULT_CONFIG_DIRNAME,
            DEFAULT_EXPORTS_DIRNAME,
            DEFAULT_ATTACHMENTS_DIRNAME,
        ):
            (staging_root / dirname).mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_db, staging_root / DEFAULT_DATA_DIRNAME / DEFAULT_DB_FILENAME)
        copied = {
            "database": True,
            "attachments": _copy_if_exists(
                source_state / DEFAULT_ATTACHMENTS_DIRNAME,
                staging_root / DEFAULT_ATTACHMENTS_DIRNAME,
            ),
            "exports": _copy_if_exists(
                source_state / DEFAULT_EXPORTS_DIRNAME,
                staging_root / DEFAULT_EXPORTS_DIRNAME,
            ),
            "backends_env": _copy_if_exists(
                source_state / DEFAULT_CONFIG_DIRNAME / "backends.env",
                staging_root / DEFAULT_CONFIG_DIRNAME / "backends.env",
            ),
            "settings_json": _copy_if_exists(
                source_state / DEFAULT_CONFIG_DIRNAME / "settings.json",
                staging_root / DEFAULT_CONFIG_DIRNAME / "settings.json",
            ),
        }
        status = (
            "staged_multi_workspace_single_project_copy"
            if workspace_count is not None and workspace_count > 1
            else "staged_single_project_copy"
        )
        marker = {
            "schema_version": 1,
            "source_state_root": str(source_state),
            "source_data_root": str(layout.data_root),
            "source_database": str(source_db),
            "created_at": now_iso(),
            "requires_single_workspace_validation": workspace_count is None,
            "legacy_workspace_count": workspace_count,
        }
        marker_path = staging_root / DEFAULT_CONFIG_DIRNAME / PROJECT_MIGRATION_MARKER
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        report = _write_migration_report(
            state_root=source_state,
            status=status,
            source_state_root=source_state,
            source_database=source_db,
            target_project_root=target_root,
            details={
                "copied": copied,
                "workspace_count": workspace_count,
                "reason": (
                    "legacy DB has multiple workspaces; migrated as one default "
                    "project container because a project may contain multiple books"
                )
                if workspace_count is not None and workspace_count > 1
                else "legacy DB migrated as one default project container",
            },
        )
        marker["report"] = str(report)
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        staging_root.replace(target_root)
        moved = _move_legacy_artifacts_aside(layout)
        marker_path = target_root / DEFAULT_CONFIG_DIRNAME / PROJECT_MIGRATION_MARKER
        marker["legacy_artifacts"] = moved
        marker["legacy_artifacts_moved_at"] = now_iso()
        marker_path.write_text(
            json.dumps(marker, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except Exception:
        shutil.rmtree(staging_root, ignore_errors=True)
        raise

    return create_project(
        DEFAULT_PROJECT_NAME,
        project_id=DEFAULT_PROJECT_ID,
        root=target_root,
        encrypted=_database_encrypted_at(target_root / DEFAULT_DATA_DIRNAME),
        select=True,
    )


def validate_project_migration_after_unlock(data_root: str | Path, conn: sqlite3.Connection) -> None:
    state_root = Path(resolve_effective_state_root(data_root)).expanduser()
    marker_path = state_root / DEFAULT_CONFIG_DIRNAME / PROJECT_MIGRATION_MARKER
    if not marker_path.exists():
        return
    try:
        marker = json.loads(marker_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError, TypeError, ValueError):
        marker = {}
    if not marker.get("requires_single_workspace_validation"):
        return
    workspace_count = int(conn.execute("SELECT COUNT(*) FROM workspaces").fetchone()[0])
    source_state = Path(marker.get("source_state_root") or DEFAULT_STATE_ROOT).expanduser()
    source_db_raw = marker.get("source_database")
    source_db = Path(source_db_raw).expanduser() if isinstance(source_db_raw, str) else None
    if workspace_count > 1:
        report = _write_migration_report(
            state_root=source_state,
            status="validated_multi_workspace_single_project",
            source_state_root=source_state,
            source_database=source_db,
            target_project_root=state_root,
            details={
                "workspace_count": workspace_count,
                "reason": (
                    "encrypted legacy DB has multiple workspaces; it remains a "
                    "single project container until the user chooses an explicit split"
                ),
            },
        )
        marker["report_after_unlock"] = str(report)
    marker["requires_single_workspace_validation"] = False
    marker["legacy_workspace_count"] = workspace_count
    marker["validated_at"] = now_iso()
    marker_path.write_text(
        json.dumps(marker, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
