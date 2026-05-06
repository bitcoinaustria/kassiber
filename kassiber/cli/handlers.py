import argparse
import base64
import binascii
import json
import os
import sqlite3
import sys
import uuid
from collections import defaultdict
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
from functools import lru_cache
from pathlib import Path

from .. import __version__
from ..backends import (
    BACKEND_CLEAR_FIELD_ALIASES,
    BACKEND_KINDS,
    DEFAULT_ENV_FILENAME,
    DEFAULT_BACKENDS,
    _backend_row_to_dict,
    _validate_backend_kind,
    clear_default_backend,
    create_db_backend,
    delete_db_backend,
    get_db_backend,
    list_backends,
    list_db_backends,
    redact_backend_url,
    resolve_backend,
    set_default_backend,
    update_db_backend,
)
from ..core import accounts as core_accounts
from ..core import attachments as core_attachments
from ..core import imports as core_imports
from ..core import metadata as core_metadata
from ..core import pricing
from ..core import rates as core_rates
from ..core import reports as core_reports
from ..core import sync as core_sync
from ..core import sync_backends as core_sync_backends
from ..core import wallets as core_wallets
from ..core.engines import TaxEngineLedgerInputs, build_tax_engine
from ..core.repo import current_context_snapshot, resolve_account
from ..core.runtime import (
    build_status_payload,
)
from ..db import (
    APP_NAME,
    DEFAULT_DATA_ROOT,
    SCHEMA,
    ensure_column,
    ensure_schema_compat,
    get_setting,
    resolve_attachments_root,
    resolve_config_root,
    resolve_database_path,
    resolve_effective_data_root,
    resolve_effective_state_root,
    resolve_exports_root,
    resolve_settings_path,
    set_setting,
)
from ..envelope import (
    OUTPUT_FORMATS,
    build_envelope,
    derive_kind,
    emit,
    format_table_value,
    print_table,
)
from ..errors import AppError
from ..msat import (
    MSAT_PER_BTC,
    SATS_PER_BTC,
    btc_to_msat,
    dec,
    msat_to_btc,
)
from ..pdf_report import format_table, write_text_pdf
from ..time_utils import (
    UNKNOWN_OCCURRED_AT,
    _iso_z,
    _parse_iso_datetime,
    now_iso,
    timestamp_to_iso,
)
from ..util import (
    normalize_chain_value,
    normalize_network_value,
    parse_int,
)
from ..tax_policy import (
    DEFAULT_LONG_TERM_DAYS,
    DEFAULT_TAX_COUNTRY,
    build_tax_policy,
    require_tax_country_supported_for_profile_mutation,
    require_tax_processing_supported,
)
from ..wallet_descriptors import (
    DEFAULT_DESCRIPTOR_GAP_LIMIT,
    derive_descriptor_targets,
    liquid_plan_can_unblind,
    load_descriptor_plan,
    normalize_asset_code,
    normalize_chain,
    normalize_network,
)
from ..sync_btcpay import (
    DEFAULT_PAGE_SIZE as BTCPAY_DEFAULT_PAGE_SIZE,
    DEFAULT_PAYMENT_METHOD_ID as BTCPAY_DEFAULT_PAYMENT_METHOD_ID,
    fetch_btcpay_records,
)


ACCOUNT_TYPES = {"asset", "liability", "equity", "income", "expense"}
RP2_ACCOUNTING_METHODS = (
    "FIFO",
    "LIFO",
    "HIFO",
    "LOFO",
    "MOVING_AVERAGE",
    "MOVING_AVERAGE_AT",
)
WALLET_KINDS = [
    "descriptor",
    "xpub",
    "address",
    "coreln",
    "lnd",
    "nwc",
    "phoenix",
    "river",
    "custom",
]

def normalize_code(value):
    code = str(value).strip().lower().replace(" ", "-")
    if not code:
        raise AppError("Code cannot be empty")
    return code


def normalize_wallet_kind(value):
    kind = str(value).strip().lower()
    if kind not in WALLET_KINDS:
        raise AppError(f"Unsupported wallet kind '{value}'. Supported: {', '.join(WALLET_KINDS)}")
    return kind


def normalize_addresses(values):
    if values is None:
        return []
    if isinstance(values, str):
        values = [part.strip() for part in values.split(",")]
    output = []
    seen = set()
    for value in values:
        address = str(value).strip()
        if not address or address in seen:
            continue
        seen.add(address)
        output.append(address)
    return output


def resolve_workspace(conn, ref=None):
    ref = ref or get_setting(conn, "context_workspace")
    if not ref:
        raise AppError("No workspace selected. Create one or run `kassiber context set --workspace ...`.")
    row = conn.execute(
        "SELECT * FROM workspaces WHERE id = ? OR lower(label) = lower(?) LIMIT 1",
        (ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Workspace '{ref}' not found")
    return row


def resolve_profile(conn, workspace_id, ref=None):
    ref = ref or get_setting(conn, "context_profile")
    if not ref:
        raise AppError("No profile selected. Create one or run `kassiber context set --profile ...`.")
    row = conn.execute(
        """
        SELECT * FROM profiles
        WHERE workspace_id = ? AND (id = ? OR lower(label) = lower(?))
        LIMIT 1
        """,
        (workspace_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Profile '{ref}' not found in the selected workspace")
    return row


def resolve_scope(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    return workspace, profile


def resolve_wallet(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT w.*, a.code AS account_code, a.label AS account_label
        FROM wallets w
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE w.profile_id = ? AND (w.id = ? OR lower(w.label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Wallet '{ref}' not found")
    return row


def resolve_transaction(conn, profile_id, ref, direction=None):
    id_query = ["SELECT * FROM transactions WHERE profile_id = ? AND id = ?"]
    id_params = [profile_id, ref]
    if direction is not None:
        id_query.append("AND direction = ?")
        id_params.append(direction)
    row = conn.execute(" ".join(id_query), tuple(id_params)).fetchone()
    if row:
        return row

    external_query = ["SELECT * FROM transactions WHERE profile_id = ? AND external_id = ?"]
    external_params = [profile_id, ref]
    if direction is not None:
        external_query.append("AND direction = ?")
        external_params.append(direction)
    external_query.append("ORDER BY occurred_at DESC, created_at DESC, id DESC LIMIT 2")
    rows = conn.execute(" ".join(external_query), tuple(external_params)).fetchall()
    if len(rows) > 1:
        direction_hint = f" {direction}" if direction else ""
        raise AppError(
            f"Transaction external_id '{ref}' matches multiple{direction_hint} transactions",
            code="ambiguous_reference",
            hint="Use the Kassiber transaction id from `transactions list` or narrow the command to an unambiguous row.",
        )
    row = rows[0] if rows else None
    if not row:
        if direction is None:
            raise AppError(f"Transaction '{ref}' not found")
        raise AppError(f"{direction.capitalize()} transaction '{ref}' not found")
    return row


def resolve_tag(conn, profile_id, ref):
    row = conn.execute(
        """
        SELECT * FROM tags
        WHERE profile_id = ? AND (id = ? OR lower(code) = lower(?) OR lower(label) = lower(?))
        LIMIT 1
        """,
        (profile_id, ref, ref, ref),
    ).fetchone()
    if not row:
        raise AppError(f"Tag '{ref}' not found")
    return row


def invalidate_journals(conn, profile_id):
    conn.execute(
        "UPDATE profiles SET last_processed_at = NULL, last_processed_tx_count = 0 WHERE id = ?",
        (profile_id,),
    )


TRANSFER_PAIR_KINDS = ("manual", "peg-in", "peg-out", "submarine-swap")
TRANSFER_PAIR_POLICIES = ("carrying-value", "taxable")


def _pair_to_dict(row):
    return {
        "id": row["id"],
        "workspace_id": row["workspace_id"],
        "profile_id": row["profile_id"],
        "out_transaction_id": row["out_transaction_id"],
        "in_transaction_id": row["in_transaction_id"],
        "kind": row["kind"],
        "policy": row["policy"],
        "notes": row["notes"],
        "created_at": row["created_at"],
    }


def create_transaction_pair(
    conn,
    workspace_ref,
    profile_ref,
    out_ref,
    in_ref,
    kind="manual",
    policy="carrying-value",
    notes=None,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if kind not in TRANSFER_PAIR_KINDS:
        raise AppError(
            f"Unsupported pair kind '{kind}'. Supported: {', '.join(TRANSFER_PAIR_KINDS)}",
            code="validation",
        )
    if policy not in TRANSFER_PAIR_POLICIES:
        raise AppError(
            f"Unsupported pair policy '{policy}'. Supported: {', '.join(TRANSFER_PAIR_POLICIES)}",
            code="validation",
        )
    out_row = resolve_transaction(conn, profile["id"], out_ref, direction="outbound")
    in_row = resolve_transaction(conn, profile["id"], in_ref, direction="inbound")
    if out_row["id"] == in_row["id"]:
        raise AppError("--tx-out and --tx-in must reference different transactions", code="validation")
    if out_row["wallet_id"] == in_row["wallet_id"] and out_row["asset"] == in_row["asset"]:
        raise AppError(
            "Same-wallet pairs must be cross-asset swaps; same-asset legs should stay unpaired or use different wallets.",
            code="validation",
        )
    if out_row["asset"] == in_row["asset"] and policy == "taxable":
        raise AppError(
            f"Same-asset taxable pairs are not supported yet "
            f"(asset={out_row['asset']}). Leave the legs unpaired to keep "
            f"normal SELL + BUY treatment, or use --policy carrying-value "
            f"for a self-transfer.",
            code="validation",
            hint="Re-run with --policy carrying-value, or omit the pair entirely to preserve taxable SELL + BUY behavior.",
        )
    if out_row["asset"] != in_row["asset"] and policy == "carrying-value":
        tax_country = str(profile["tax_country"] or "").strip().lower()
        if tax_country != "at":
            raise AppError(
                f"Cross-asset carrying-value pairs are only supported for Austrian profiles right now "
                f"(out={out_row['asset']}, in={in_row['asset']}). "
                f"Use --policy taxable for other tax countries.",
                code="validation",
                hint="Re-run with --policy taxable, or use an Austrian profile for cross-asset carrying-value swaps.",
            )
    existing = conn.execute(
        """
        SELECT id FROM transaction_pairs
        WHERE profile_id = ? AND (out_transaction_id IN (?, ?) OR in_transaction_id IN (?, ?))
        LIMIT 1
        """,
        (profile["id"], out_row["id"], in_row["id"], out_row["id"], in_row["id"]),
    ).fetchone()
    if existing:
        raise AppError(
            f"One of the transactions is already paired (pair id={existing['id']}). "
            f"Run `kassiber transfers unpair --pair-id {existing['id']}` first.",
            code="conflict",
        )
    pair_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO transaction_pairs(
            id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
            kind, policy, notes, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            pair_id,
            workspace["id"],
            profile["id"],
            out_row["id"],
            in_row["id"],
            kind,
            policy,
            notes,
            now_iso(),
        ),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return _pair_to_dict(
        conn.execute("SELECT * FROM transaction_pairs WHERE id = ?", (pair_id,)).fetchone()
    )


def list_transaction_pairs(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            p.*,
            tout.external_id AS out_external_id,
            tout.asset AS out_asset,
            tout.amount AS out_amount_msat,
            wout.label AS out_wallet,
            tin.external_id AS in_external_id,
            tin.asset AS in_asset,
            tin.amount AS in_amount_msat,
            win.label AS in_wallet
        FROM transaction_pairs p
        JOIN transactions tout ON tout.id = p.out_transaction_id
        JOIN transactions tin ON tin.id = p.in_transaction_id
        JOIN wallets wout ON wout.id = tout.wallet_id
        JOIN wallets win ON win.id = tin.wallet_id
        WHERE p.profile_id = ?
        ORDER BY p.created_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        entry = _pair_to_dict(row)
        entry["out"] = {
            "transaction_id": row["out_transaction_id"],
            "external_id": row["out_external_id"] or "",
            "wallet": row["out_wallet"],
            "asset": row["out_asset"],
            "amount": float(msat_to_btc(row["out_amount_msat"])),
            "amount_msat": int(row["out_amount_msat"]),
        }
        entry["in"] = {
            "transaction_id": row["in_transaction_id"],
            "external_id": row["in_external_id"] or "",
            "wallet": row["in_wallet"],
            "asset": row["in_asset"],
            "amount": float(msat_to_btc(row["in_amount_msat"])),
            "amount_msat": int(row["in_amount_msat"]),
        }
        output.append(entry)
    return output


def delete_transaction_pair(conn, workspace_ref, profile_ref, pair_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        "SELECT * FROM transaction_pairs WHERE id = ? AND profile_id = ?",
        (pair_id, profile["id"]),
    ).fetchone()
    if not row:
        raise AppError(f"Pair '{pair_id}' not found", code="not_found")
    conn.execute("DELETE FROM transaction_pairs WHERE id = ?", (pair_id,))
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {"deleted": pair_id}


def init_app(conn):
    set_setting(conn, "app_version", __version__)
    conn.commit()


def read_text_argument(value, file_path, label):
    if value not in (None, ""):
        return str(value).strip()
    if not file_path:
        return None
    text = Path(file_path).expanduser().read_text(encoding="utf-8").strip()
    if not text:
        raise AppError(f"{label} file '{file_path}' is empty")
    return text


def wallet_live_chain_config(config):
    if not any(
        [
            config.get("descriptor"),
            config.get("change_descriptor"),
            config.get("addresses"),
            config.get("chain"),
            config.get("network"),
        ]
    ):
        return None, None
    chain = normalize_chain_value(config.get("chain"))
    network = normalize_network_value(chain, config.get("network"))
    return chain, network


def load_wallet_descriptor_plan_from_config(config):
    try:
        return load_descriptor_plan(config)
    except ValueError as exc:
        raise AppError(str(exc)) from exc


def parse_wallet_config(args):
    config = {}
    if getattr(args, "config", None):
        config.update(json.loads(args.config))
    if getattr(args, "config_file", None):
        with open(args.config_file, "r", encoding="utf-8") as handle:
            config.update(json.load(handle))
    if getattr(args, "backend", None):
        config["backend"] = args.backend.strip().lower()
    from ..secrets.cli_input import enforce_single_stdin_consumer, read_secret_from_args

    enforce_single_stdin_consumer(args, ("descriptor", "change_descriptor"))
    descriptor_text = read_secret_from_args(args, "descriptor")
    if descriptor_text is None:
        descriptor_text = read_text_argument(
            None,
            getattr(args, "descriptor_file", None),
            "Descriptor",
        )
    if descriptor_text:
        config["descriptor"] = descriptor_text
    change_descriptor_text = read_secret_from_args(
        args, "change-descriptor", legacy_attr="change_descriptor"
    )
    if change_descriptor_text is None:
        change_descriptor_text = read_text_argument(
            None,
            getattr(args, "change_descriptor_file", None),
            "Change descriptor",
        )
    if change_descriptor_text:
        config["change_descriptor"] = change_descriptor_text
    addresses = normalize_addresses(getattr(args, "address", None))
    existing_addresses = normalize_addresses(config.get("addresses"))
    if addresses or existing_addresses:
        config["addresses"] = normalize_addresses(existing_addresses + addresses)
    if getattr(args, "chain", None):
        config["chain"] = normalize_chain_value(args.chain)
    if getattr(args, "network", None):
        chain = normalize_chain_value(config.get("chain"))
        config["network"] = normalize_network_value(chain, args.network)
    if getattr(args, "gap_limit", None) is not None:
        if args.gap_limit <= 0:
            raise AppError("Descriptor gap limit must be positive")
        config["gap_limit"] = args.gap_limit
    if getattr(args, "policy_asset", None):
        config["policy_asset"] = normalize_asset_code(args.policy_asset)
    if getattr(args, "source_file", None):
        config["source_file"] = os.path.abspath(args.source_file)
    if getattr(args, "source_format", None):
        config["source_format"] = args.source_format
    chain, network = wallet_live_chain_config(config)
    if chain:
        config["chain"] = chain
        config["network"] = network
    return config


WALLET_KIND_CATALOG = {
    "descriptor": {
        "summary": "Output-descriptor wallet with optional change branch; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "change_descriptor", "gap_limit", "backend", "chain", "network", "policy_asset"],
        "requires": ["descriptor"],
    },
    "xpub": {
        "summary": "Extended-public-key wallet derived to address set; supports on-chain sync via mempool/esplora.",
        "config_fields": ["descriptor", "gap_limit", "backend", "chain", "network"],
        "requires": ["descriptor"],
    },
    "address": {
        "summary": "Bare-address list wallet; useful for receive-only tracking or imports.",
        "config_fields": ["addresses", "backend", "chain", "network", "source_file", "source_format"],
        "requires": ["addresses|source_file"],
    },
    "coreln": {
        "summary": "Core Lightning CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "lnd": {
        "summary": "LND CSV-derived wallet (deposits/withdrawals from node exports).",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "nwc": {
        "summary": "Nostr Wallet Connect wallet fed by CSV exports.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "phoenix": {
        "summary": "Phoenix Wallet CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "river": {
        "summary": "River Financial CSV importer.",
        "config_fields": ["source_file", "source_format"],
        "requires": [],
    },
    "custom": {
        "summary": "Custom CSV/JSON source; use with --config/--config-file to describe field mapping.",
        "config_fields": ["source_file", "source_format", "config"],
        "requires": ["source_file"],
    },
}


def import_into_wallet(conn, workspace_ref, profile_ref, wallet_ref, file_path, input_format):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    return _import_file_for_sync(conn, profile, wallet, file_path, input_format)


@lru_cache(maxsize=1)
def _metadata_hooks():
    return core_metadata.MetadataHooks(
        resolve_scope=resolve_scope,
        resolve_wallet=resolve_wallet,
        resolve_tag=resolve_tag,
        resolve_transaction=resolve_transaction,
        normalize_code=normalize_code,
        now_iso=now_iso,
        invalidate_journals=invalidate_journals,
        parse_iso_datetime=_parse_iso_datetime,
        iso_z=_iso_z,
        encode_cursor=_encode_event_cursor,
        decode_cursor=_decode_event_cursor,
    )


@lru_cache(maxsize=1)
def _attachment_hooks():
    return core_attachments.AttachmentHooks(
        resolve_scope=resolve_scope,
        resolve_transaction=resolve_transaction,
        now_iso=now_iso,
    )


@lru_cache(maxsize=1)
def _report_hooks():
    return core_reports.ReportHooks(
        resolve_scope=resolve_scope,
        resolve_account=resolve_account,
        resolve_wallet=resolve_wallet,
        require_processed_journals=require_processed_journals,
        build_ledger_state=build_ledger_state,
        list_journal_entries=list_journal_entries,
        list_wallets=core_wallets.list_wallets,
        parse_iso_datetime=_parse_iso_datetime,
        iso_z=_iso_z,
        now_iso=now_iso,
        format_table=format_table,
        write_text_pdf=write_text_pdf,
    )


@lru_cache(maxsize=1)
def _import_coordinator_hooks():
    return core_imports.ImportCoordinatorHooks(
        ensure_tag_row=lambda conn, workspace_id, profile_id, code, label: core_metadata.ensure_tag_row(
            conn,
            workspace_id,
            profile_id,
            code,
            label,
            _metadata_hooks(),
        ),
        invalidate_journals=invalidate_journals,
    )


def _import_file_for_sync(conn, profile, wallet, file_path, input_format, *, commit=True):
    return core_imports.import_file_into_wallet(
        conn,
        profile,
        wallet,
        file_path,
        input_format,
        _import_coordinator_hooks(),
        commit=commit,
    )


def _import_records_for_sync(
    conn,
    profile,
    wallet,
    records,
    source_label,
    *,
    apply_btcpay=False,
    apply_phoenix=False,
    commit=True,
):
    return core_imports.import_records_into_wallet(
        conn,
        profile,
        wallet,
        records,
        source_label,
        _import_coordinator_hooks(),
        apply_btcpay=apply_btcpay,
        apply_phoenix=apply_phoenix,
        commit=commit,
    )


def _insert_records_for_sync(conn, profile, wallet, records, source_label, *, commit=True):
    return _import_records_for_sync(
        conn,
        profile,
        wallet,
        records,
        source_label,
        commit=commit,
    )


def _wallet_sync_hooks(commit=True):
    return core_sync.WalletSyncHooks(
        import_file=lambda conn, profile, wallet, file_path, input_format: _import_file_for_sync(
            conn,
            profile,
            wallet,
            file_path,
            input_format,
            commit=commit,
        ),
        insert_records=lambda conn, profile, wallet, records, source_label: _insert_records_for_sync(
            conn,
            profile,
            wallet,
            records,
            source_label,
            commit=commit,
        ),
        resolve_backend=resolve_backend,
        resolve_sync_state=core_sync_backends.resolve_wallet_sync_targets,
        normalize_addresses=core_wallets.normalize_addresses,
        backend_adapters=core_sync_backends.SYNC_BACKEND_ADAPTERS,
        sync_btcpay_wallet=lambda conn, runtime_config, profile, wallet: sync_configured_btcpay_wallet(
            conn,
            runtime_config,
            profile,
            wallet,
            commit=commit,
        ),
    )


def sync_wallet_from_backend(conn, runtime_config, workspace_ref, profile_ref, wallet):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    return core_sync.sync_wallet_from_backend(
        conn,
        runtime_config,
        profile,
        wallet,
        _wallet_sync_hooks(),
    )


def sync_wallet(conn, runtime_config, workspace_ref, profile_ref, wallet_ref=None, sync_all=False):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if sync_all and wallet_ref:
        raise AppError("--wallet and --all are mutually exclusive", code="validation")
    if sync_all:
        wallets = conn.execute("SELECT * FROM wallets WHERE profile_id = ? ORDER BY label ASC", (profile["id"],)).fetchall()
        results = []
        for idx, wallet in enumerate(wallets):
            savepoint = f"wallet_sync_{idx}"
            conn.execute(f"SAVEPOINT {savepoint}")
            try:
                results.extend(
                    core_sync.sync_wallets(
                        conn,
                        runtime_config,
                        profile,
                        [wallet],
                        _wallet_sync_hooks(commit=False),
                    )
                )
            except AppError as exc:
                conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
                conn.execute(f"RELEASE SAVEPOINT {savepoint}")
                results.append(
                    {
                        "wallet": wallet["label"],
                        "status": "error",
                        "code": exc.code,
                        "message": str(exc),
                        "hint": exc.hint or "",
                        "retryable": bool(exc.retryable),
                    }
                )
            else:
                conn.execute(f"RELEASE SAVEPOINT {savepoint}")
                conn.commit()
        return results
    else:
        if not wallet_ref:
            raise AppError("Provide --wallet or use --all")
        wallets = [resolve_wallet(conn, profile["id"], wallet_ref)]
        return core_sync.sync_wallets(
            conn,
            runtime_config,
            profile,
            wallets,
            _wallet_sync_hooks(),
        )


def _sync_btcpay_wallet(
    conn,
    runtime_config,
    profile,
    wallet,
    *,
    page_size=BTCPAY_DEFAULT_PAGE_SIZE,
    commit=True,
):
    config = json.loads(wallet["config_json"] or "{}")
    btcpay_config = core_wallets.wallet_btcpay_sync_config(config)
    if btcpay_config is None:
        raise AppError(
            f"Wallet '{wallet['label']}' does not have BTCPay sync configured",
            code="validation",
            hint="Run `kassiber wallets sync-btcpay --wallet ... --backend ... --store-id ...` first, or store the config with `wallets update`.",
        )
    backend = resolve_backend(runtime_config, btcpay_config["backend"])
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token <api-key>`.",
        )
    records = fetch_btcpay_records(
        backend,
        store_id=btcpay_config["store_id"],
        payment_method_id=btcpay_config["payment_method_id"],
        page_size=page_size,
    )
    outcome = _import_records_for_sync(
        conn,
        profile,
        wallet,
        records,
        f"btcpay:{backend['name']}:{btcpay_config['store_id']}",
        apply_btcpay=True,
        commit=commit,
    )
    outcome["backend"] = backend["name"]
    outcome["backend_kind"] = kind
    outcome["backend_url"] = redact_backend_url(backend["url"])
    outcome["store_id"] = btcpay_config["store_id"]
    outcome["payment_method_id"] = btcpay_config["payment_method_id"]
    outcome["page_size"] = page_size
    outcome["fetched"] = len(records)
    return outcome


def sync_configured_btcpay_wallet(conn, runtime_config, profile, wallet, *, commit=True):
    return _sync_btcpay_wallet(
        conn,
        runtime_config,
        profile,
        wallet,
        page_size=BTCPAY_DEFAULT_PAGE_SIZE,
        commit=commit,
    )


def sync_btcpay_into_wallet(
    conn,
    runtime_config,
    workspace_ref,
    profile_ref,
    wallet_ref,
    backend_name,
    store_id,
    payment_method_id,
    page_size,
):
    store_id = core_wallets.normalize_btcpay_store_id(store_id)
    payment_method_id = core_wallets.normalize_btcpay_payment_method_id(
        payment_method_id
    )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    backend = resolve_backend(runtime_config, backend_name)
    kind = core_sync.normalize_backend_kind(backend["kind"])
    if kind != "btcpay":
        raise AppError(
            f"Backend '{backend['name']}' has kind '{backend['kind']}', expected 'btcpay'",
            code="validation",
            hint="Create a BTCPay backend with `kassiber backends create --kind btcpay --url <server> --token <api-key>`.",
        )
    core_wallets.update_wallet(
        conn,
        workspace_ref,
        profile_ref,
        wallet_ref,
        {
            "config": {
                "backend": backend_name,
                "store_id": store_id,
                "payment_method_id": payment_method_id,
                "sync_source": core_wallets.BTCPAY_SYNC_SOURCE,
            },
            "clear": [],
        },
    )
    wallet = resolve_wallet(conn, profile["id"], wallet["id"])
    return _sync_btcpay_wallet(
        conn,
        runtime_config,
        profile,
        wallet,
        page_size=page_size,
    )


def resolve_descriptor_branch_index(plan, branch):
    if branch in (None, "", "all"):
        return None
    normalized = str(branch).strip().lower()
    if normalized in {"0", "receive", "external"}:
        return 0
    if normalized in {"1", "change", "internal"}:
        return 1
    raise AppError("Descriptor branch must be one of: all, receive, change, 0, 1")


def derive_wallet_targets(conn, workspace_ref, profile_ref, wallet_ref, branch=None, start=0, count=None):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    wallet = resolve_wallet(conn, profile["id"], wallet_ref)
    config = json.loads(wallet["config_json"] or "{}")
    plan = load_wallet_descriptor_plan_from_config(config) if config.get("descriptor") else None
    if plan is None:
        raise AppError(f"Wallet '{wallet['label']}' does not have a descriptor configured")
    if start < 0:
        raise AppError("Descriptor derivation start must be non-negative")
    count = count if count is not None else plan.gap_limit
    if count <= 0:
        raise AppError("Descriptor derivation count must be positive")
    branch_index = resolve_descriptor_branch_index(plan, branch)
    return [
        core_sync_backends.sync_target_from_derived(target)
        for target in derive_descriptor_targets(
            plan,
            branch_index=branch_index,
            start=start,
            end=start + count,
        )
    ]


TRANSACTION_SORT_COLUMNS = {
    "occurred-at": "t.occurred_at",
    "amount": "t.amount",
    "fiat-value": "COALESCE(t.fiat_value, 0)",
    "fee": "t.fee",
}
MAX_TRANSACTION_PAGE_SIZE = 1000


def _transaction_cursor_filters(
    workspace_id,
    profile_id,
    wallet_id=None,
    direction=None,
    asset=None,
    start_ts=None,
    end_ts=None,
):
    return {
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "wallet_id": wallet_id or "",
        "direction": direction or "",
        "asset": asset.upper() if asset else "",
        "start": start_ts or "",
        "end": end_ts or "",
    }


def _decode_transaction_cursor(cursor, sort, order, filters):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        if payload.get("sort") != sort or payload.get("order") != order:
            raise ValueError("cursor sort/order mismatch")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        required = {"sort", "order", "filters", "value", "occurred_at", "created_at", "id"}
        if not required.issubset(payload):
            raise ValueError("missing cursor fields")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def _transaction_cursor_value(row, sort):
    if sort == "occurred-at":
        return row["occurred_at"]
    if sort == "amount":
        return int(row["amount"])
    if sort == "fee":
        return int(row["fee"])
    if sort == "fiat-value":
        return float(row["fiat_value"] or 0)
    raise AppError(
        f"Unsupported transaction sort: {sort}",
        code="validation",
        hint="Use one of: occurred-at, amount, fiat-value, fee.",
    )


def _encode_transaction_cursor(row, sort, order, filters):
    payload = {
        "sort": sort,
        "order": order,
        "filters": filters,
        "value": _transaction_cursor_value(row, sort),
        "occurred_at": row["occurred_at"],
        "created_at": row["_created_at"],
        "id": row["id"],
    }
    token = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def list_transactions(
    conn,
    workspace_ref,
    profile_ref,
    wallet_ref=None,
    limit=100,
    *,
    direction=None,
    asset=None,
    start=None,
    end=None,
    cursor=None,
    sort="occurred-at",
    order="desc",
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if limit > MAX_TRANSACTION_PAGE_SIZE:
        raise AppError(
            f"--limit cannot exceed {MAX_TRANSACTION_PAGE_SIZE}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_TRANSACTION_PAGE_SIZE}.",
        )
    if direction and direction not in {"inbound", "outbound"}:
        raise AppError("--direction must be inbound or outbound", code="validation")
    sort_column = TRANSACTION_SORT_COLUMNS.get(sort)
    if not sort_column:
        raise AppError(
            f"Unsupported transaction sort: {sort}",
            code="validation",
            hint="Use one of: occurred-at, amount, fiat-value, fee.",
        )
    if order not in {"asc", "desc"}:
        raise AppError("--order must be asc or desc", code="validation")
    order_sql = order.upper()
    if sort == "occurred-at":
        order_by = f"t.occurred_at {order_sql}, t.created_at {order_sql}, t.id {order_sql}"
    else:
        order_by = f"{sort_column} {order_sql}, t.occurred_at DESC, t.created_at DESC, t.id DESC"

    params = [profile["id"]]
    filters = ["t.profile_id = ?"]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None
    wallet_id = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_id = wallet["id"]
        filters.append("t.wallet_id = ?")
        params.append(wallet_id)
    if direction:
        filters.append("t.direction = ?")
        params.append(direction)
    if asset:
        filters.append("upper(t.asset) = ?")
        params.append(asset.upper())
    if start_ts:
        filters.append("t.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        filters.append("t.occurred_at <= ?")
        params.append(end_ts)

    cursor_filters = _transaction_cursor_filters(
        workspace["id"],
        profile["id"],
        wallet_id,
        direction,
        asset,
        start_ts,
        end_ts,
    )
    cursor_data = _decode_transaction_cursor(cursor, sort, order, cursor_filters)
    if cursor_data:
        if sort == "occurred-at":
            op = ">" if order == "asc" else "<"
            filters.append(
                f"(t.occurred_at {op} ? OR "
                f"(t.occurred_at = ? AND t.created_at {op} ?) OR "
                f"(t.occurred_at = ? AND t.created_at = ? AND t.id {op} ?))"
            )
            params.extend(
                [
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
        else:
            primary_op = ">" if order == "asc" else "<"
            filters.append(
                f"({sort_column} {primary_op} ? OR "
                f"({sort_column} = ? AND "
                "(t.occurred_at < ? OR "
                "(t.occurred_at = ? AND t.created_at < ?) OR "
                "(t.occurred_at = ? AND t.created_at = ? AND t.id < ?))))"
            )
            params.extend(
                [
                    cursor_data["value"],
                    cursor_data["value"],
                    cursor_data["occurred_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["occurred_at"],
                    cursor_data["created_at"],
                    cursor_data["id"],
                ]
            )
    params.append(limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            t.id,
            COALESCE(t.external_id, '') AS external_id,
            t.occurred_at,
            t.confirmed_at,
            t.created_at AS _created_at,
            w.label AS wallet,
            t.direction,
            t.asset,
            t.amount,
            t.fee,
            COALESCE(t.fiat_rate, 0) AS fiat_rate,
            COALESCE(t.fiat_value, 0) AS fiat_value,
            t.fiat_rate_exact,
            t.fiat_value_exact,
            t.fiat_price_source,
            t.pricing_source_kind,
            t.pricing_provider,
            t.pricing_pair,
            t.pricing_timestamp,
            t.pricing_fetched_at,
            t.pricing_granularity,
            t.pricing_method,
            t.pricing_external_ref,
            t.pricing_quality,
            COALESCE(t.kind, '') AS kind,
            COALESCE(t.description, '') AS description,
            COALESCE(t.note, '') AS note,
            t.excluded
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        WHERE {' AND '.join(filters)}
        ORDER BY {order_by}
        LIMIT ?
        """,
        params,
    ).fetchall()
    has_more = len(rows) > limit
    page = rows[:limit]
    tags_by_transaction = {row["id"]: [] for row in page}
    if page:
        placeholders = ", ".join("?" for _ in page)
        for tag in conn.execute(
            f"""
            SELECT tt.transaction_id, tags.code, tags.label
            FROM transaction_tags tt
            JOIN tags ON tags.id = tt.tag_id
            WHERE tt.transaction_id IN ({placeholders})
            ORDER BY tt.transaction_id ASC, tags.code ASC
            """,
            [row["id"] for row in page],
        ).fetchall():
            tags_by_transaction.setdefault(tag["transaction_id"], []).append(
                {"code": tag["code"], "label": tag["label"]}
            )
    results = []
    for row in page:
        record = dict(row)
        record.pop("_created_at", None)
        record["amount_msat"] = int(record["amount"])
        record["amount"] = float(msat_to_btc(record["amount"]))
        record["fee_msat"] = int(record["fee"])
        record["fee"] = float(msat_to_btc(record["fee"]))
        record["excluded"] = bool(record["excluded"])
        record["tags"] = tags_by_transaction.get(record["id"], [])
        results.append(record)
    next_cursor = _encode_transaction_cursor(page[-1], sort, order, cursor_filters) if has_more and page else None
    return results, {
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": limit,
        "sort": sort,
        "order": order,
    }


def available_quantity(lots):
    total = Decimal("0")
    for lot in lots:
        total += lot["quantity"]
    return total


def consume_lots(lots, quantity, algorithm):
    remaining = dec(quantity)
    cost_basis = Decimal("0")
    while remaining > 0:
        if not lots:
            raise AppError("Not enough lots to consume")
        lot = lots[0] if algorithm == "FIFO" else lots[-1]
        take = min(remaining, lot["quantity"])
        cost_basis += take * lot["unit_cost"]
        lot["quantity"] -= take
        remaining -= take
        if lot["quantity"] <= Decimal("0"):
            if algorithm == "FIFO":
                lots.pop(0)
            else:
                lots.pop()
    return cost_basis


def latest_rates_for_profile(conn, profile_id):
    rows = conn.execute(
        """
        SELECT asset, fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount
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
        rate = pricing.decimal_from_exact(row["fiat_rate_exact"], row["fiat_rate"])
        value = pricing.decimal_from_exact(row["fiat_value_exact"], row["fiat_value"])
        if rate is not None:
            rates[asset] = rate
        elif value is not None and row["amount"]:
            rates[asset] = value / msat_to_btc(row["amount"])
    return rates


def auto_price_transactions_from_rates_cache(conn, profile):
    tx_rows = conn.execute(
        """
        SELECT id, occurred_at, asset, amount, fiat_currency, fiat_rate, fiat_value,
               fiat_rate_exact, fiat_value_exact, fiat_price_source,
               pricing_source_kind, pricing_quality, confirmed_at
        FROM transactions
        WHERE profile_id = ? AND excluded = 0
          AND (
            (
              fiat_rate IS NULL AND fiat_value IS NULL
              AND fiat_rate_exact IS NULL AND fiat_value_exact IS NULL
            )
            OR (
              fiat_price_source = ?
              AND pricing_source_kind IS NULL
              AND pricing_quality IS NULL
            )
          )
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        """,
        (profile["id"], pricing.LEGACY_SOURCE_RATES_CACHE),
    ).fetchall()
    auto_priced = 0
    for row in tx_rows:
        price_was_missing = (
            row["fiat_rate"] is None
            and row["fiat_value"] is None
            and row["fiat_rate_exact"] is None
            and row["fiat_value_exact"] is None
        )
        pair = core_rates.transaction_rate_pair(row["asset"], row["fiat_currency"] or profile["fiat_currency"])
        if pair is None:
            continue
        pricing_at = row["confirmed_at"] or row["occurred_at"]
        cached_rate = core_rates.get_cached_rate_at_or_before(conn, pair, pricing_at)
        if cached_rate is None:
            continue
        rate = pricing.decimal_from_exact(
            row["fiat_rate_exact"],
            cached_rate.get("rate_exact"),
            row["fiat_rate"],
            cached_rate["rate"],
        )
        fiat_value = pricing.decimal_from_exact(row["fiat_value_exact"])
        if fiat_value is None and rate is not None and row["amount"] > 0:
            fiat_value = rate * msat_to_btc(row["amount"])
        if fiat_value is None:
            fiat_value = pricing.decimal_from_exact(row["fiat_value"])
        if rate is None and fiat_value is not None and row["amount"] > 0:
            rate = fiat_value / msat_to_btc(row["amount"])
        source_kind = pricing.rate_cache_source_kind(cached_rate)
        quality = pricing.rate_cache_quality(cached_rate)
        payload = pricing.pricing_payload(
            rate=rate,
            value=fiat_value,
            source_kind=source_kind,
            quality=quality,
            provider=cached_rate["source"],
            pair=cached_rate["pair"],
            pricing_timestamp=cached_rate["timestamp"],
            fetched_at=cached_rate["fetched_at"],
            granularity=cached_rate.get("granularity"),
            method=cached_rate.get("method"),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
                fiat_rate_exact = ?, fiat_value_exact = ?,
                pricing_source_kind = ?, pricing_provider = ?, pricing_pair = ?,
                pricing_timestamp = ?, pricing_fetched_at = ?,
                pricing_granularity = ?, pricing_method = ?,
                pricing_external_ref = ?, pricing_quality = ?
            WHERE id = ?
            """,
            (
                payload["fiat_rate"],
                payload["fiat_value"],
                payload["fiat_price_source"],
                payload["fiat_rate_exact"],
                payload["fiat_value_exact"],
                payload["pricing_source_kind"],
                payload["pricing_provider"],
                payload["pricing_pair"],
                payload["pricing_timestamp"],
                payload["pricing_fetched_at"],
                payload["pricing_granularity"],
                payload["pricing_method"],
                payload["pricing_external_ref"],
                payload["pricing_quality"],
                row["id"],
            ),
        )
        if price_was_missing:
            auto_priced += 1
    return auto_priced


def build_ledger_state(conn, profile):
    require_tax_processing_supported(profile)
    rows = conn.execute(
        """
        SELECT
            t.*,
            w.label AS wallet_label,
            w.kind AS wallet_kind,
            w.account_id AS wallet_account_id,
            w.config_json AS config_json,
            COALESCE(a.code, 'treasury') AS account_code,
            COALESCE(a.label, 'Treasury') AS account_label
        FROM transactions t
        JOIN wallets w ON w.id = t.wallet_id
        LEFT JOIN accounts a ON a.id = w.account_id
        WHERE t.profile_id = ? AND t.excluded = 0
        ORDER BY t.occurred_at ASC, t.created_at ASC, t.id ASC
        """,
        (profile["id"],),
    ).fetchall()
    manual_pair_records = conn.execute(
        "SELECT * FROM transaction_pairs WHERE profile_id = ?",
        (profile["id"],),
    ).fetchall()
    tax_engine = build_tax_engine(profile)
    rates = latest_rates_for_profile(conn, profile["id"])
    wallet_refs_by_id = {}
    for row in rows:
        wallet_config = json.loads(row["config_json"] or "{}")
        wallet_refs_by_id[row["wallet_id"]] = {
            "id": row["wallet_id"],
            "label": row["wallet_label"],
            "wallet_account_id": row["wallet_account_id"],
            "account_code": row["account_code"],
            "account_label": row["account_label"],
        }
    engine_state = tax_engine.build_ledger_state(
        TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pair_records,
        )
    )
    return {
        "entries": engine_state.entries,
        "quarantines": engine_state.quarantines,
        "intra_audit": engine_state.intra_audit,
        "cross_asset_pairs": engine_state.cross_asset_pairs,
        "tax_summary": engine_state.tax_summary,
        "account_holdings": engine_state.account_holdings,
        "wallet_holdings": engine_state.wallet_holdings,
        "latest_rates": rates,
    }


def process_journals(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    auto_priced = auto_price_transactions_from_rates_cache(conn, profile)
    state = build_ledger_state(conn, profile)
    conn.execute("DELETE FROM journal_entries WHERE profile_id = ?", (profile["id"],))
    conn.execute("DELETE FROM journal_quarantines WHERE profile_id = ?", (profile["id"],))
    created_at = now_iso()
    pricing_by_tx = {
        row["id"]: row
        for row in conn.execute(
            """
            SELECT id, pricing_source_kind, pricing_quality
            FROM transactions
            WHERE profile_id = ?
            """,
            (profile["id"],),
        ).fetchall()
    }
    for entry in state["entries"]:
        exact_payload = pricing.journal_exact_payload(entry)
        tx_pricing = pricing_by_tx.get(entry["transaction_id"])
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, fiat_value_exact, unit_cost_exact,
                cost_basis_exact, proceeds_exact, gain_loss_exact, pricing_source_kind,
                pricing_quality, description, at_category, at_kennzahl, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                entry["id"],
                entry["workspace_id"],
                entry["profile_id"],
                entry["transaction_id"],
                entry["wallet_id"],
                entry["account_id"],
                entry["occurred_at"],
                entry["entry_type"],
                entry["asset"],
                btc_to_msat(entry["quantity"]),
                float(entry["fiat_value"]),
                float(entry["unit_cost"]),
                float(entry["cost_basis"]) if entry["cost_basis"] is not None else None,
                float(entry["proceeds"]) if entry["proceeds"] is not None else None,
                float(entry["gain_loss"]) if entry["gain_loss"] is not None else None,
                exact_payload["fiat_value_exact"],
                exact_payload["unit_cost_exact"],
                exact_payload["cost_basis_exact"],
                exact_payload["proceeds_exact"],
                exact_payload["gain_loss_exact"],
                tx_pricing["pricing_source_kind"] if tx_pricing else None,
                tx_pricing["pricing_quality"] if tx_pricing else None,
                entry["description"],
                entry.get("at_category"),
                entry.get("at_kennzahl"),
                created_at,
            ),
        )
    for quarantine in state["quarantines"]:
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                quarantine["transaction_id"],
                quarantine["workspace_id"],
                quarantine["profile_id"],
                quarantine["reason"],
                quarantine["detail_json"],
                created_at,
            ),
        )
    tx_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    conn.execute(
        "UPDATE profiles SET last_processed_at = ?, last_processed_tx_count = ? WHERE id = ?",
        (created_at, tx_count, profile["id"]),
    )
    conn.commit()
    result = {
        "profile": profile["label"],
        "entries_created": len(state["entries"]),
        "quarantined": len(state["quarantines"]),
        "transfers_detected": len(state.get("intra_audit", [])),
        "cross_asset_pairs": len(state.get("cross_asset_pairs", [])),
        "auto_priced": auto_priced,
        "processed_transactions": tx_count,
        "processed_at": created_at,
    }
    return result


def _journal_processing_status(conn, profile):
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    return {
        "last_processed_at": profile["last_processed_at"],
        "last_processed_tx_count": int(profile["last_processed_tx_count"] or 0),
        "current_active_tx_count": int(current_count or 0),
        "processed_journals_current": bool(
            profile["last_processed_at"] and current_count == profile["last_processed_tx_count"]
        ),
    }


def _audit_transaction_refs(conn, profile_id, transaction_ids):
    ids = list(dict.fromkeys(str(value) for value in transaction_ids if value))
    if not ids:
        return {}
    rows = []
    chunk_size = 400
    for start in range(0, len(ids), chunk_size):
        chunk = ids[start : start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows.extend(
            conn.execute(
                f"""
                SELECT
                    t.id,
                    t.external_id,
                    t.occurred_at,
                    t.asset,
                    w.label AS wallet
                FROM transactions t
                JOIN wallets w ON w.id = t.wallet_id
                WHERE t.profile_id = ? AND t.id IN ({placeholders})
                """,
                [profile_id, *chunk],
            ).fetchall()
        )
    return {str(row["id"]): dict(row) for row in rows}


def _serialize_intra_audit(rows):
    return [
        {
            "out_id": row["out_id"],
            "in_id": row["in_id"],
            "external_id": row["external_id"],
            "occurred_at": row["occurred_at"],
            "asset": row["asset"],
            "from_wallet": row["from_wallet_label"],
            "to_wallet": row["to_wallet_label"],
            "sent": float(dec(row["crypto_sent"])),
            "sent_msat": btc_to_msat(dec(row["crypto_sent"])),
            "received": float(dec(row["crypto_received"])),
            "received_msat": btc_to_msat(dec(row["crypto_received"])),
            "fee": float(dec(row["crypto_fee"])),
            "fee_msat": btc_to_msat(dec(row["crypto_fee"])),
            "spot_price": float(dec(row["spot_price"])),
        }
        for row in sorted(
            rows,
            key=lambda item: (item["occurred_at"], item["out_id"], item["in_id"]),
        )
    ]


def _serialize_cross_asset_pairs(rows, refs_by_id):
    serialized = []
    for row in sorted(
        rows,
        key=lambda item: (
            refs_by_id.get(str(item["out_id"]), {}).get("occurred_at", ""),
            str(item.get("pair_id") or ""),
            str(item["out_id"]),
            str(item["in_id"]),
        ),
    ):
        out_ref = refs_by_id.get(str(row["out_id"]), {})
        in_ref = refs_by_id.get(str(row["in_id"]), {})
        serialized.append(
            {
                "pair_id": row.get("pair_id"),
                "kind": row.get("kind"),
                "policy": row.get("policy"),
                "out_id": row["out_id"],
                "out_asset": row["out_asset"],
                "out_wallet": out_ref.get("wallet"),
                "out_external_id": out_ref.get("external_id"),
                "out_occurred_at": out_ref.get("occurred_at"),
                "in_id": row["in_id"],
                "in_asset": row["in_asset"],
                "in_wallet": in_ref.get("wallet"),
                "in_external_id": in_ref.get("external_id"),
                "in_occurred_at": in_ref.get("occurred_at"),
            }
        )
    return serialized


def inspect_transfer_audit(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    require_tax_processing_supported(profile)
    state = build_ledger_state(conn, profile)
    tx_refs = _audit_transaction_refs(
        conn,
        profile["id"],
        [row["out_id"] for row in state["cross_asset_pairs"]]
        + [row["in_id"] for row in state["cross_asset_pairs"]],
    )
    intra_transfers = _serialize_intra_audit(state["intra_audit"])
    cross_asset_pairs = _serialize_cross_asset_pairs(state["cross_asset_pairs"], tx_refs)
    return {
        "profile": profile["label"],
        "processing": _journal_processing_status(conn, profile),
        "summary": {
            "same_asset_transfers": len(intra_transfers),
            "cross_asset_pairs": len(cross_asset_pairs),
            "quarantines": len(state["quarantines"]),
        },
        "same_asset_transfers": intra_transfers,
        "cross_asset_pairs": cross_asset_pairs,
    }


DEFAULT_EVENTS_LIMIT = 100
MAX_EVENTS_LIMIT = 1000


def _journal_cursor_filters(
    workspace_id,
    profile_id,
    wallet_id=None,
    account_id=None,
    asset=None,
    entry_type=None,
    start_ts=None,
    end_ts=None,
):
    return {
        "workspace_id": workspace_id,
        "profile_id": profile_id,
        "wallet_id": wallet_id or "",
        "account_id": account_id or "",
        "asset": asset.upper() if asset else "",
        "entry_type": entry_type.lower() if entry_type else "",
        "start": start_ts or "",
        "end": end_ts or "",
    }


def _cursor_created_at(row):
    if "created_at" in row.keys():
        return row["created_at"]
    return row["_created_at"]


def _encode_event_cursor(row, filters):
    payload = {
        "filters": filters,
        "occurred_at": row["occurred_at"],
        "created_at": _cursor_created_at(row),
        "id": row["id"],
    }
    token = json.dumps(payload, separators=(",", ":"), sort_keys=True)
    return base64.urlsafe_b64encode(token.encode("utf-8")).decode("ascii").rstrip("=")


def _decode_event_cursor(cursor, filters):
    if not cursor:
        return None
    try:
        padding = "=" * (-len(cursor) % 4)
        decoded = base64.urlsafe_b64decode(cursor + padding).decode("utf-8")
        payload = json.loads(decoded)
        required = {"filters", "occurred_at", "created_at", "id"}
        if not required.issubset(payload):
            raise ValueError("missing cursor fields")
        if payload.get("filters") != filters:
            raise ValueError("cursor filter mismatch")
        return payload
    except (ValueError, UnicodeDecodeError, binascii.Error, json.JSONDecodeError) as exc:
        raise AppError(
            f"Invalid cursor: {cursor}",
            code="validation",
            hint="Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        ) from exc


def list_journal_events(
    conn,
    workspace_ref,
    profile_ref,
    wallet=None,
    account=None,
    asset=None,
    entry_type=None,
    start=None,
    end=None,
    cursor=None,
    limit=None,
):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    effective_limit = limit if limit is not None else DEFAULT_EVENTS_LIMIT
    if effective_limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    if effective_limit > MAX_EVENTS_LIMIT:
        raise AppError(
            f"--limit cannot exceed {MAX_EVENTS_LIMIT}",
            code="validation",
            hint=f"Use cursor-based pagination instead of larger limits; max page size is {MAX_EVENTS_LIMIT}.",
        )

    where = ["je.profile_id = ?"]
    params = [profile["id"]]
    start_ts = _iso_z(_parse_iso_datetime(start, "start")) if start else None
    end_ts = _iso_z(_parse_iso_datetime(end, "end")) if end else None

    wallet_id = ""
    account_id = ""
    if wallet:
        wallet_row = resolve_wallet(conn, profile["id"], wallet)
        wallet_id = wallet_row["id"]
        where.append("je.wallet_id = ?")
        params.append(wallet_id)
    if account:
        account_row = resolve_account(conn, profile["id"], account)
        account_id = account_row["id"]
        where.append("je.account_id = ?")
        params.append(account_id)
    if asset:
        where.append("upper(je.asset) = ?")
        params.append(asset.upper())
    if entry_type:
        where.append("lower(je.entry_type) = ?")
        params.append(entry_type.lower())
    if start_ts:
        where.append("je.occurred_at >= ?")
        params.append(start_ts)
    if end_ts:
        where.append("je.occurred_at <= ?")
        params.append(end_ts)

    cursor_filters = _journal_cursor_filters(
        workspace["id"],
        profile["id"],
        wallet_id,
        account_id,
        asset,
        entry_type,
        start_ts,
        end_ts,
    )
    cursor_data = _decode_event_cursor(cursor, cursor_filters)
    if cursor_data:
        where.append(
            "(je.occurred_at < ? OR "
            "(je.occurred_at = ? AND je.created_at < ?) OR "
            "(je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )

    query = f"""
        SELECT
            je.id,
            je.occurred_at,
            je.created_at,
            je.transaction_id,
            je.wallet_id,
            w.label AS wallet,
            je.account_id,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            je.unit_cost,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            je.fiat_value_exact,
            je.unit_cost_exact,
            je.cost_basis_exact,
            je.proceeds_exact,
            je.gain_loss_exact,
            je.pricing_source_kind,
            je.pricing_quality,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE {' AND '.join(where)}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        LIMIT ?
    """
    params.append(effective_limit + 1)
    rows = conn.execute(query, params).fetchall()

    has_more = len(rows) > effective_limit
    page = rows[:effective_limit]
    events = []
    for row in page:
        event = dict(row)
        event["quantity_msat"] = int(event["quantity"])
        event["quantity"] = float(msat_to_btc(event["quantity"]))
        if event.get("at_category") is None:
            event.pop("at_category", None)
        if event.get("at_kennzahl") is None:
            event.pop("at_kennzahl", None)
        events.append(event)
    next_cursor = _encode_event_cursor(page[-1], cursor_filters) if has_more and page else None

    return {
        "events": events,
        "next_cursor": next_cursor,
        "has_more": has_more,
        "limit": effective_limit,
    }


def get_journal_event(conn, workspace_ref, profile_ref, event_id):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    row = conn.execute(
        """
        SELECT
            je.*,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            COALESCE(a.label, '') AS account_label,
            t.external_id AS transaction_external_id,
            t.direction AS transaction_direction,
            t.counterparty AS transaction_counterparty,
            t.note AS transaction_note
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        LEFT JOIN transactions t ON t.id = je.transaction_id
        WHERE je.profile_id = ? AND je.id = ?
        """,
        (profile["id"], event_id),
    ).fetchone()
    if not row:
        raise AppError(
            f"Journal event '{event_id}' not found",
            code="not_found",
            hint="Run `kassiber journals events list` to find valid event ids.",
        )
    event = dict(row)
    event["quantity_msat"] = int(event["quantity"])
    event["quantity"] = float(msat_to_btc(event["quantity"]))
    return event


def list_journal_entries(conn, workspace_ref, profile_ref, limit=200, cursor=None, return_meta=False):
    workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    if limit is not None and limit <= 0:
        raise AppError("--limit must be positive", code="validation")
    limit_clause = ""
    params = [profile["id"]]
    cursor_clause = ""
    cursor_filters = _journal_cursor_filters(workspace["id"], profile["id"])
    cursor_data = _decode_event_cursor(cursor, cursor_filters)
    if cursor_data:
        cursor_clause = (
            "AND (je.occurred_at < ? OR "
            "(je.occurred_at = ? AND je.created_at < ?) OR "
            "(je.occurred_at = ? AND je.created_at = ? AND je.id < ?))"
        )
        params.extend(
            [
                cursor_data["occurred_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["occurred_at"],
                cursor_data["created_at"],
                cursor_data["id"],
            ]
        )
    if limit is not None:
        limit_clause = "LIMIT ?"
        params.append(limit + 1)
    rows = conn.execute(
        f"""
        SELECT
            je.id,
            je.occurred_at,
            je.created_at AS _created_at,
            w.label AS wallet,
            COALESCE(a.code, '') AS account,
            je.entry_type,
            je.asset,
            je.quantity,
            je.fiat_value,
            COALESCE(je.cost_basis, 0) AS cost_basis,
            COALESCE(je.proceeds, 0) AS proceeds,
            COALESCE(je.gain_loss, 0) AS gain_loss,
            COALESCE(je.description, '') AS description,
            je.at_category,
            je.at_kennzahl
        FROM journal_entries je
        JOIN wallets w ON w.id = je.wallet_id
        LEFT JOIN accounts a ON a.id = je.account_id
        WHERE je.profile_id = ? {cursor_clause}
        ORDER BY je.occurred_at DESC, je.created_at DESC, je.id DESC
        {limit_clause}
        """,
        params,
    ).fetchall()
    has_more = bool(limit is not None and len(rows) > limit)
    page = rows[:limit] if limit is not None else rows
    results = []
    for row in page:
        entry = dict(row)
        entry.pop("_created_at", None)
        entry["quantity_msat"] = int(entry["quantity"])
        entry["quantity"] = float(msat_to_btc(entry["quantity"]))
        if entry.get("at_category") is None:
            entry.pop("at_category", None)
        if entry.get("at_kennzahl") is None:
            entry.pop("at_kennzahl", None)
        results.append(entry)
    meta = {
        "next_cursor": _encode_event_cursor(
            {
                "occurred_at": page[-1]["occurred_at"],
                "created_at": page[-1]["_created_at"],
                "id": page[-1]["id"],
            },
            cursor_filters,
        )
        if has_more and page
        else None,
        "has_more": has_more,
        "limit": limit,
    }
    return (results, meta) if return_meta else results


def list_quarantines(conn, workspace_ref, profile_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    rows = conn.execute(
        """
        SELECT
            q.transaction_id,
            t.external_id,
            t.occurred_at,
            t.confirmed_at,
            w.label AS wallet,
            t.asset,
            t.amount,
            t.fee,
            q.reason,
            q.detail_json
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ?
        ORDER BY t.occurred_at DESC
        """,
        (profile["id"],),
    ).fetchall()
    output = []
    for row in rows:
        detail = json.loads(row["detail_json"] or "{}")
        output.append(
            {
                "transaction_id": row["transaction_id"],
                "external_id": row["external_id"] or "",
                "occurred_at": row["occurred_at"],
                "confirmed_at": row["confirmed_at"],
                "wallet": row["wallet"],
                "asset": row["asset"],
                "amount": float(msat_to_btc(row["amount"])),
                "amount_msat": int(row["amount"]),
                "fee": float(msat_to_btc(row["fee"])),
                "fee_msat": int(row["fee"]),
                "reason": row["reason"],
                "detail": detail,
            }
        )
    return output


def show_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    row = conn.execute(
        """
        SELECT q.transaction_id, q.reason, q.detail_json, q.created_at,
               w.label AS wallet, t.external_id, t.occurred_at, t.confirmed_at, t.asset,
               t.amount, t.fee, t.fiat_rate, t.fiat_value, t.fiat_rate_exact,
               t.fiat_value_exact, t.pricing_source_kind, t.pricing_provider,
               t.pricing_pair, t.pricing_timestamp, t.pricing_granularity,
               t.pricing_method, t.pricing_quality, t.direction, t.excluded
        FROM journal_quarantines q
        JOIN transactions t ON t.id = q.transaction_id
        JOIN wallets w ON w.id = t.wallet_id
        WHERE q.profile_id = ? AND q.transaction_id = ?
        """,
        (profile["id"], tx["id"]),
    ).fetchone()
    if not row:
        raise AppError(
            f"Transaction '{tx_ref}' has no active quarantine",
            code="not_found",
            hint="Only transactions flagged during `journals process` appear here.",
        )
    return {
        "transaction_id": row["transaction_id"],
        "external_id": row["external_id"] or "",
        "wallet": row["wallet"],
        "occurred_at": row["occurred_at"],
        "confirmed_at": row["confirmed_at"],
        "direction": row["direction"],
        "asset": row["asset"],
        "amount": float(msat_to_btc(row["amount"])),
        "amount_msat": int(row["amount"]),
        "fee": float(msat_to_btc(row["fee"])),
        "fee_msat": int(row["fee"]),
        "fiat_rate": row["fiat_rate"],
        "fiat_value": row["fiat_value"],
        "fiat_rate_exact": row["fiat_rate_exact"],
        "fiat_value_exact": row["fiat_value_exact"],
        "pricing": {
            "source_kind": row["pricing_source_kind"],
            "provider": row["pricing_provider"],
            "pair": row["pricing_pair"],
            "timestamp": row["pricing_timestamp"],
            "granularity": row["pricing_granularity"],
            "method": row["pricing_method"],
            "quality": row["pricing_quality"],
        },
        "excluded": bool(row["excluded"]),
        "reason": row["reason"],
        "detail": json.loads(row["detail_json"] or "{}"),
        "quarantined_at": row["created_at"],
    }


def _ensure_quarantined(conn, profile_id, transaction_id):
    row = conn.execute(
        "SELECT reason FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile_id, transaction_id),
    ).fetchone()
    if not row:
        raise AppError(
            "Transaction is not quarantined",
            code="not_found",
            hint="Run `kassiber journals quarantined` to see active entries.",
        )
    return row["reason"]


def resolve_quarantine_price_override(
    conn, workspace_ref, profile_ref, tx_ref, fiat_rate=None, fiat_value=None
):
    if fiat_rate is None and fiat_value is None:
        raise AppError(
            "Provide at least one of --fiat-rate or --fiat-value",
            code="validation",
        )
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    new_rate = dec(fiat_rate) if fiat_rate is not None else None
    new_value = dec(fiat_value) if fiat_value is not None else None
    amount = abs(msat_to_btc(tx["amount"]))
    if new_rate is None and new_value is not None and amount > 0:
        new_rate = new_value / amount
    if new_value is None and new_rate is not None and amount > 0:
        new_value = new_rate * amount
    if new_rate is not None and new_rate <= 0:
        raise AppError("--fiat-rate must be positive", code="validation")
    if new_value is not None and new_value < 0:
        raise AppError("--fiat-value must not be negative", code="validation")
    payload = pricing.pricing_payload(
        rate=new_rate,
        value=new_value,
        source_kind=pricing.SOURCE_MANUAL_OVERRIDE,
        quality=pricing.QUALITY_EXACT,
        provider="manual",
        pricing_timestamp=tx["confirmed_at"] or tx["occurred_at"],
        fetched_at=now_iso(),
        granularity="exact",
        method="quarantine_price_override",
    )
    conn.execute(
        """
        UPDATE transactions
        SET fiat_rate = ?, fiat_value = ?, fiat_price_source = ?,
            fiat_rate_exact = ?, fiat_value_exact = ?,
            pricing_source_kind = ?, pricing_provider = ?, pricing_pair = ?,
            pricing_timestamp = ?, pricing_fetched_at = ?,
            pricing_granularity = ?, pricing_method = ?,
            pricing_external_ref = ?, pricing_quality = ?
        WHERE id = ?
        """,
        (
            payload["fiat_rate"],
            payload["fiat_value"],
            payload["fiat_price_source"],
            payload["fiat_rate_exact"],
            payload["fiat_value_exact"],
            payload["pricing_source_kind"],
            payload["pricing_provider"],
            payload["pricing_pair"],
            payload["pricing_timestamp"],
            payload["pricing_fetched_at"],
            payload["pricing_granularity"],
            payload["pricing_method"],
            payload["pricing_external_ref"],
            payload["pricing_quality"],
            tx["id"],
        ),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "price-override",
        "fiat_rate": float(new_rate) if new_rate is not None else None,
        "fiat_value": float(new_value) if new_value is not None else None,
        "fiat_rate_exact": payload["fiat_rate_exact"],
        "fiat_value_exact": payload["fiat_value_exact"],
        "pricing_source_kind": payload["pricing_source_kind"],
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def resolve_quarantine_exclude(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "UPDATE transactions SET excluded = 1 WHERE id = ?",
        (tx["id"],),
    )
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "exclude",
        "excluded": True,
        "note": "Run `kassiber journals process` to regenerate entries.",
    }


def clear_quarantine(conn, workspace_ref, profile_ref, tx_ref):
    _, profile = resolve_scope(conn, workspace_ref, profile_ref)
    tx = resolve_transaction(conn, profile["id"], tx_ref)
    _ensure_quarantined(conn, profile["id"], tx["id"])
    conn.execute(
        "DELETE FROM journal_quarantines WHERE profile_id = ? AND transaction_id = ?",
        (profile["id"], tx["id"]),
    )
    invalidate_journals(conn, profile["id"])
    conn.commit()
    return {
        "transaction_id": tx["id"],
        "resolution": "clear",
        "note": "Run `kassiber journals process` to re-evaluate.",
    }


def require_processed_journals(conn, profile):
    current_count = conn.execute(
        "SELECT COUNT(*) AS count FROM transactions WHERE profile_id = ? AND excluded = 0",
        (profile["id"],),
    ).fetchone()["count"]
    if not profile["last_processed_at"] or current_count != profile["last_processed_tx_count"]:
        raise AppError("Reports require fresh journals. Run `kassiber journals process` first.")


def show_status(conn, data_root):
    return build_status_payload(conn, data_root)


def get_profile_details(conn, workspace_ref=None, profile_ref=None):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)
    current_profile_id = get_setting(conn, "context_profile")
    current_workspace_id = get_setting(conn, "context_workspace")
    return {
        "id": profile["id"],
        "workspace_id": profile["workspace_id"],
        "workspace_label": workspace["label"],
        "label": profile["label"],
        "fiat_currency": profile["fiat_currency"],
        "tax_country": profile["tax_country"],
        "tax_long_term_days": profile["tax_long_term_days"],
        "gains_algorithm": profile["gains_algorithm"],
        "last_processed_at": profile["last_processed_at"],
        "last_processed_tx_count": profile["last_processed_tx_count"],
        "created_at": profile["created_at"],
        "is_current": profile["id"] == current_profile_id and profile["workspace_id"] == current_workspace_id,
    }


def update_profile(conn, workspace_ref, profile_ref, updates):
    workspace = resolve_workspace(conn, workspace_ref)
    profile = resolve_profile(conn, workspace["id"], profile_ref)

    new_label = updates.get("label")
    new_fiat = updates.get("fiat_currency")
    new_country = updates.get("tax_country")
    new_long_term = updates.get("tax_long_term_days")
    new_algo = updates.get("gains_algorithm")

    merged_fiat = new_fiat if new_fiat is not None else profile["fiat_currency"]
    merged_country = new_country if new_country is not None else profile["tax_country"]
    merged_long_term = new_long_term if new_long_term is not None else profile["tax_long_term_days"]
    merged_algo = new_algo if new_algo is not None else profile["gains_algorithm"]
    merged_label = new_label if new_label is not None else profile["label"]

    if new_long_term is not None and new_long_term < 0:
        raise AppError(
            "Tax long-term days cannot be negative",
            code="validation",
            hint="Use a non-negative integer; pass 0 to treat every disposal as short-term.",
        )
    if new_algo is not None and new_algo.upper() not in RP2_ACCOUNTING_METHODS:
        raise AppError(
            f"Unsupported gains algorithm '{new_algo}'",
            code="validation",
            hint=f"Choose one of: {', '.join(RP2_ACCOUNTING_METHODS)}",
        )
    if new_country is not None:
        require_tax_country_supported_for_profile_mutation(new_country)
    try:
        policy = build_tax_policy(
            {
                "fiat_currency": merged_fiat,
                "tax_country": merged_country,
                "tax_long_term_days": merged_long_term,
            }
        )
    except ValueError as exc:
        raise AppError(str(exc), code="validation") from exc
    normalized_algo = merged_algo.upper()
    policy_changed = (
        policy.fiat_currency != profile["fiat_currency"]
        or policy.tax_country != profile["tax_country"]
        or policy.long_term_days != profile["tax_long_term_days"]
        or normalized_algo != profile["gains_algorithm"]
    )

    conn.execute(
        """
        UPDATE profiles
        SET label = ?, fiat_currency = ?, tax_country = ?, tax_long_term_days = ?, gains_algorithm = ?
        WHERE id = ?
        """,
        (
            merged_label,
            policy.fiat_currency,
            policy.tax_country,
            policy.long_term_days,
            normalized_algo,
            profile["id"],
        ),
    )
    if policy_changed:
        invalidate_journals(conn, profile["id"])
    conn.commit()
    return get_profile_details(conn, workspace["label"], profile["id"])


def cmd_init(conn, args):
    init_app(conn)
    state_root = resolve_effective_state_root(args.data_root)
    effective_data_root = resolve_effective_data_root(args.data_root)
    emit(
        args,
        {
            "version": __version__,
            "state_root": str(state_root),
            "data_root": str(effective_data_root),
            "database": str(resolve_database_path(effective_data_root)),
            "config_root": str(resolve_config_root(args.data_root)),
            "settings_file": str(resolve_settings_path(args.data_root)),
            "exports_root": str(resolve_exports_root(args.data_root)),
            "attachments_root": str(resolve_attachments_root(args.data_root)),
            "env_file": str(args.env_file),
        },
    )


def cmd_status(conn, args):
    payload = show_status(conn, args.data_root)
    payload["default_backend"] = args.runtime_config["default_backend"]
    payload["env_file"] = args.runtime_config["env_file"]
    emit(args, payload)


def cmd_context_show(conn, args):
    emit(args, current_context_snapshot(conn))


def cmd_context_set(conn, args):
    if args.workspace:
        workspace = resolve_workspace(conn, args.workspace)
        set_setting(conn, "context_workspace", workspace["id"])
        if args.profile:
            profile = resolve_profile(conn, workspace["id"], args.profile)
            set_setting(conn, "context_profile", profile["id"])
        else:
            current_profile_id = get_setting(conn, "context_profile")
            if current_profile_id:
                profile = conn.execute(
                    "SELECT 1 FROM profiles WHERE id = ? AND workspace_id = ?",
                    (current_profile_id, workspace["id"]),
                ).fetchone()
                if not profile:
                    set_setting(conn, "context_profile", "")
        conn.commit()
    elif args.profile:
        workspace = resolve_workspace(conn)
        profile = resolve_profile(conn, workspace["id"], args.profile)
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()
    else:
        raise AppError("Provide --workspace and/or --profile")
    cmd_context_show(conn, args)
