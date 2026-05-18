from __future__ import annotations

import csv
import json
import ssl
import uuid
from pathlib import Path
from typing import Any, Mapping
from urllib import error as urlerror
from urllib import parse as urlparse
from urllib import request as urlrequest

from .. import __version__
from ..backends import backend_timeout, backend_value
from ..db import APP_NAME
from ..errors import AppError
from ..msat import msat_to_btc
from ..time_utils import now_iso, timestamp_to_iso

LND_DEFAULT_PAGE_SIZE = 100
LND_MAX_PAGE_SIZE = 1000
LND_DATASETS = (
    "channels",
    "closed_channels",
    "forwards",
    "payments",
    "invoices",
    "wallet_transactions",
    "snapshots",
)


def _int(value: Any, default: int = 0) -> int:
    if value in (None, ""):
        return default
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


def _msat(value: Any, *, sats: bool = False) -> int:
    amount = _int(value)
    return amount * 1000 if sats else amount


def _timestamp(value: Any) -> str | None:
    raw = _int(value)
    if raw <= 0:
        return None
    return timestamp_to_iso(raw)


def _stable_key(*parts: Any) -> str:
    return ":".join(str(part) for part in parts if part not in (None, ""))


def _json(payload: Any) -> str:
    return json.dumps(
        payload if payload is not None else {},
        sort_keys=True,
        separators=(",", ":"),
    )


def _cursor(conn, profile_id: str, backend_name: str, dataset: str) -> str | None:
    row = conn.execute(
        """
        SELECT cursor_value FROM lnd_sync_state
        WHERE profile_id = ? AND backend_name = ? AND dataset = ?
        """,
        (profile_id, backend_name, dataset),
    ).fetchone()
    return row["cursor_value"] if row else None


def _save_cursor(
    conn,
    profile_id: str,
    backend_name: str,
    dataset: str,
    cursor_value: str | int | None,
    raw: Mapping[str, Any] | None = None,
) -> None:
    conn.execute(
        """
        INSERT INTO lnd_sync_state(profile_id, backend_name, dataset, cursor_value, synced_at, raw_cursor_json)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, dataset) DO UPDATE SET
            cursor_value = excluded.cursor_value,
            synced_at = excluded.synced_at,
            raw_cursor_json = excluded.raw_cursor_json
        """,
        (
            profile_id,
            backend_name,
            dataset,
            str(cursor_value) if cursor_value not in (None, "") else None,
            now_iso(),
            _json(raw or {}),
        ),
    )


def _backend_config(backend: Mapping[str, Any]) -> dict[str, Any]:
    raw = backend.get("config") or {}
    if isinstance(raw, dict):
        return raw
    try:
        loaded = json.loads(backend.get("config_json") or "{}")
    except (TypeError, ValueError, json.JSONDecodeError):
        return {}
    return loaded if isinstance(loaded, dict) else {}


def _macaroon_hex(backend: Mapping[str, Any]) -> str:
    value = backend_value(backend, "token")
    if not value:
        raise AppError(
            "LND backend is missing a read-only macaroon",
            code="config_error",
            hint="Store a scoped read-only macaroon with `backends create --kind lnd --token-stdin` or the desktop backend form.",
            retryable=False,
        )
    cleaned = value.strip()
    if (
        all(char in "0123456789abcdefABCDEF" for char in cleaned)
        and len(cleaned) % 2 == 0
    ):
        return cleaned.lower()
    raise AppError(
        "LND macaroon must be stored as hex",
        code="validation",
        hint="Use `xxd -p -c 256 readonly.macaroon | kassiber backends create --kind lnd --token-stdin ...`.",
        retryable=False,
    )


def _ssl_context(backend: Mapping[str, Any]) -> ssl.SSLContext | None:
    config = _backend_config(backend)
    insecure = bool(config.get("insecure"))
    if insecure:
        return ssl._create_unverified_context()
    cert = backend_value({**backend, **config}, "certificate")
    if not cert:
        return None
    context = ssl.create_default_context()
    cert_path = Path(cert).expanduser()
    if cert_path.exists():
        context.load_verify_locations(cafile=str(cert_path))
    else:
        context.load_verify_locations(cadata=cert)
    return context


class LndRestClient:
    def __init__(self, backend: Mapping[str, Any]):
        self.backend = backend
        self.base_url = (backend.get("url") or "").rstrip("/")
        if not self.base_url:
            raise AppError("LND backend URL is required", code="config_error")
        self.timeout = backend_timeout(backend)
        self.macaroon = _macaroon_hex(backend)
        self.context = _ssl_context(backend)

    def request(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, Any] | None = None,
        payload: Mapping[str, Any] | None = None,
    ) -> dict[str, Any]:
        query = ""
        if params:
            query = "?" + urlparse.urlencode(
                {k: v for k, v in params.items() if v not in (None, "")}
            )
        data = json.dumps(payload or {}).encode("utf-8") if payload is not None else None
        request = urlrequest.Request(
            f"{self.base_url}{path}{query}",
            data=data,
            method=method,
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "Grpc-Metadata-macaroon": self.macaroon,
                "User-Agent": f"{APP_NAME}/{__version__}",
            },
        )
        try:
            with urlrequest.urlopen(request, timeout=self.timeout, context=self.context) as response:
                raw = response.read().decode("utf-8")
        except urlerror.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            if exc.code == 403:
                hint = "Check that the macaroon has read-only permissions for LND info, offchain, onchain, invoices, and routing history."
            else:
                hint = "Check the LND REST URL, TLS certificate, and macaroon."
            raise AppError(
                f"LND returned HTTP {exc.code}: {detail[:200]}",
                code="backend_error",
                hint=hint,
                retryable=500 <= exc.code < 600,
            ) from exc
        except urlerror.URLError as exc:
            raise AppError(
                f"Failed to reach LND backend: {exc.reason}",
                code="backend_error",
                hint="Check the LND REST host, port, TLS certificate, and network reachability.",
                retryable=True,
            ) from exc
        if not raw:
            return {}
        try:
            payload_obj = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise AppError(
                "LND returned non-JSON data",
                code="backend_error",
                details={"response_preview": raw[:120]},
                retryable=True,
            ) from exc
        if not isinstance(payload_obj, dict):
            raise AppError(
                "LND returned an unexpected response shape",
                code="backend_error",
                details={"response_type": type(payload_obj).__name__},
                retryable=True,
            )
        return payload_obj

    def get(self, path: str, *, params: Mapping[str, Any] | None = None) -> dict[str, Any]:
        return self.request("GET", path, params=params)

    def post(self, path: str, payload: Mapping[str, Any] | None = None) -> dict[str, Any]:
        return self.request("POST", path, payload=payload or {})


def _upsert_channel(
    conn,
    workspace,
    profile,
    backend_name: str,
    row: Mapping[str, Any],
    *,
    closed: bool = False,
) -> None:
    channel_point = (
        row.get("channel_point")
        or row.get("funding_txid")
        or row.get("closing_tx_hash")
    )
    chan_id = str(row.get("chan_id") or "") or None
    key = _stable_key(
        "closed" if closed else "open",
        chan_id,
        channel_point,
        row.get("remote_pubkey"),
    )
    if not key:
        return
    ts = now_iso()
    close_type = row.get("close_type") if closed else None
    closed_at = (
        _timestamp(row.get("close_time") or row.get("close_timestamp"))
        if closed
        else None
    )
    conn.execute(
        """
        INSERT INTO lnd_channels(
            id, workspace_id, profile_id, backend_name, stable_key, chan_id,
            channel_point, remote_pubkey, capacity_msat, local_balance_msat,
            remote_balance_msat, commit_fee_msat, active, private, opened_at,
            closed_at, close_type, raw_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, stable_key) DO UPDATE SET
            chan_id = excluded.chan_id,
            channel_point = excluded.channel_point,
            remote_pubkey = excluded.remote_pubkey,
            capacity_msat = excluded.capacity_msat,
            local_balance_msat = excluded.local_balance_msat,
            remote_balance_msat = excluded.remote_balance_msat,
            commit_fee_msat = excluded.commit_fee_msat,
            active = excluded.active,
            private = excluded.private,
            closed_at = COALESCE(excluded.closed_at, lnd_channels.closed_at),
            close_type = COALESCE(excluded.close_type, lnd_channels.close_type),
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            key,
            chan_id,
            channel_point,
            row.get("remote_pubkey"),
            _msat(row.get("capacity"), sats=True),
            _msat(row.get("local_balance"), sats=True),
            _msat(row.get("remote_balance"), sats=True),
            _msat(row.get("commit_fee"), sats=True),
            1 if row.get("active") else 0,
            1 if row.get("private") else 0,
            None,
            closed_at,
            close_type,
            _json(row),
            ts,
            ts,
        ),
    )


def _upsert_forward(conn, workspace, profile, backend_name: str, row: Mapping[str, Any]) -> None:
    occurred_at = _timestamp(row.get("timestamp")) or now_iso()
    key = _stable_key(
        row.get("timestamp"),
        row.get("timestamp_ns"),
        row.get("chan_id_in"),
        row.get("chan_id_out"),
        row.get("amt_in_msat") or row.get("amt_in"),
        row.get("amt_out_msat") or row.get("amt_out"),
        row.get("fee_msat") or row.get("fee"),
    )
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO lnd_forwards(
            id, workspace_id, profile_id, backend_name, stable_key, occurred_at,
            chan_id_in, chan_id_out, amount_in_msat, amount_out_msat, fee_msat,
            raw_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, stable_key) DO UPDATE SET
            occurred_at = excluded.occurred_at,
            chan_id_in = excluded.chan_id_in,
            chan_id_out = excluded.chan_id_out,
            amount_in_msat = excluded.amount_in_msat,
            amount_out_msat = excluded.amount_out_msat,
            fee_msat = excluded.fee_msat,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            key,
            occurred_at,
            str(row.get("chan_id_in") or "") or None,
            str(row.get("chan_id_out") or "") or None,
            _msat(
                row.get("amt_in_msat") or row.get("amt_in"),
                sats=not row.get("amt_in_msat"),
            ),
            _msat(
                row.get("amt_out_msat") or row.get("amt_out"),
                sats=not row.get("amt_out_msat"),
            ),
            _msat(row.get("fee_msat") or row.get("fee"), sats=not row.get("fee_msat")),
            _json(row),
            ts,
            ts,
        ),
    )


def _payment_fee_msat(row: Mapping[str, Any]) -> int:
    if row.get("fee_msat") not in (None, ""):
        return _msat(row.get("fee_msat"))
    if row.get("fee_sat") not in (None, ""):
        return _msat(row.get("fee_sat"), sats=True)
    return _msat(row.get("fee"), sats=True)


def _upsert_payment(conn, workspace, profile, backend_name: str, row: Mapping[str, Any]) -> None:
    key = _stable_key(
        row.get("payment_index"),
        row.get("payment_hash"),
        row.get("creation_time_ns"),
    )
    if not key:
        return
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO lnd_payments(
            id, workspace_id, profile_id, backend_name, stable_key, payment_hash,
            occurred_at, status, value_msat, fee_msat, classification,
            raw_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, stable_key) DO UPDATE SET
            payment_hash = excluded.payment_hash,
            occurred_at = excluded.occurred_at,
            status = excluded.status,
            value_msat = excluded.value_msat,
            fee_msat = excluded.fee_msat,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            key,
            row.get("payment_hash"),
            _timestamp(row.get("creation_date")),
            row.get("status"),
            _msat(
                row.get("value_msat") or row.get("value"),
                sats=not row.get("value_msat"),
            ),
            _payment_fee_msat(row),
            "unclassified",
            _json(row),
            ts,
            ts,
        ),
    )


def _upsert_invoice(conn, workspace, profile, backend_name: str, row: Mapping[str, Any]) -> None:
    r_hash = row.get("r_hash") or row.get("payment_hash")
    if isinstance(r_hash, str):
        payment_hash = r_hash
    else:
        payment_hash = None
    key = _stable_key(row.get("add_index"), payment_hash, row.get("payment_request"))
    if not key:
        return
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO lnd_invoices(
            id, workspace_id, profile_id, backend_name, stable_key, payment_hash,
            created_at_ts, settled_at, settled, value_msat, amount_paid_msat,
            memo, raw_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, stable_key) DO UPDATE SET
            payment_hash = excluded.payment_hash,
            created_at_ts = excluded.created_at_ts,
            settled_at = excluded.settled_at,
            settled = excluded.settled,
            value_msat = excluded.value_msat,
            amount_paid_msat = excluded.amount_paid_msat,
            memo = excluded.memo,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            key,
            payment_hash,
            _timestamp(row.get("creation_date")),
            _timestamp(row.get("settle_date")),
            1 if row.get("settled") else 0,
            _msat(
                row.get("value_msat") or row.get("value"),
                sats=not row.get("value_msat"),
            ),
            _msat(
                row.get("amt_paid_msat") or row.get("amt_paid_sat"),
                sats=not row.get("amt_paid_msat"),
            ),
            row.get("memo"),
            _json(row),
            ts,
            ts,
        ),
    )


def _upsert_wallet_tx(conn, workspace, profile, backend_name: str, row: Mapping[str, Any]) -> None:
    tx_hash = row.get("tx_hash")
    key = _stable_key(tx_hash, row.get("block_height"), row.get("time_stamp"))
    if not key:
        return
    ts = now_iso()
    conn.execute(
        """
        INSERT INTO lnd_wallet_transactions(
            id, workspace_id, profile_id, backend_name, stable_key, tx_hash,
            occurred_at, block_height, amount_msat, fee_msat, raw_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(profile_id, backend_name, stable_key) DO UPDATE SET
            tx_hash = excluded.tx_hash,
            occurred_at = excluded.occurred_at,
            block_height = excluded.block_height,
            amount_msat = excluded.amount_msat,
            fee_msat = excluded.fee_msat,
            raw_json = excluded.raw_json,
            updated_at = excluded.updated_at
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            key,
            tx_hash,
            _timestamp(row.get("time_stamp")),
            _int(row.get("block_height")),
            _msat(row.get("amount"), sats=True),
            _msat(row.get("total_fees"), sats=True),
            _json(row),
            ts,
            ts,
        ),
    )


def _insert_snapshot(conn, workspace, profile, backend_name: str, raw: Mapping[str, Any]) -> None:
    captured_at = now_iso()
    channel = raw.get("channel_balance") if isinstance(raw.get("channel_balance"), dict) else {}
    wallet = raw.get("wallet_balance") if isinstance(raw.get("wallet_balance"), dict) else {}
    fees = raw.get("fee_report") if isinstance(raw.get("fee_report"), dict) else {}
    conn.execute(
        """
        INSERT OR IGNORE INTO lnd_snapshots(
            id, workspace_id, profile_id, backend_name, snapshot_type, captured_at,
            local_balance_msat, remote_balance_msat, wallet_confirmed_msat,
            routing_fee_24h_msat, routing_fee_7d_msat, routing_fee_30d_msat,
            raw_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            str(uuid.uuid4()),
            workspace["id"],
            profile["id"],
            backend_name,
            "fees_balances",
            captured_at,
            _msat(
                channel.get("local_balance", {}).get("sat")
                if isinstance(channel.get("local_balance"), dict)
                else channel.get("balance"),
                sats=True,
            ),
            _msat(
                channel.get("remote_balance", {}).get("sat")
                if isinstance(channel.get("remote_balance"), dict)
                else channel.get("remote_balance"),
                sats=True,
            ),
            _msat(wallet.get("confirmed_balance"), sats=True),
            _msat(fees.get("day_fee_sum"), sats=True),
            _msat(fees.get("week_fee_sum"), sats=True),
            _msat(fees.get("month_fee_sum"), sats=True),
            _json(raw),
        ),
    )


def _sync_list(
    client: LndRestClient,
    path: str,
    key: str,
    *,
    params: Mapping[str, Any] | None = None,
) -> list[dict[str, Any]]:
    payload = client.get(path, params=params)
    rows = payload.get(key) or []
    return [row for row in rows if isinstance(row, dict)]


def _funding_txid(channel_point: str | None) -> str | None:
    if not channel_point:
        return None
    return channel_point.split(":", 1)[0]


def sync_lnd_backend(
    conn,
    workspace,
    profile,
    backend: Mapping[str, Any],
    *,
    page_size: int = LND_DEFAULT_PAGE_SIZE,
    commit: bool = True,
) -> dict[str, Any]:
    if str(backend.get("kind") or "").lower() != "lnd":
        raise AppError(
            f"Backend '{backend.get('name')}' has kind '{backend.get('kind')}', expected 'lnd'",
            code="validation",
            retryable=False,
        )
    page_size = max(
        1,
        min(int(page_size or LND_DEFAULT_PAGE_SIZE), LND_MAX_PAGE_SIZE),
    )
    backend_name = str(backend["name"])
    client = LndRestClient(backend)
    summary = {dataset: 0 for dataset in LND_DATASETS}
    info = client.get("/v1/getinfo")

    for row in _sync_list(client, "/v1/channels", "channels"):
        _upsert_channel(conn, workspace, profile, backend_name, row)
        summary["channels"] += 1
    _save_cursor(conn, profile["id"], backend_name, "channels", "full", {"mode": "full"})

    for row in _sync_list(client, "/v1/channels/closed", "channels"):
        _upsert_channel(conn, workspace, profile, backend_name, row, closed=True)
        summary["closed_channels"] += 1
    _save_cursor(conn, profile["id"], backend_name, "closed_channels", "full", {"mode": "full"})

    current_forward = _int(_cursor(conn, profile["id"], backend_name, "forwards"))
    forwarding_payload: dict[str, Any] = {}
    while True:
        forwarding_payload = client.post(
            "/v1/switch",
            {
                "index_offset": current_forward,
                "num_max_events": page_size,
            },
        )
        forwards = [
            row
            for row in forwarding_payload.get("forwarding_events") or []
            if isinstance(row, dict)
        ]
        for row in forwards:
            _upsert_forward(conn, workspace, profile, backend_name, row)
        summary["forwards"] += len(forwards)
        next_forward = _int(forwarding_payload.get("last_offset_index"), current_forward)
        if not forwards or next_forward <= current_forward or len(forwards) < page_size:
            current_forward = max(current_forward, next_forward)
            break
        current_forward = next_forward
    _save_cursor(conn, profile["id"], backend_name, "forwards", current_forward, forwarding_payload)

    current_payment = _int(_cursor(conn, profile["id"], backend_name, "payments"))
    payments_payload: dict[str, Any] = {}
    while True:
        payments_payload = client.get(
            "/v1/payments",
            params={
                "include_incomplete": "true",
                "index_offset": current_payment,
                "max_payments": page_size,
            },
        )
        payments = [
            row for row in payments_payload.get("payments") or [] if isinstance(row, dict)
        ]
        for row in payments:
            _upsert_payment(conn, workspace, profile, backend_name, row)
        summary["payments"] += len(payments)
        next_payment = _int(
            payments_payload.get("last_index_offset"),
            max((_int(row.get("payment_index")) for row in payments), default=current_payment),
        )
        if not payments or next_payment <= current_payment or len(payments) < page_size:
            current_payment = max(current_payment, next_payment)
            break
        current_payment = next_payment
    _save_cursor(conn, profile["id"], backend_name, "payments", current_payment, payments_payload)

    current_invoice = _int(_cursor(conn, profile["id"], backend_name, "invoices"))
    invoices_payload: dict[str, Any] = {}
    while True:
        invoices_payload = client.get(
            "/v1/invoices",
            params={
                "index_offset": current_invoice,
                "num_max_invoices": page_size,
            },
        )
        invoices = [
            row for row in invoices_payload.get("invoices") or [] if isinstance(row, dict)
        ]
        for row in invoices:
            _upsert_invoice(conn, workspace, profile, backend_name, row)
        summary["invoices"] += len(invoices)
        next_invoice = _int(
            invoices_payload.get("last_index_offset"),
            max((_int(row.get("add_index")) for row in invoices), default=current_invoice),
        )
        if not invoices or next_invoice <= current_invoice or len(invoices) < page_size:
            current_invoice = max(current_invoice, next_invoice)
            break
        current_invoice = next_invoice
    _save_cursor(conn, profile["id"], backend_name, "invoices", current_invoice, invoices_payload)

    for row in _sync_list(client, "/v1/transactions", "transactions"):
        _upsert_wallet_tx(conn, workspace, profile, backend_name, row)
        summary["wallet_transactions"] += 1
    _save_cursor(conn, profile["id"], backend_name, "wallet_transactions", "full", {"mode": "full"})

    snapshot_raw = {
        "wallet_balance": client.get("/v1/balance/blockchain"),
        "channel_balance": client.get("/v1/balance/channels"),
        "fee_report": client.get("/v1/fees"),
    }
    _insert_snapshot(conn, workspace, profile, backend_name, snapshot_raw)
    summary["snapshots"] = 1
    _save_cursor(conn, profile["id"], backend_name, "snapshots", now_iso(), {"mode": "latest"})

    if commit:
        conn.commit()
    return {
        "backend": backend_name,
        "backend_kind": "lnd",
        "identity_pubkey": info.get("identity_pubkey"),
        "alias": info.get("alias"),
        "synced_to_chain": info.get("synced_to_chain"),
        "synced_to_graph": info.get("synced_to_graph"),
        "page_size": page_size,
        "datasets": summary,
        "status": "synced",
    }


def lnd_status(conn, profile, backend_name: str | None = None) -> dict[str, Any]:
    params: list[Any] = [profile["id"]]
    filter_sql = ""
    if backend_name:
        filter_sql = " AND backend_name = ?"
        params.append(backend_name)
    rows = conn.execute(
        f"""
        SELECT backend_name, dataset, cursor_value, synced_at
        FROM lnd_sync_state
        WHERE profile_id = ?{filter_sql}
        ORDER BY backend_name ASC, dataset ASC
        """,
        params,
    ).fetchall()
    counts = {}
    for table, key in (
        ("lnd_channels", "channels"),
        ("lnd_forwards", "forwards"),
        ("lnd_payments", "payments"),
        ("lnd_invoices", "invoices"),
        ("lnd_wallet_transactions", "wallet_transactions"),
        ("lnd_snapshots", "snapshots"),
    ):
        count_params = [profile["id"]]
        count_filter = ""
        if backend_name:
            count_filter = " AND backend_name = ?"
            count_params.append(backend_name)
        row = conn.execute(
            f"SELECT COUNT(*) AS count FROM {table} WHERE profile_id = ?{count_filter}",
            count_params,
        ).fetchone()
        counts[key] = int(row["count"] or 0)
    closed_params = [profile["id"]]
    closed_filter = ""
    if backend_name:
        closed_filter = " AND backend_name = ?"
        closed_params.append(backend_name)
    row = conn.execute(
        f"""
        SELECT COUNT(*) AS count
        FROM lnd_channels
        WHERE profile_id = ?{closed_filter}
          AND (close_type IS NOT NULL OR stable_key LIKE 'closed:%')
        """,
        closed_params,
    ).fetchone()
    counts["closed_channels"] = int(row["count"] or 0)
    return {
        "backend": backend_name,
        "cursors": [dict(row) for row in rows],
        "counts": counts,
        "datasets": list(LND_DATASETS),
    }


def lnd_profitability_report(conn, profile, backend_name: str | None = None) -> dict[str, Any]:
    params: list[Any] = [profile["id"]]
    filter_sql = ""
    if backend_name:
        filter_sql = " AND backend_name = ?"
        params.append(backend_name)
    summary_row = conn.execute(
        f"""
        SELECT
            COALESCE((SELECT SUM(fee_msat) FROM lnd_forwards WHERE profile_id = ?{filter_sql}), 0) AS routing_fees_earned_msat,
            COALESCE((SELECT SUM(fee_msat) FROM lnd_payments WHERE profile_id = ?{filter_sql} AND (status IS NULL OR status = '' OR lower(status) IN ('succeeded', 'success'))), 0) AS payment_fees_paid_msat,
            COALESCE((SELECT SUM(ABS(fee_msat)) FROM lnd_wallet_transactions WHERE profile_id = ?{filter_sql}), 0) AS wallet_fees_paid_msat
        """,
        params + params + params,
    ).fetchone()
    routing = int(summary_row["routing_fees_earned_msat"] or 0)
    payment_fees = int(summary_row["payment_fees_paid_msat"] or 0)
    wallet_fees = int(summary_row["wallet_fees_paid_msat"] or 0)

    channel_params = [profile["id"]]
    channel_filter = ""
    if backend_name:
        channel_filter = " AND c.backend_name = ?"
        channel_params.append(backend_name)
    channel_rows = conn.execute(
        f"""
        SELECT
            c.backend_name,
            c.chan_id,
            c.channel_point,
            c.remote_pubkey,
            c.capacity_msat,
            c.local_balance_msat,
            c.remote_balance_msat,
            c.commit_fee_msat,
            c.active,
            c.close_type,
            COALESCE(SUM(CASE WHEN f.chan_id_out = c.chan_id THEN f.fee_msat ELSE 0 END), 0) AS outbound_fees_earned_msat,
            COALESCE(SUM(CASE WHEN f.chan_id_in = c.chan_id THEN f.fee_msat ELSE 0 END), 0) AS inbound_forward_fees_msat
        FROM lnd_channels c
        LEFT JOIN lnd_forwards f
          ON f.profile_id = c.profile_id
         AND f.backend_name = c.backend_name
         AND (f.chan_id_out = c.chan_id OR f.chan_id_in = c.chan_id)
        WHERE c.profile_id = ?{channel_filter}
        GROUP BY c.id
        ORDER BY c.backend_name ASC, c.chan_id ASC
        """,
        channel_params,
    ).fetchall()
    channels = []
    for row in channel_rows:
        earned = int(row["outbound_fees_earned_msat"] or 0)
        funding_txid = _funding_txid(row["channel_point"])
        funding_wallet_fee = 0
        if funding_txid:
            wallet_params: list[Any] = [profile["id"], row["backend_name"], funding_txid]
            wallet_row = conn.execute(
                """
                SELECT SUM(ABS(fee_msat)) AS fee_msat
                FROM lnd_wallet_transactions
                WHERE profile_id = ? AND backend_name = ? AND tx_hash = ?
                """,
                wallet_params,
            ).fetchone()
            funding_wallet_fee = int(wallet_row["fee_msat"] or 0)
        lifecycle = int(row["commit_fee_msat"] or 0) + funding_wallet_fee
        net = earned - lifecycle
        channels.append(
            {
                "backend": row["backend_name"],
                "chan_id": row["chan_id"],
                "channel_point": row["channel_point"],
                "remote_pubkey": row["remote_pubkey"],
                "active": bool(row["active"]),
                "close_type": row["close_type"],
                "capacity_msat": int(row["capacity_msat"] or 0),
                "local_balance_msat": int(row["local_balance_msat"] or 0),
                "remote_balance_msat": int(row["remote_balance_msat"] or 0),
                "routing_fees_earned_msat": earned,
                "inbound_forward_fees_msat": int(row["inbound_forward_fees_msat"] or 0),
                "funding_wallet_fee_msat": funding_wallet_fee,
                "lifecycle_cost_msat": lifecycle,
                "net_profit_msat": net,
                "break_even_msat": max(lifecycle - earned, 0),
                "break_even": net >= 0,
            }
        )
    audit_rows = []
    for table, row_type, cols in (
        ("lnd_forwards", "routing_fee", "stable_key, occurred_at, chan_id_in, chan_id_out, fee_msat"),
        ("lnd_payments", "payment_fee", "stable_key, occurred_at, payment_hash, status, fee_msat"),
        ("lnd_wallet_transactions", "wallet_fee", "stable_key, occurred_at, tx_hash, block_height, fee_msat"),
    ):
        row_params = [profile["id"]]
        row_filter = ""
        if backend_name:
            row_filter = " AND backend_name = ?"
            row_params.append(backend_name)
        for row in conn.execute(f"SELECT backend_name, {cols} FROM {table} WHERE profile_id = ?{row_filter} ORDER BY occurred_at ASC, stable_key ASC", row_params).fetchall():
            payload = dict(row)
            payload["row_type"] = row_type
            audit_rows.append(payload)
    return {
        "backend": backend_name,
        "summary": {
            "routing_fees_earned_msat": routing,
            "routing_fees_earned": float(msat_to_btc(routing)),
            "payment_fees_paid_msat": payment_fees,
            "payment_fees_paid": float(msat_to_btc(payment_fees)),
            "wallet_fees_paid_msat": wallet_fees,
            "wallet_fees_paid": float(msat_to_btc(wallet_fees)),
            "net_profit_msat": routing - payment_fees - wallet_fees,
            "channel_count": len(channels),
            "break_even_channel_count": sum(1 for row in channels if row["break_even"]),
            "conservative_rebalance_classification": True,
        },
        "channels": channels,
        "audit_rows": audit_rows,
    }


def export_lnd_profitability_csv(conn, profile, file_path: str, *, backend_name: str | None = None) -> dict[str, Any]:
    report = lnd_profitability_report(conn, profile, backend_name)
    path = Path(file_path).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    fieldnames = [
        "row_type",
        "backend_name",
        "stable_key",
        "occurred_at",
        "chan_id_in",
        "chan_id_out",
        "payment_hash",
        "status",
        "tx_hash",
        "block_height",
        "fee_msat",
    ]
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        for row in report["audit_rows"]:
            writer.writerow({key: row.get(key, "") for key in fieldnames})
    return {"file": str(path), "rows": len(report["audit_rows"]), "backend": backend_name}
