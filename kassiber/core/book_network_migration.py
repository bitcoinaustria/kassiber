"""Non-destructive, closure-checked encrypted network partition exports.

Stable authored IDs and their commitments survive because each partition gets
its own database. No connection credentials or other profiles enter the bundle.
"""
from __future__ import annotations

import io
import shutil
import json
import os
import re
import sqlite3
import tarfile
import tempfile
from pathlib import Path

from ..backup.age_cli import encrypt_age_stream
from ..db import RETIRED_DEVICE_SYNC_COLUMNS, open_db, resolve_attachments_root
from .attachments import _resolve_stored_path, _hash_file
from .book_network import ENVIRONMENTS, _digest, _error, inventory_book_network, plan_book_network, apply_book_network

_PRIVATE_EXTENDED_KEY = re.compile(r"(?:^|[^a-z])(xprv|yprv|zprv|tprv|uprv|vprv)[a-z0-9]*", re.IGNORECASE)
_WIF_PRIVATE_KEY = re.compile(r"(?<![A-Za-z0-9])[5KLc9][1-9A-HJ-NP-Za-km-z]{50,51}(?![A-Za-z0-9])")
_RAW_HEX_PRIVATE_KEY = re.compile(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64}(?![0-9A-Fa-f])")
# Wallet configuration a partition carries: public watch policy only. Anything
# else (backend bindings, credentials, private or confidential descriptors)
# stays in the original book and must be reconnected explicitly.
_PUBLIC_WALLET_CONFIG_FIELDS = frozenset(
    {
        "addresses",
        "chain",
        "network",
        "chain_instance_id",
        "gap_limit",
        "policy_asset",
        "altbestand",
        "descriptor_source",
        "synthesize_change",
        "script_types",
        "descriptor",
        "change_descriptor",
        "xpub",
        "deprecated",
    }
)


def _is_public_watch_material(value):
    if not isinstance(value, str) or not value.strip():
        return False
    text = value.strip()
    if (
        _PRIVATE_EXTENDED_KEY.search(text)
        or _WIF_PRIVATE_KEY.search(text)
        or _RAW_HEX_PRIVATE_KEY.search(text)
    ):
        return False
    # Confidential descriptors carry blinding material; the user re-enters it.
    lowered = text.lower()
    return not (lowered.startswith("ct(") or "slip77(" in lowered)


def public_wallet_config(raw):
    """Project wallet config onto the public watch policy a partition keeps."""
    if isinstance(raw, str):
        try:
            raw = json.loads(raw)
        except json.JSONDecodeError:
            return {}
    if not isinstance(raw, dict):
        return {}
    output = {}
    for key in sorted(_PUBLIC_WALLET_CONFIG_FIELDS):
        if key not in raw:
            continue
        value = raw[key]
        if key in {"descriptor", "change_descriptor", "xpub"}:
            if _is_public_watch_material(value):
                output[key] = value
            continue
        output[key] = value
    return output


def _profile_scope(table):
    return f"SELECT * FROM {table} WHERE profile_id = ?"


# The book's authored tables, in dependency order. Each query selects one
# profile's rows; derived journals, caches, secrets and settings are absent.
AUTHORED_SCOPES = {
    "workspaces": "SELECT * FROM workspaces WHERE id = (SELECT workspace_id FROM profiles WHERE id = ?)",
    "profiles": "SELECT * FROM profiles WHERE id = ?",
    "filed_report_snapshots": _profile_scope("filed_report_snapshots"),
    "accounts": _profile_scope("accounts"),
    "wallets": _profile_scope("wallets"),
    "transactions": _profile_scope("transactions"),
    "tags": _profile_scope("tags"),
    "transaction_tags": "SELECT tt.* FROM transaction_tags tt JOIN transactions t ON t.id = tt.transaction_id WHERE t.profile_id = ?",
    **{
        table: _profile_scope(table)
        for table in (
            "custody_components",
            "custody_component_legs",
            "custody_component_allocations",
            "custody_component_economic_terms",
            "custody_component_evidence_commitments",
            "custody_gap_reviews",
            "custody_gap_review_relation_sets",
            "custody_gap_review_transactions",
            "custody_filed_report_impacts",
            "custody_filed_report_impact_resolutions",
            "transaction_pairs",
            "direct_swap_payouts",
            "transaction_pair_dismissals",
            "loan_legs",
            "swap_matching_rules",
            "saved_views",
            "bip329_labels",
            "attachments",
            "external_documents",
        )
    },
    "external_document_attachments": "SELECT j.* FROM external_document_attachments j JOIN external_documents d ON d.id = j.document_id WHERE d.profile_id = ?",
    "commercial_links": _profile_scope("commercial_links"),
    "source_funds_sources": _profile_scope("source_funds_sources"),
    "source_funds_links": _profile_scope("source_funds_links"),
    "source_funds_link_attachments": "SELECT j.* FROM source_funds_link_attachments j JOIN source_funds_links l ON l.id = j.link_id WHERE l.profile_id = ?",
    "source_funds_source_attachments": "SELECT j.* FROM source_funds_source_attachments j JOIN source_funds_sources s ON s.id = j.source_id WHERE s.profile_id = ?",
    "source_funds_cases": _profile_scope("source_funds_cases"),
    "source_funds_snapshots": "SELECT s.* FROM source_funds_snapshots s JOIN source_funds_cases c ON c.id = s.case_id WHERE c.profile_id = ?",
    "source_funds_recipients": _profile_scope("source_funds_recipients"),
    "book_network_bindings": _profile_scope("book_network_bindings"),
}

# Disposable SDK state is deliberately not carried. Historical observer proofs
# and policy epochs are retained; new sources must be reconnected explicitly.
EXTRA_SCOPES = {
    "transaction_edit_events": "SELECT * FROM transaction_edit_events WHERE profile_id=?",
    "transaction_edit_fields": "SELECT f.* FROM transaction_edit_fields f JOIN transaction_edit_events e ON e.id=f.event_id WHERE e.profile_id=?",
    "custody_authored_evidence_snapshots": "SELECT * FROM custody_authored_evidence_snapshots WHERE profile_id=?",
    "btcpay_provenance_records": "SELECT * FROM btcpay_provenance_records WHERE profile_id=?",
    "chain_observation_provenance": "SELECT * FROM chain_observation_provenance WHERE profile_id=?",
    "wallet_policy_epochs": "SELECT * FROM wallet_policy_epochs WHERE profile_id=?",
    "wallet_policy_sources": "SELECT s.* FROM wallet_policy_sources s JOIN wallet_policy_epochs e ON e.id=s.epoch_id WHERE e.profile_id=?",
    "wallet_policy_coverage_witnesses": "SELECT w.* FROM wallet_policy_coverage_witnesses w JOIN wallet_policy_sources s ON s.id=w.source_id JOIN wallet_policy_epochs e ON e.id=s.epoch_id WHERE e.profile_id=?",
}
# Frozen reports/cases remain in the original; copying them would imply that
# their original whole-book totals described the new partition.
HISTORICAL_ONLY = {"book_network_bindings","filed_report_snapshots", "custody_filed_report_impacts", "custody_filed_report_impact_resolutions", "source_funds_cases"}
SHARED = {"profiles", "workspaces", "accounts", "tags"}


def _rows(conn, profile_id):
    scopes = {table: sql for table, sql in AUTHORED_SCOPES.items() if table not in HISTORICAL_ONLY}
    scopes.update(EXTRA_SCOPES)
    tables = {}
    for table, sql in scopes.items():
        tables[table] = [dict(row) for row in conn.execute(sql, (profile_id,))]
    return tables


def _references(value, identities):
    if isinstance(value, str):
        if value in identities:
            return {identities[value]}
        if value.startswith(("{", "[")):
            try:
                return _references(json.loads(value), identities)
            except ValueError:
                pass
    if isinstance(value, dict):
        return set().union(*(_references(child, identities) for child in value.values())) if value else set()
    if isinstance(value, list):
        return set().union(*(_references(child, identities) for child in value)) if value else set()
    return set()


def _partition(conn, profile_id, args):
    from .book_network import read_network_snapshot
    with read_network_snapshot(conn):
        return _partition_snapshot(conn, profile_id, args)


def _partition_snapshot(conn, profile_id, args):
    args = dict(args)
    import uuid
    if args.get("environment") not in ENVIRONMENTS:
        _error("Choose a supported target environment", code="validation")
    if args["environment"] == "regtest":
        try:
            args["chain_instance_id"] = str(uuid.UUID(str(args.get("chain_instance_id"))))
        except (ValueError, TypeError, AttributeError):
            _error("Regtest partitions need a local instance UUID", code="validation")
    elif args.get("chain_instance_id"):
        _error("Standard networks cannot use a local instance UUID", code="validation")
    inventory = inventory_book_network(conn, profile_id)
    selected = set(args.get("wallet_ids") or [])
    if not selected or not selected <= {row["wallet_id"] for row in inventory["wallets"]}:
        _error("Select connections from this book for the partition", code="validation")
    tables = _rows(conn, profile_id)
    entries = [(table, index, row) for table, rows in tables.items() for index, row in enumerate(rows)]
    parents = list(range(len(entries)))
    def root(index):
        while parents[index] != index:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index
    identities = {str(row["id"]): index for index, (table, _, row) in enumerate(entries) if row.get("id") and table not in SHARED}
    for index, (table, _, row) in enumerate(entries):
        if table in SHARED:
            continue
        for field, value in row.items():
            # Raw payloads are preserved as evidence, never interpreted as
            # authored ID references or rewritten during migration.
            if field == "raw_json":
                continue
            if field not in {"profile_id", "workspace_id", "account_id"}:
                for other in _references(value, identities):
                    parents[root(index)] = root(other)
    component_wallets = {}
    for index, (table, _, row) in enumerate(entries):
        if table == "wallets":
            component_wallets.setdefault(root(index), set()).add(row["id"])
    # Every retained accounting workflow requires this immutable enrollment.
    # Its ledger/evidence cannot be split by wallet without losing commitments.
    accounting_configured = conn.execute("SELECT 1 FROM gl_books WHERE profile_id=?", (profile_id,)).fetchone() is not None
    blockers = [{"code": "accounting_partition_unsupported"}] if accounting_configured else []
    for wallets in component_wallets.values():
        if wallets & selected and wallets - selected:
            blockers.append({"code": "relation_crosses_partition", "wallet_ids": sorted(wallets)})
    environment = args.get("environment")
    instance = args.get("chain_instance_id")
    declared = set(args.get("declared_wallet_ids") or [])
    if not declared <= selected:
        _error("Declare only selected partition connections", code="validation")
    for wallet in inventory["wallets"]:
        if wallet["wallet_id"] not in selected:
            continue
        if wallet["conflict_count"] or any(value != environment for value in wallet["environments"]) or any(value != instance for value in wallet["chain_instances"]):
            blockers.append({"code": "incompatible_wallet_history", "wallet_id": wallet["wallet_id"]})
        if wallet["requires_declaration"] and wallet["wallet_id"] not in declared:
            blockers.append({"code": "scope_declaration_required", "wallet_id": wallet["wallet_id"]})
    kept = {table: [] for table in tables}
    for index, (table, _, row) in enumerate(entries):
        if table in SHARED or component_wallets.get(root(index), set()) & selected:
            kept[table].append(row)
    # FK closure catches records that cannot be safely copied without history
    # deliberately retained only in the original book.
    kept_ids = {table: {str(row["id"]) for row in rows if row.get("id")} for table, rows in kept.items()}
    for table, rows in kept.items():
        fks = [dict(row) for row in conn.execute(f'PRAGMA foreign_key_list("{table}")')]
        for row in rows:
            for fk in fks:
                if fk["to"] == "id" and row.get(fk["from"]) and str(row[fk["from"]]) not in kept_ids.get(fk["table"], set()):
                    blockers.append({"code": "unresolved_relation", "table": table, "field": fk["from"]})
    recipe = {"profile_id": profile_id, "environment": environment, "chain_instance_id": instance, "wallet_ids": sorted(selected), "declared_wallet_ids": sorted(declared), "inventory_digest": inventory["inventory_digest"], "authored_digest": _digest(tables), "accounting_configured": accounting_configured}
    plan = {**recipe, "plan_id": _digest(recipe), "can_apply": not blockers, "blockers": blockers, "counts": {table: len(rows) for table, rows in kept.items() if rows}, "historical_reports_remain_in_original": True, "requires_source_reconnection": True}
    return plan, kept


def plan_network_partition(conn, profile_id, args):
    return _partition(conn, profile_id, args)[0]


def export_network_partition(conn, profile_id, args, *, data_root, output_path, recipient=None, backup_passphrase=None, db_passphrase=None):
    plan, kept = _partition(conn, profile_id, args)
    if plan["plan_id"] != args.get("plan_id"):
        _error("Network partition preview is stale", code="stale_context")
    if not plan["can_apply"]:
        _error("Network partition has unresolved relationships", code="book_network_review_required", details={"blockers": plan["blockers"]})
    if not isinstance(output_path, (str, Path)) or not str(output_path).strip():
        _error("Choose a backup destination", code="validation")
    destination = Path(output_path).expanduser().absolute()
    if destination.exists():
        _error("Choose a new destination file", code="conflict")
    # Validate encryption before creating plaintext staging. No network lookup.
    if (recipient is None) == (backup_passphrase is None):
        _error("Choose one backup passphrase or age recipient", code="validation")
    if recipient is not None and (not isinstance(recipient, str) or not recipient.startswith("age1")):
        _error("An age public recipient is required", code="validation")
    if backup_passphrase is not None and (not isinstance(backup_passphrase, str) or len(backup_passphrase) < 12):
        _error("Use a backup passphrase of at least 12 characters", code="validation")
    if conn.execute("PRAGMA cipher_version").fetchone() and not db_passphrase:
        _error("The unlocked project key is required for an encrypted partition", code="passphrase_required")
    with tempfile.TemporaryDirectory(prefix="kassiber-network-partition-") as stage:
        root = Path(stage)
        target = open_db(root, passphrase=db_passphrase)
        try:
            target.execute("PRAGMA foreign_keys=OFF")
            for table, rows in kept.items():
                for source in rows:
                    row = {
                        column: value
                        for column, value in dict(source).items()
                        if column not in RETIRED_DEVICE_SYNC_COLUMNS.get(table, ())
                    }
                    if table == "profiles":
                        row.update(last_processed_at=None, last_processed_tx_count=0, last_processed_input_version=0)
                    if table == "wallets":
                        row["config_json"] = json.dumps(public_wallet_config(row["config_json"]), sort_keys=True)
                    columns = list(row)
                    target.execute(f'INSERT INTO "{table}" ({",".join(chr(34)+column+chr(34) for column in columns)}) VALUES({",".join("?" for _ in columns)})', [row[column] for column in columns])
            profile = target.execute("SELECT * FROM profiles WHERE id=?", (profile_id,)).fetchone()
            target.execute("INSERT INTO settings(key,value) VALUES('context_profile',?)", (profile_id,))
            target.execute("INSERT INTO settings(key,value) VALUES('context_workspace',?)", (profile["workspace_id"],))
            binding_recipe = {key: plan[key] for key in ("environment", "chain_instance_id", "declared_wallet_ids")}
            binding_plan = plan_book_network(target, profile_id, binding_recipe)
            apply_book_network(target, profile_id, {**binding_recipe, "plan_id": binding_plan["plan_id"]})
            if target.execute("PRAGMA foreign_key_check").fetchone():
                _error("Partition does not preserve all required relationships", code="book_network_review_required")
            target.commit()
        finally:
            target.close()
        archive = root / "partition.tar"
        with tarfile.open(archive, "w") as tar:
            tar.add(root / "kassiber.sqlite3", arcname="kassiber.sqlite3")
            manifest = json.dumps({"schema_version": 1, "entries": {"database": "kassiber.sqlite3", "attachments_files": sum(bool(row.get("stored_relpath")) for row in kept.get("attachments", [])), "backends_env": False, "settings_json": False}, "notes": {"scope": "single_project_container", "inner_db_encrypted": bool(db_passphrase), "inner_db_passphrase_required": bool(db_passphrase)}, "network_partition": {"version": 1, **plan}}, sort_keys=True).encode()
            entry = tarfile.TarInfo("manifest.json")
            entry.size, entry.mode = len(manifest), 0o600
            tar.addfile(entry, io.BytesIO(manifest))
            attachments_root = Path(resolve_attachments_root(data_root))
            for attachment in kept.get("attachments", []):
                relpath = attachment.get("stored_relpath")
                if not relpath:
                    continue
                path, valid = _resolve_stored_path(attachments_root, relpath)
                if not valid or path is None or not path.is_file():
                    _error("Partition attachment is missing or unsafe", code="attachment_missing")
                staged_attachment = root / f"attachment-{attachment['id']}"
                shutil.copyfile(path, staged_attachment)
                _, content_hash = _hash_file(staged_attachment)
                if not attachment.get("sha256") or content_hash != attachment["sha256"]:
                    _error("Partition attachment no longer matches its stored hash", code="attachment_changed")
                tar.add(staged_attachment, arcname=f"attachments/{relpath}", recursive=False)
        temporary = None
        try:
            with archive.open("rb") as source, tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
                temporary = Path(output.name)
                encrypt_age_stream(source, output, **({"recipients": [recipient]} if recipient else {"passphrase": backup_passphrase}))
                output.flush()
                os.fsync(output.fileno())
            # Hard link is atomic and refuses an existing destination, including
            # one created after the preview. Never replace another user's file.
            os.link(temporary, destination)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    return {"file_path": str(destination), "plan_id": plan["plan_id"], "requires_journal_rebuild": True, "requires_source_reconnection": True}
