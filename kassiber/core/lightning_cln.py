from __future__ import annotations

"""Read-only Core Lightning sync and profitability helpers."""

import hashlib
import json
import sqlite3
import subprocess
import uuid
from dataclasses import dataclass
from typing import Any, Callable, Mapping, Sequence

from ..backends import backend_timeout, backend_value, redact_backend_url
from ..envelope import json_ready
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import UNKNOWN_OCCURRED_AT, now_iso, timestamp_to_iso
from ..util import str_or_none
from . import imports as core_imports
from .repo import invalidate_journals, resolve_scope, resolve_wallet

CLN_BACKEND_KIND = "coreln"
CLN_WALLET_KIND = "coreln"
CLN_IMPORT_SOURCE = "core-lightning"
CLN_ALLOWED_METHODS = (
    "getinfo",
    "bkpr-listaccountevents",
    "bkpr-listincome",
    "bkpr-listbalances",
    "listfunds",
    "listforwards",
    "listpays",
    "listinvoices",
    "listtransactions",
    "listpeerchannels",
)
CLN_READONLY_RUNE_RESTRICTIONS = (
    '[["method^list","method^get","method^bkpr-list","method=summary"],'
    '["method/listdatastore"]]'
)


RpcCall = Callable[[str, Sequence[str] | None], Mapping[str, Any]]


@dataclass(frozen=True)
class CoreLightningSnapshot:
    node_id: str
    node_alias: str
    method_payloads: Mapping[str, Mapping[str, Any]]
    errors: Mapping[str, str]


def _json_dumps(value: Any) -> str:
    return json.dumps(json_ready(value), sort_keys=True, separators=(",", ":"))


def _stable_hash(parts: Sequence[Any]) -> str:
    payload = _json_dumps(list(parts)).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def _parse_msat(value: Any) -> int:
    if value in (None, ""):
        return 0
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    if isinstance(value, Mapping):
        for key in ("msat", "millisatoshis", "millisatoshi"):
            if key in value:
                return _parse_msat(value[key])
        return 0
    text = str(value).strip().lower().replace(",", "")
    if not text:
        return 0
    for suffix, factor in (("msat", 1), ("sat", 1000), ("btc", 100_000_000_000)):
        if text.endswith(suffix):
            number = text[: -len(suffix)].strip()
            try:
                return int(float(number) * factor)
            except ValueError as exc:
                raise AppError(f"Invalid Core Lightning msat value '{value}'") from exc
    try:
        return int(text)
    except ValueError as exc:
        raise AppError(f"Invalid Core Lightning msat value '{value}'") from exc


def _timestamp(value: Any) -> str:
    if value in (None, "", 0, "0"):
        return UNKNOWN_OCCURRED_AT
    try:
        return timestamp_to_iso(int(float(value)))
    except (TypeError, ValueError):
        return UNKNOWN_OCCURRED_AT


def _normalize_payment_hash(value: Any) -> str | None:
    text = str_or_none(value)
    if text is None:
        return None
    text = text.lower()
    if len(text) != 64:
        return None
    try:
        bytes.fromhex(text)
    except ValueError:
        return None
    return text


def _record_id(
    profile_id: str,
    wallet_id: str,
    backend_name: str,
    record_type: str,
    external_id: str,
) -> str:
    return _stable_hash((profile_id, wallet_id, backend_name, record_type, external_id))


def _command_for_backend(backend: Mapping[str, Any], method: str, args: Sequence[str] | None) -> list[str]:
    binary = backend_value(backend, "lightning_cli") or "lightning-cli"
    command = [binary, "--json", "--raw"]
    lightning_dir = backend_value(backend, "lightning_dir")
    if lightning_dir:
        command.append(f"--lightning-dir={lightning_dir}")
    rpc_file = backend_value(backend, "rpc_file")
    if rpc_file:
        command.append(f"--rpc-file={rpc_file}")
    network = backend_value(backend, "network")
    if network:
        command.append(f"--network={network}")
    peer_id = backend_value(backend, "commando_peer_id")
    rune = backend_value(backend, "token")
    wants_commando = (
        bool(peer_id)
        or bool(rune)
        or str(backend.get("url") or "").lower().startswith("cln://commando")
    )
    if wants_commando and not (peer_id and rune):
        raise AppError(
            "Core Lightning commando sync requires both --commando-peer-id and a rune token",
            code="validation",
            hint="Pipe a restricted rune through --token-stdin or --token-fd FD.",
        )
    if wants_commando:
        command.append(f"--commando={peer_id}:{rune}")
    command.append(method)
    command.extend(args or ())
    return command


def _redacted_command(command: Sequence[str]) -> list[str]:
    output = []
    for part in command:
        if part.startswith("--commando="):
            output.append("--commando=<redacted>")
        else:
            output.append(part)
    return output


def call_core_lightning(backend: Mapping[str, Any], method: str, args: Sequence[str] | None = None) -> Mapping[str, Any]:
    if method not in CLN_ALLOWED_METHODS:
        raise AppError(
            f"Core Lightning sync refused unsupported RPC method '{method}'",
            code="validation",
            hint="Only read-only list/get/bookkeeper methods are allowed.",
        )
    command = _command_for_backend(backend, method, args)
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=backend_timeout(backend, default=30),
        )
    except FileNotFoundError as exc:
        raise AppError(
            "Core Lightning CLI executable was not found",
            code="dependency_missing",
            hint="Install Core Lightning or set backend config lightning_cli to the lightning-cli path.",
            details={"command": command[0]},
            retryable=True,
        ) from exc
    except subprocess.TimeoutExpired as exc:
        raise AppError(
            f"Core Lightning RPC method '{method}' timed out",
            code="timeout",
            details={"method": method, "command": _redacted_command(command)},
            retryable=True,
        ) from exc
    if completed.returncode != 0:
        stderr = (completed.stderr or completed.stdout or "").strip()
        raise AppError(
            f"Core Lightning RPC method '{method}' failed",
            code="sync_error",
            hint="Check that the backend points at a running CLN node and that the rune allows this read method.",
            details={
                "method": method,
                "command": _redacted_command(command),
                "stderr": stderr[-800:],
            },
            retryable=True,
        )
    try:
        payload = json.loads(completed.stdout or "{}")
    except json.JSONDecodeError as exc:
        raise AppError(
            f"Core Lightning RPC method '{method}' did not return JSON",
            code="sync_error",
            details={"method": method, "command": _redacted_command(command)},
            retryable=True,
        ) from exc
    return payload if isinstance(payload, Mapping) else {"result": payload}


def fetch_core_lightning_snapshot(
    backend: Mapping[str, Any],
    *,
    rpc_call: RpcCall | None = None,
) -> CoreLightningSnapshot:
    caller = rpc_call or (lambda method, args=None: call_core_lightning(backend, method, args))
    info = caller("getinfo", None)
    node_id = str_or_none(info.get("id")) or ""
    node_alias = str_or_none(info.get("alias")) or ""
    payloads: dict[str, Mapping[str, Any]] = {"getinfo": info}
    errors: dict[str, str] = {}
    for method in CLN_ALLOWED_METHODS:
        if method == "getinfo":
            continue
        try:
            payloads[method] = caller(method, None)
        except AppError as exc:
            errors[method] = str(exc)
    return CoreLightningSnapshot(
        node_id=node_id,
        node_alias=node_alias,
        method_payloads=payloads,
        errors=errors,
    )


def _account_event_record(event: Mapping[str, Any]) -> dict[str, Any]:
    credit = _parse_msat(event.get("credit_msat"))
    debit = _parse_msat(event.get("debit_msat"))
    net = credit - debit
    event_type = str_or_none(event.get("type"))
    tag = str_or_none(event.get("tag"))
    txid = str_or_none(event.get("txid"))
    outpoint = str_or_none(event.get("outpoint"))
    payment_hash = _normalize_payment_hash(event.get("payment_id"))
    account = str_or_none(event.get("account"))
    external_id = _stable_hash(
        (
            "account_event",
            account,
            event_type,
            tag,
            event.get("timestamp"),
            txid,
            outpoint,
            event.get("part_id"),
            credit,
            debit,
            payment_hash,
        )
    )
    return {
        "record_type": "account_event",
        "external_id": external_id,
        "occurred_at": _timestamp(event.get("timestamp")),
        "account": account,
        "channel_id": account if event_type == "channel" else None,
        "direction": "inbound" if net > 0 else "outbound" if net < 0 else "",
        "amount_msat": abs(net),
        "fee_msat": _parse_msat(event.get("fees_msat")),
        "tag": tag,
        "status": event_type,
        "currency": str_or_none(event.get("currency")),
        "payment_hash": payment_hash,
        "txid": txid,
        "outpoint": outpoint,
        "raw_json": event,
    }


def _income_record(event: Mapping[str, Any]) -> dict[str, Any]:
    credit = _parse_msat(event.get("credit_msat"))
    debit = _parse_msat(event.get("debit_msat"))
    net = credit - debit
    txid = str_or_none(event.get("txid"))
    outpoint = str_or_none(event.get("outpoint"))
    payment_hash = _normalize_payment_hash(event.get("payment_id"))
    external_id = _stable_hash(
        (
            "income",
            event.get("account"),
            event.get("tag"),
            event.get("timestamp"),
            txid,
            outpoint,
            payment_hash,
            credit,
            debit,
        )
    )
    return {
        "record_type": "income",
        "external_id": external_id,
        "occurred_at": _timestamp(event.get("timestamp")),
        "account": str_or_none(event.get("account")),
        "channel_id": str_or_none(event.get("account")),
        "direction": "inbound" if net > 0 else "outbound" if net < 0 else "",
        "amount_msat": abs(net),
        "fee_msat": 0,
        "tag": str_or_none(event.get("tag")),
        "status": str_or_none(event.get("description")),
        "currency": str_or_none(event.get("currency")),
        "payment_hash": payment_hash,
        "txid": txid,
        "outpoint": outpoint,
        "raw_json": event,
    }


def _balance_record(account: Mapping[str, Any], synced_at: str) -> dict[str, Any]:
    balances = account.get("balances") if isinstance(account.get("balances"), list) else []
    total_msat = sum(_parse_msat(entry.get("balance_msat")) for entry in balances if isinstance(entry, Mapping))
    account_name = str_or_none(account.get("account"))
    return {
        "record_type": "balance_snapshot",
        "external_id": _stable_hash(("balance", synced_at, account_name, account.get("peer_id"))),
        "occurred_at": synced_at,
        "account": account_name,
        "peer_id": str_or_none(account.get("peer_id")),
        "channel_id": account_name if account.get("peer_id") else None,
        "direction": "",
        "amount_msat": total_msat,
        "fee_msat": 0,
        "tag": "balance",
        "status": "closed" if account.get("account_closed") else "open",
        "currency": "",
        "payment_hash": None,
        "txid": None,
        "outpoint": None,
        "raw_json": account,
    }


def _forward_record(forward: Mapping[str, Any]) -> dict[str, Any]:
    resolved_time = forward.get("resolved_time") or forward.get("received_time")
    out_channel = str_or_none(forward.get("out_channel"))
    in_channel = str_or_none(forward.get("in_channel"))
    fee_msat = _parse_msat(forward.get("fee_msat"))
    external_id = _stable_hash(
        (
            "forward",
            in_channel,
            out_channel,
            forward.get("received_time"),
            resolved_time,
            forward.get("payment_hash"),
            fee_msat,
            forward.get("status"),
        )
    )
    return {
        "record_type": "forward",
        "external_id": external_id,
        "occurred_at": _timestamp(resolved_time),
        "account": None,
        "peer_id": None,
        "channel_id": out_channel or in_channel,
        "direction": "inbound",
        "amount_msat": _parse_msat(forward.get("out_msat") or forward.get("in_msat")),
        "fee_msat": fee_msat,
        "tag": "routing_fee",
        "status": str_or_none(forward.get("status")),
        "currency": "bc",
        "payment_hash": _normalize_payment_hash(forward.get("payment_hash")),
        "txid": None,
        "outpoint": None,
        "raw_json": forward,
    }


def _pay_record(pay: Mapping[str, Any]) -> dict[str, Any]:
    payment_hash = _normalize_payment_hash(pay.get("payment_hash"))
    amount_sent = _parse_msat(pay.get("amount_sent_msat"))
    amount = _parse_msat(pay.get("amount_msat"))
    fee_msat = max(0, amount_sent - amount)
    external_id = str_or_none(pay.get("payment_hash")) or _stable_hash(("pay", pay))
    return {
        "record_type": "pay",
        "external_id": external_id,
        "occurred_at": _timestamp(pay.get("created_at") or pay.get("completed_at")),
        "account": None,
        "peer_id": str_or_none(pay.get("destination")),
        "channel_id": None,
        "direction": "outbound",
        "amount_msat": amount or amount_sent,
        "fee_msat": fee_msat,
        "tag": "payment",
        "status": str_or_none(pay.get("status")),
        "currency": "bc",
        "payment_hash": payment_hash,
        "txid": None,
        "outpoint": None,
        "raw_json": pay,
    }


def _invoice_record(invoice: Mapping[str, Any]) -> dict[str, Any]:
    payment_hash = _normalize_payment_hash(invoice.get("payment_hash"))
    label = str_or_none(invoice.get("label"))
    external_id = str_or_none(invoice.get("payment_hash")) or label or _stable_hash(("invoice", invoice))
    return {
        "record_type": "invoice",
        "external_id": external_id,
        "occurred_at": _timestamp(invoice.get("paid_at") or invoice.get("expires_at") or invoice.get("created_at")),
        "account": None,
        "peer_id": None,
        "channel_id": None,
        "direction": "inbound" if invoice.get("status") == "paid" else "",
        "amount_msat": _parse_msat(invoice.get("amount_received_msat") or invoice.get("amount_msat")),
        "fee_msat": 0,
        "tag": "invoice",
        "status": str_or_none(invoice.get("status")),
        "currency": "bc",
        "payment_hash": payment_hash,
        "txid": None,
        "outpoint": None,
        "raw_json": invoice,
    }


def _fund_record(record_type: str, row: Mapping[str, Any], synced_at: str) -> dict[str, Any]:
    txid = str_or_none(row.get("txid") or row.get("funding_txid"))
    outnum = row.get("output") if row.get("output") is not None else row.get("funding_output")
    outpoint = f"{txid}:{outnum}" if txid and outnum is not None else str_or_none(row.get("outpoint"))
    channel_id = str_or_none(row.get("short_channel_id") or row.get("channel_id"))
    external_id = outpoint or channel_id or _stable_hash((record_type, row))
    return {
        "record_type": record_type,
        "external_id": external_id,
        "occurred_at": _timestamp(row.get("timestamp") or row.get("blockheight")) if row.get("timestamp") else synced_at,
        "account": channel_id,
        "peer_id": str_or_none(row.get("peer_id")),
        "channel_id": channel_id,
        "direction": "",
        "amount_msat": _parse_msat(row.get("amount_msat") or row.get("our_amount_msat")),
        "fee_msat": 0,
        "tag": record_type,
        "status": str_or_none(row.get("status") or row.get("state")),
        "currency": "bc",
        "payment_hash": None,
        "txid": txid,
        "outpoint": outpoint,
        "raw_json": row,
    }


def _transaction_record(tx: Mapping[str, Any]) -> dict[str, Any]:
    txid = str_or_none(tx.get("hash") or tx.get("txid"))
    external_id = txid or _stable_hash(("onchain_transaction", tx))
    return {
        "record_type": "onchain_transaction",
        "external_id": external_id,
        "occurred_at": _timestamp(tx.get("blocktime") or tx.get("time")),
        "account": "wallet",
        "peer_id": None,
        "channel_id": None,
        "direction": "",
        "amount_msat": 0,
        "fee_msat": 0,
        "tag": "onchain_transaction",
        "status": str_or_none(tx.get("status")),
        "currency": "bc",
        "payment_hash": None,
        "txid": txid,
        "outpoint": None,
        "raw_json": tx,
    }


def snapshot_records(snapshot: CoreLightningSnapshot, synced_at: str) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    account_events = snapshot.method_payloads.get("bkpr-listaccountevents", {}).get("events") or []
    records.extend(_account_event_record(row) for row in account_events if isinstance(row, Mapping))
    income_events = snapshot.method_payloads.get("bkpr-listincome", {}).get("income_events") or []
    records.extend(_income_record(row) for row in income_events if isinstance(row, Mapping))
    accounts = snapshot.method_payloads.get("bkpr-listbalances", {}).get("accounts") or []
    records.extend(_balance_record(row, synced_at) for row in accounts if isinstance(row, Mapping))
    forwards = snapshot.method_payloads.get("listforwards", {}).get("forwards") or []
    records.extend(_forward_record(row) for row in forwards if isinstance(row, Mapping))
    pays = snapshot.method_payloads.get("listpays", {}).get("pays") or []
    records.extend(_pay_record(row) for row in pays if isinstance(row, Mapping))
    invoices = snapshot.method_payloads.get("listinvoices", {}).get("invoices") or []
    records.extend(_invoice_record(row) for row in invoices if isinstance(row, Mapping))
    funds = snapshot.method_payloads.get("listfunds", {})
    records.extend(
        _fund_record("onchain_output", row, synced_at)
        for row in funds.get("outputs", []) or []
        if isinstance(row, Mapping)
    )
    records.extend(
        _fund_record("channel", row, synced_at)
        for row in funds.get("channels", []) or []
        if isinstance(row, Mapping)
    )
    transactions = snapshot.method_payloads.get("listtransactions", {}).get("transactions") or []
    records.extend(_transaction_record(row) for row in transactions if isinstance(row, Mapping))
    peer_channels = snapshot.method_payloads.get("listpeerchannels", {}).get("channels") or []
    records.extend(
        _fund_record("peer_channel", row, synced_at)
        for row in peer_channels
        if isinstance(row, Mapping)
    )
    return records


def _upsert_lightning_record(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    sync_id: str,
    node_id: str,
    record: Mapping[str, Any],
    timestamp: str,
) -> tuple[bool, bool]:
    record_id = _record_id(
        profile["id"],
        wallet["id"],
        backend["name"],
        record["record_type"],
        record["external_id"],
    )
    existing = conn.execute(
        "SELECT raw_json FROM lightning_node_records WHERE id = ?",
        (record_id,),
    ).fetchone()
    raw_json = _json_dumps(record["raw_json"])
    conn.execute(
        """
        INSERT INTO lightning_node_records(
            id, workspace_id, profile_id, wallet_id, backend_name, node_id,
            record_type, external_id, occurred_at, account, peer_id, channel_id,
            direction, amount_msat, fee_msat, tag, status, currency, payment_hash,
            txid, outpoint, sync_id, raw_json, first_seen_at, updated_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, wallet_id, backend_name, record_type, external_id)
        DO UPDATE SET
            node_id = excluded.node_id,
            occurred_at = excluded.occurred_at,
            account = excluded.account,
            peer_id = excluded.peer_id,
            channel_id = excluded.channel_id,
            direction = excluded.direction,
            amount_msat = excluded.amount_msat,
            fee_msat = excluded.fee_msat,
            tag = excluded.tag,
            status = excluded.status,
            currency = excluded.currency,
            payment_hash = excluded.payment_hash,
            txid = excluded.txid,
            outpoint = excluded.outpoint,
            sync_id = excluded.sync_id,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            record_id,
            profile["workspace_id"],
            profile["id"],
            wallet["id"],
            backend["name"],
            node_id,
            record["record_type"],
            record["external_id"],
            record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
            record.get("account"),
            record.get("peer_id"),
            record.get("channel_id"),
            record.get("direction"),
            int(record.get("amount_msat") or 0),
            int(record.get("fee_msat") or 0),
            record.get("tag"),
            record.get("status"),
            record.get("currency"),
            record.get("payment_hash"),
            record.get("txid"),
            record.get("outpoint"),
            sync_id,
            raw_json,
            timestamp,
            timestamp,
        ),
    )
    if existing is None:
        return True, False
    return False, existing["raw_json"] != raw_json


def _record_to_import(record: Mapping[str, Any]) -> dict[str, Any] | None:
    if record.get("record_type") != "income":
        return None
    amount_msat = int(record.get("amount_msat") or 0)
    if amount_msat <= 0:
        return None
    direction = record.get("direction")
    if direction != "inbound":
        return None
    return {
        "id": f"cln:income:{record['external_id']}",
        "occurred_at": record.get("occurred_at") or UNKNOWN_OCCURRED_AT,
        "direction": direction,
        "asset": "BTC",
        "amount": msat_to_btc(amount_msat),
        "fee": 0,
        "kind": f"cln_{record.get('tag') or 'income'}",
        "description": record.get("status") or record.get("tag") or "Core Lightning income event",
        "counterparty": record.get("account"),
        "payment_hash": record.get("payment_hash"),
        "payment_hash_source": "core_lightning" if record.get("payment_hash") else None,
        "raw_json": _json_dumps(record.get("raw_json") or {}),
    }


def sync_core_lightning_wallet(
    conn: sqlite3.Connection,
    profile: Mapping[str, Any],
    wallet: Mapping[str, Any],
    backend: Mapping[str, Any],
    hooks: core_imports.ImportCoordinatorHooks,
    *,
    commit: bool = True,
    rpc_call: RpcCall | None = None,
) -> dict[str, Any]:
    if str(backend.get("kind") or "").lower() != CLN_BACKEND_KIND:
        raise AppError(
            f"Backend '{backend.get('name')}' has kind '{backend.get('kind')}', expected 'coreln'",
            code="validation",
            hint="Create a Core Lightning backend with `kassiber backends create <name> --kind coreln --url cln://local`.",
        )
    started_at = now_iso()
    sync_id = str(uuid.uuid4())
    conn.execute(
        """
        INSERT INTO lightning_node_syncs(
            id, workspace_id, profile_id, wallet_id, backend_name, started_at,
            status, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            sync_id,
            profile["workspace_id"],
            profile["id"],
            wallet["id"],
            backend["name"],
            started_at,
            "running",
            started_at,
        ),
    )
    snapshot = fetch_core_lightning_snapshot(backend, rpc_call=rpc_call)
    completed_at = now_iso()
    records = snapshot_records(snapshot, completed_at)
    inserted = 0
    updated = 0
    counts: dict[str, int] = {}
    import_records: list[dict[str, Any]] = []
    for record in records:
        counts[record["record_type"]] = counts.get(record["record_type"], 0) + 1
        was_inserted, was_updated = _upsert_lightning_record(
            conn,
            profile,
            wallet,
            backend,
            sync_id,
            snapshot.node_id,
            record,
            completed_at,
        )
        inserted += 1 if was_inserted else 0
        updated += 1 if was_updated else 0
        import_record = _record_to_import(record)
        if import_record is not None:
            import_records.append(import_record)
    import_outcome = core_imports.insert_wallet_records(
        conn,
        profile,
        wallet,
        import_records,
        CLN_IMPORT_SOURCE,
        hooks,
        commit=False,
    )
    status = "partial" if snapshot.errors else "synced"
    conn.execute(
        """
        UPDATE lightning_node_syncs
        SET node_id = ?, node_alias = ?, completed_at = ?, status = ?,
            fetched_counts_json = ?, error_json = ?
        WHERE id = ?
        """,
        (
            snapshot.node_id,
            snapshot.node_alias,
            completed_at,
            status,
            _json_dumps(counts),
            _json_dumps(snapshot.errors),
            sync_id,
        ),
    )
    invalidate_journals(conn, profile["id"])
    if commit:
        conn.commit()
    return {
        "wallet": wallet["label"],
        "backend": backend["name"],
        "backend_kind": CLN_BACKEND_KIND,
        "backend_url": redact_backend_url(backend["url"]),
        "node_id": snapshot.node_id,
        "node_alias": snapshot.node_alias,
        "status": status,
        "sync_id": sync_id,
        "started_at": started_at,
        "completed_at": completed_at,
        "records_fetched": len(records),
        "records_inserted": inserted,
        "records_updated": updated,
        "fetched_counts": counts,
        "method_errors": dict(snapshot.errors),
        "transactions": {
            "fetched": len(import_records),
            "imported": import_outcome["imported"],
            "skipped": import_outcome["skipped"],
            "updated": import_outcome.get("updated", 0),
        },
        "read_only": True,
    }


def _record_rows(conn: sqlite3.Connection, workspace_ref: str | None, profile_ref: str | None, wallet_ref: str | None):
    _workspace, profile = resolve_scope(conn, workspace_ref, profile_ref)
    params: list[Any] = [profile["id"]]
    wallet_filter = ""
    if wallet_ref:
        wallet = resolve_wallet(conn, profile["id"], wallet_ref)
        wallet_filter = "AND wallet_id = ?"
        params.append(wallet["id"])
    rows = conn.execute(
        f"""
        SELECT *
        FROM lightning_node_records
        WHERE profile_id = ?
          {wallet_filter}
        ORDER BY occurred_at ASC, record_type ASC
        """,
        tuple(params),
    ).fetchall()
    return profile, rows


def _sum(rows: Sequence[sqlite3.Row], predicate) -> int:
    return sum(int(row["amount_msat"] or 0) for row in rows if predicate(row))


def _sum_fee(rows: Sequence[sqlite3.Row], predicate) -> int:
    return sum(int(row["fee_msat"] or 0) for row in rows if predicate(row))


def _status(row: sqlite3.Row) -> str:
    return (row["status"] or "").lower()


def report_lightning_profitability(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    *,
    wallet_ref: str | None = None,
) -> dict[str, Any]:
    _profile, rows = _record_rows(conn, workspace_ref, profile_ref, wallet_ref)
    routing_revenue = _sum_fee(
        rows,
        lambda row: row["record_type"] == "forward"
        and _status(row) in {"settled", "local_failed", "offered"},
    )
    settled_routing_revenue = _sum_fee(
        rows,
        lambda row: row["record_type"] == "forward" and _status(row) == "settled",
    )
    invoice_income = _sum(
        rows,
        lambda row: row["record_type"] == "income"
        and row["direction"] == "inbound"
        and (row["tag"] or "").lower() == "invoice",
    )
    bookkeeper_income_net = sum(
        int(row["amount_msat"] or 0) * (1 if row["direction"] == "inbound" else -1)
        for row in rows
        if row["record_type"] == "income"
    )
    payment_cost = _sum_fee(
        rows,
        lambda row: row["record_type"] == "pay" and _status(row) in {"complete", "completed", "paid"},
    )
    onchain_cost = _sum(
        rows,
        lambda row: row["record_type"] in {"income", "account_event"}
        and row["direction"] == "outbound"
        and (row["tag"] or "").lower() in {"onchain_fee", "onchain_fees"},
    )
    rebalances = [
        row
        for row in rows
        if row["record_type"] in {"income", "account_event", "pay"}
        and "rebalance" in (row["raw_json"] or "").lower()
    ]
    rebalance_cost = sum(int(row["amount_msat"] or 0) + int(row["fee_msat"] or 0) for row in rebalances if row["direction"] == "outbound")
    channel_totals: dict[str, dict[str, Any]] = {}
    latest_balances: dict[str, sqlite3.Row] = {}
    for row in rows:
        channel_id = row["channel_id"] or row["account"]
        if not channel_id:
            continue
        bucket = channel_totals.setdefault(
            channel_id,
            {
                "channel_id": channel_id,
                "routing_revenue_msat": 0,
                "cost_msat": 0,
                "balance_msat": 0,
                "break_even": False,
            },
        )
        if row["record_type"] == "forward" and (row["status"] or "").lower() == "settled":
            bucket["routing_revenue_msat"] += int(row["fee_msat"] or 0)
        if row["record_type"] in {"income", "account_event"} and row["direction"] == "outbound":
            bucket["cost_msat"] += int(row["amount_msat"] or 0)
        if row["record_type"] == "balance_snapshot":
            current = latest_balances.get(channel_id)
            if current is None or row["occurred_at"] >= current["occurred_at"]:
                latest_balances[channel_id] = row
    for channel_id, row in latest_balances.items():
        channel_totals[channel_id]["balance_msat"] = int(row["amount_msat"] or 0)
    for bucket in channel_totals.values():
        bucket["net_msat"] = bucket["routing_revenue_msat"] - bucket["cost_msat"]
        bucket["break_even"] = bucket["net_msat"] >= 0
    sync_rows = conn.execute(
        """
        SELECT wallet_id, backend_name, node_id, node_alias, completed_at, status, error_json
        FROM lightning_node_syncs
        WHERE profile_id = ?
        ORDER BY completed_at DESC, started_at DESC
        LIMIT 5
        """,
        (_profile["id"],),
    ).fetchall()
    return {
        "wallet": wallet_ref or "",
        "record_count": len(rows),
        "routing_revenue_msat": settled_routing_revenue,
        "routing_revenue_candidate_msat": routing_revenue,
        "invoice_income_msat": invoice_income,
        "bookkeeper_income_net_msat": bookkeeper_income_net,
        "payment_cost_msat": payment_cost,
        "rebalance_cost_msat": rebalance_cost,
        "onchain_cost_msat": onchain_cost,
        "net_routing_profit_msat": settled_routing_revenue - payment_cost - rebalance_cost - onchain_cost,
        "channels": sorted(channel_totals.values(), key=lambda row: row["channel_id"]),
        "recent_syncs": [dict(row) for row in sync_rows],
    }


def export_lightning_profitability_csv(
    conn: sqlite3.Connection,
    workspace_ref: str | None,
    profile_ref: str | None,
    file_path: str,
    *,
    wallet_ref: str | None = None,
) -> dict[str, Any]:
    import csv
    from pathlib import Path

    report = report_lightning_profitability(conn, workspace_ref, profile_ref, wallet_ref=wallet_ref)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=(
                "channel_id",
                "routing_revenue_msat",
                "cost_msat",
                "net_msat",
                "balance_msat",
                "break_even",
            ),
        )
        writer.writeheader()
        for row in report["channels"]:
            writer.writerow(row)
    return {
        "file": str(path.resolve()),
        "rows": len(report["channels"]),
        "wallet": wallet_ref or "",
    }


__all__ = [
    "CLN_ALLOWED_METHODS",
    "CLN_BACKEND_KIND",
    "CLN_READONLY_RUNE_RESTRICTIONS",
    "CLN_WALLET_KIND",
    "call_core_lightning",
    "export_lightning_profitability_csv",
    "fetch_core_lightning_snapshot",
    "report_lightning_profitability",
    "snapshot_records",
    "sync_core_lightning_wallet",
]
