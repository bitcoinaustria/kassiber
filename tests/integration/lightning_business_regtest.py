from __future__ import annotations

import json
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

from kassiber.backends import (
    load_runtime_config,
    merge_db_backends,
    resolve_effective_env_file,
)
from kassiber.db import open_db, resolve_database_path

ROOT = Path(__file__).resolve().parents[2]
WORKSPACE_LABEL = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_WORKSPACE") or "Lightning Business"
)
PROFILE_LABEL = os.environ.get("KASSIBER_LIGHTNING_BUSINESS_PROFILE") or "Merchant"
CONNECTION_LABEL = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_CONNECTION_LABEL") or "cln_merchant"
)
BACKEND_NAME = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_BACKEND_NAME") or "cln-merchant"
)
BACKUP_LND_CONNECTION_LABEL = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_CONNECTION_LABEL")
    or "lnd_merchant_backup"
)
BACKUP_LND_BACKEND_NAME = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_BACKEND_NAME")
    or "lnd-merchant-backup"
)
BACKUP_LND_URL = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_URL")
    or "https://127.0.0.1:19740"
)
BACKUP_LND_MACAROON_HEX = (
    os.environ.get("KASSIBER_LIGHTNING_BUSINESS_BACKUP_LND_MACAROON_HEX")
    or "00"
)
LN_KINDS = ("coreln", "lnd", "nwc")

FORBIDDEN_AI_KEYS = {
    "peerPubkey",
    "fundingOutpoint",
    "shortChannelId",
    "inPeerAlias",
    "outPeerAlias",
    "inShortChannelId",
    "outShortChannelId",
    "payment_preimage",
    "payment_secret",
    "failure_source_pubkey",
    "erring_node",
    "bolt11",
    "routes",
    "route",
}


def _default_home() -> Path:
    project = os.environ.get("KASSIBER_REGTEST_COMPOSE_PROJECT", "kassiber-regtest")
    safe = "".join(ch if ch.isalnum() or ch in {"-", "_"} else "-" for ch in project)
    return Path(tempfile.gettempdir()) / f"kassiber-lightning-business-{safe}"


def _run_cli(
    data_root: Path,
    *args: str,
    input_text: str | None = None,
) -> dict[str, Any]:
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        input=input_text,
        capture_output=True,
        text=True,
        check=False,
    )
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout for {args}; stderr={result.stderr}"
        )
    payload = json.loads(stdout)
    if result.returncode != 0 or payload.get("kind") == "error":
        raise AssertionError(
            f"CLI failed for {args}; code={result.returncode};"
            f" payload={payload}; stderr={result.stderr}"
        )
    return payload


def _embedded_mode() -> bool:
    value = str(os.environ.get("KASSIBER_LIGHTNING_BUSINESS_EMBEDDED") or "")
    return value.lower() in {
        "1",
        "true",
        "yes",
        "on",
    }


def _book_exists(data_root: Path) -> bool:
    if not resolve_database_path(data_root).exists():
        return False
    conn = open_db(data_root)
    try:
        row = conn.execute(
            """
            SELECT 1
            FROM wallets wl
            JOIN profiles p ON p.id = wl.profile_id
            JOIN workspaces ws ON ws.id = p.workspace_id
            WHERE wl.label = ?
              AND wl.kind = 'coreln'
              AND p.label = ?
              AND ws.label = ?
            """,
            (CONNECTION_LABEL, PROFILE_LABEL, WORKSPACE_LABEL),
        ).fetchone()
        return row is not None
    finally:
        conn.close()


def _ensure_scope(data_root: Path) -> None:
    data_root.parent.mkdir(parents=True, exist_ok=True)
    if not resolve_database_path(data_root).exists():
        _run_cli(data_root, "init")

    conn = open_db(data_root)
    try:
        workspace = conn.execute(
            "SELECT id FROM workspaces WHERE lower(label) = lower(?)",
            (WORKSPACE_LABEL,),
        ).fetchone()
        profile = None
        if workspace is not None:
            profile = conn.execute(
                """
                SELECT id
                FROM profiles
                WHERE workspace_id = ? AND lower(label) = lower(?)
                """,
                (workspace["id"], PROFILE_LABEL),
            ).fetchone()
    finally:
        conn.close()

    if workspace is None:
        _run_cli(data_root, "workspaces", "create", WORKSPACE_LABEL)
    if profile is None:
        _run_cli(
            data_root,
            "profiles",
            "create",
            PROFILE_LABEL,
            "--workspace",
            WORKSPACE_LABEL,
            "--fiat-currency",
            "EUR",
            "--tax-country",
            "generic",
            "--gains-algorithm",
            "FIFO",
        )


def _ensure_backend(data_root: Path, merchant_cli: Path) -> None:
    conn = open_db(data_root)
    try:
        row = conn.execute(
            "SELECT kind, config_json FROM backends WHERE lower(name) = lower(?)",
            (BACKEND_NAME,),
        ).fetchone()
    finally:
        conn.close()
    if row is not None:
        if str(row["kind"] or "").lower() != "coreln":
            raise AssertionError(
                f"Backend {BACKEND_NAME!r} exists with kind {row['kind']!r}, expected coreln"
            )
        try:
            config = json.loads(row["config_json"] or "{}")
        except ValueError:
            config = {}
        if config.get("lightning_cli") != str(merchant_cli):
            _run_cli(
                data_root,
                "backends",
                "update",
                BACKEND_NAME,
                "--lightning-cli",
                str(merchant_cli),
            )
        return
    _run_cli(
        data_root,
        "backends",
        "create",
        BACKEND_NAME,
        "--kind",
        "coreln",
        "--url",
        "cln://local",
        "--network",
        "regtest",
        "--timeout",
        "90",
        "--lightning-cli",
        str(merchant_cli),
    )


def _ensure_backup_lnd_source(data_root: Path) -> None:
    conn = open_db(data_root)
    try:
        backend = conn.execute(
            "SELECT kind, url FROM backends WHERE lower(name) = lower(?)",
            (BACKUP_LND_BACKEND_NAME,),
        ).fetchone()
        wallet = conn.execute(
            """
            SELECT wl.kind
            FROM wallets wl
            JOIN profiles p ON p.id = wl.profile_id
            JOIN workspaces ws ON ws.id = p.workspace_id
            WHERE lower(wl.label) = lower(?)
              AND p.label = ?
              AND ws.label = ?
            """,
            (BACKUP_LND_CONNECTION_LABEL, PROFILE_LABEL, WORKSPACE_LABEL),
        ).fetchone()
    finally:
        conn.close()

    if backend is not None:
        if str(backend["kind"] or "").lower() != "lnd":
            raise AssertionError(
                f"Backend {BACKUP_LND_BACKEND_NAME!r} exists with kind "
                f"{backend['kind']!r}, expected lnd"
            )
        _run_cli(
            data_root,
            "backends",
            "update",
            BACKUP_LND_BACKEND_NAME,
            "--url",
            BACKUP_LND_URL,
            "--network",
            "regtest",
            "--timeout",
            "15",
            "--insecure",
            "true",
            "--token-stdin",
            input_text=BACKUP_LND_MACAROON_HEX,
        )
    else:
        _run_cli(
            data_root,
            "backends",
            "create",
            BACKUP_LND_BACKEND_NAME,
            "--kind",
            "lnd",
            "--url",
            BACKUP_LND_URL,
            "--network",
            "regtest",
            "--timeout",
            "15",
            "--insecure",
            "true",
            "--token-stdin",
            "--notes",
            "Demo backup LND source for wallet-list and node UI coverage.",
            input_text=BACKUP_LND_MACAROON_HEX,
        )

    if wallet is not None:
        if str(wallet["kind"] or "").lower() != "lnd":
            raise AssertionError(
                f"Wallet {BACKUP_LND_CONNECTION_LABEL!r} exists with kind "
                f"{wallet['kind']!r}, expected lnd"
            )
        return

    _run_cli(
        data_root,
        "wallets",
        "create",
        "--workspace",
        WORKSPACE_LABEL,
        "--profile",
        PROFILE_LABEL,
        "--label",
        BACKUP_LND_CONNECTION_LABEL,
        "--kind",
        "lnd",
        "--backend",
        BACKUP_LND_BACKEND_NAME,
        "--network",
        "regtest",
    )


def _build_book(data_root: Path, merchant_cli: Path) -> None:
    _ensure_scope(data_root)
    _ensure_backend(data_root, merchant_cli)
    _ensure_backup_lnd_source(data_root)
    _run_cli(
        data_root,
        "wallets",
        "create",
        "--workspace",
        WORKSPACE_LABEL,
        "--profile",
        PROFILE_LABEL,
        "--label",
        CONNECTION_LABEL,
        "--kind",
        "coreln",
        "--backend",
        BACKEND_NAME,
    )


def _read_payload_timeout(
    proc: subprocess.Popen[str], timeout: float = 10.0
) -> dict[str, Any]:
    assert proc.stdout is not None
    ready, _, _ = select.select([proc.stdout.fileno()], [], [], timeout)
    if not ready:
        raise AssertionError(f"daemon did not emit within {timeout:.1f}s")
    return json.loads(proc.stdout.readline())


def _write_payload(proc: subprocess.Popen[str], payload: dict[str, Any]) -> None:
    assert proc.stdin is not None
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()


def _shutdown_daemon_process(proc: subprocess.Popen[str], timeout: float = 10.0) -> None:
    shutdown_error: Exception | None = None
    if proc.poll() is None:
        try:
            _write_payload(
                proc,
                {"request_id": "shutdown-1", "kind": "daemon.shutdown"},
            )
            _read_payload_timeout(proc, timeout)
        except Exception as exc:
            # Shutdown RPC is best-effort during teardown; wait/kill below owns cleanup.
            shutdown_error = exc
    if proc.stdin is not None:
        try:
            proc.stdin.close()
        except (BrokenPipeError, OSError):
            # The daemon may close its stdin first while exiting.
            pass
    try:
        code = proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.kill()
        code = proc.wait(timeout=timeout)
    stderr = proc.stderr.read() if proc.stderr is not None else ""
    if proc.stdout is not None:
        proc.stdout.close()
    if proc.stderr is not None:
        proc.stderr.close()
    if code != 0:
        suffix = (
            f"; shutdown_error={shutdown_error!r}" if shutdown_error is not None else ""
        )
        raise AssertionError(f"daemon exited with {code}; stderr={stderr}{suffix}")


def _daemon_snapshot(data_root: Path, connection_label: str = CONNECTION_LABEL) -> dict[str, Any]:
    proc = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "daemon",
        ],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        ready = _read_payload_timeout(proc, 30.0)
        if ready.get("kind") != "daemon.ready":
            raise AssertionError(f"expected daemon.ready, got {ready}")
        _write_payload(
            proc,
            {
                "request_id": "ln-snapshot-1",
                "kind": "ui.connections.node.snapshot",
                "args": {"connection": connection_label, "window_days": 30},
            },
        )
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            payload = _read_payload_timeout(proc, 10.0)
            if payload.get("request_id") != "ln-snapshot-1":
                continue
            if payload.get("kind") == "error":
                raise AssertionError(f"daemon snapshot failed: {payload}")
            if payload.get("kind") == "ui.connections.node.snapshot":
                return payload["data"]
        raise AssertionError("daemon did not return ui.connections.node.snapshot")
    finally:
        _shutdown_daemon_process(proc)


def _all_keys(value: Any) -> set[str]:
    keys: set[str] = set()
    if isinstance(value, dict):
        for key, item in value.items():
            keys.add(str(key))
            keys.update(_all_keys(item))
    elif isinstance(value, list):
        for item in value:
            keys.update(_all_keys(item))
    return keys


def _runtime_config_for(data_root: Path) -> dict[str, object]:
    env_file = resolve_effective_env_file(data_root=str(data_root))
    runtime_config = load_runtime_config(env_file)
    conn = open_db(data_root)
    try:
        merge_db_backends(conn, runtime_config)
    finally:
        conn.close()
    return runtime_config


def _assert_db_state(data_root: Path) -> dict[str, int]:
    conn = open_db(data_root)
    try:
        ln_wallets = [
            dict(row)
            for row in conn.execute(
                "SELECT label, kind, config_json FROM wallets"
                f" WHERE kind IN ({','.join('?' for _ in LN_KINDS)})"
                " ORDER BY label",
                LN_KINDS,
            )
        ]
        expected_ln_wallets = {
            (CONNECTION_LABEL, "coreln"),
            (BACKUP_LND_CONNECTION_LABEL, "lnd"),
        }
        actual_ln_wallets = {
            (str(row["label"] or ""), str(row["kind"] or "").lower())
            for row in ln_wallets
        }
        if actual_ln_wallets != expected_ln_wallets:
            raise AssertionError(
                f"unexpected LN wallets: expected {expected_ln_wallets}, got {ln_wallets}"
            )
        all_wallets = [
            dict(row)
            for row in conn.execute("SELECT label, kind FROM wallets ORDER BY label")
        ]
        if not _embedded_mode():
            actual_wallets = {
                (str(row["label"] or ""), str(row["kind"] or "").lower())
                for row in all_wallets
            }
            if actual_wallets != expected_ln_wallets:
                raise AssertionError(
                    f"expected only merchant LN wallets, got {all_wallets}"
                )
        for forbidden in ("customer", "supplier", "router"):
            for wallet in ln_wallets:
                if forbidden in wallet["label"].lower():
                    raise AssertionError(
                        f"scenario actor leaked into wallet label: {wallet}"
                    )

        backend_rows = [
            dict(row)
            for row in conn.execute(
                "SELECT name, kind FROM backends WHERE kind = 'coreln' ORDER BY name"
            )
        ]
        if backend_rows != [{"name": BACKEND_NAME, "kind": "coreln"}]:
            raise AssertionError(f"unexpected Core Lightning backends: {backend_rows}")
        lnd_backend_rows = [
            dict(row)
            for row in conn.execute(
                "SELECT name, kind FROM backends WHERE kind = 'lnd' ORDER BY name"
            )
        ]
        if lnd_backend_rows != [{"name": BACKUP_LND_BACKEND_NAME, "kind": "lnd"}]:
            raise AssertionError(f"unexpected LND backends: {lnd_backend_rows}")

        counts = {
            row["record_type"]: int(row["count"])
            for row in conn.execute(
                "SELECT record_type, COUNT(*) AS count"
                " FROM lightning_node_records GROUP BY record_type"
            )
        }
        if counts.get("forward_day", 0) < 1:
            raise AssertionError(f"expected forward_day records, got {counts}")
        if counts.get("pay", 0) < 2:
            raise AssertionError(f"expected merchant pay records, got {counts}")
        if counts.get("income", 0) < 3:
            raise AssertionError(f"expected merchant invoice income records, got {counts}")
        if counts.get("balance_snapshot", 0) < 1:
            raise AssertionError(f"expected balance snapshot records, got {counts}")

        tx_count = conn.execute(
            "SELECT COUNT(*) AS count FROM transactions WHERE kind = 'cln_invoice'"
        ).fetchone()["count"]
        if int(tx_count) < 5:
            raise AssertionError(f"expected synced CLN invoice transactions, got {tx_count}")

        raw_leaks = conn.execute(
            "SELECT COUNT(*) AS count FROM lightning_node_records"
            " WHERE raw_json != '{}' AND record_type != 'forward_day'"
        ).fetchone()["count"]
        if int(raw_leaks) != 0:
            raise AssertionError("Lightning raw RPC payloads leaked into persistence")
        return counts
    finally:
        conn.close()


def _assert_snapshot(snapshot: dict[str, Any]) -> None:
    if snapshot.get("alias") != "kassiber-merchant":
        raise AssertionError(f"unexpected merchant alias: {snapshot.get('alias')}")
    if len(str(snapshot.get("pubkey") or "")) < 66:
        raise AssertionError("merchant pubkey missing from desktop snapshot")
    if snapshot.get("network") != "regtest":
        raise AssertionError(f"expected regtest snapshot, got {snapshot.get('network')}")
    if int(snapshot.get("totalCapacitySat") or 0) <= 0:
        raise AssertionError("merchant channel capacity missing")
    if int(snapshot.get("totalLocalBalanceSat") or 0) <= 0:
        raise AssertionError("merchant local liquidity missing")
    if int(snapshot.get("totalRemoteBalanceSat") or 0) <= 0:
        raise AssertionError("merchant remote liquidity missing")
    if int(snapshot.get("onchainBalanceSat") or 0) <= 0:
        raise AssertionError("merchant on-chain balance missing after L1 scenario")
    if len(snapshot.get("channels") or []) < 2:
        raise AssertionError(f"expected merchant channels, got {snapshot.get('channels')}")
    if int(snapshot.get("paidInvoiceCount") or 0) < 5:
        raise AssertionError(f"merchant paid invoices missing from snapshot: {snapshot}")
    if int(snapshot.get("completedPaymentCount") or 0) < 2:
        raise AssertionError(f"merchant payments missing from snapshot: {snapshot}")
    if int(snapshot.get("expiredInvoiceCount") or 0) < 1:
        raise AssertionError(f"merchant expired invoice missing from snapshot: {snapshot}")
    if int(snapshot.get("failedPaymentCount") or 0) < 1:
        raise AssertionError(f"merchant failed payment missing from snapshot: {snapshot}")
    routing = snapshot.get("routing") or {}
    if int(routing.get("forwardCount") or 0) < 1:
        raise AssertionError(f"merchant routing summary missing forwards: {routing}")
    if int(routing.get("routingRevenueSat") or 0) <= 0:
        raise AssertionError(f"merchant routing revenue missing: {routing}")
    if int(routing.get("paymentCostSat") or 0) <= 0:
        raise AssertionError(f"merchant payment costs missing: {routing}")
    if not snapshot.get("forwards"):
        raise AssertionError("merchant forward rows missing from snapshot")
    for channel in snapshot.get("channels") or []:
        if channel.get("isPrivate") and channel.get("peerPubkey") is not None:
            raise AssertionError(f"private channel leaked peer pubkey: {channel}")


def _assert_lnd_snapshot(snapshot: dict[str, Any]) -> None:
    if snapshot.get("alias") != "kassiber-lnd-backup":
        raise AssertionError(f"unexpected LND alias: {snapshot.get('alias')}")
    if len(str(snapshot.get("pubkey") or "")) < 66:
        raise AssertionError("LND pubkey missing from desktop snapshot")
    if snapshot.get("network") != "regtest":
        raise AssertionError(f"expected LND regtest snapshot, got {snapshot.get('network')}")
    if int(snapshot.get("totalCapacitySat") or 0) <= 0:
        raise AssertionError("LND channel capacity missing")
    if len(snapshot.get("channels") or []) < 2:
        raise AssertionError(f"expected LND backup channels, got {snapshot.get('channels')}")
    if int(snapshot.get("paidInvoiceCount") or 0) < 1:
        raise AssertionError(f"expected paid LND invoices, got {snapshot}")
    if int(snapshot.get("completedPaymentCount") or 0) < 1:
        raise AssertionError(f"expected completed LND payments, got {snapshot}")


def _assert_profitability(report: dict[str, Any], csv_file: Path) -> None:
    summary = report.get("summary") or {}
    if int(summary.get("forwardCount") or 0) < 1:
        raise AssertionError(f"profitability report missing forwards: {summary}")
    if int(summary.get("routingRevenueSat") or 0) <= 0:
        raise AssertionError(f"profitability report missing routing revenue: {summary}")
    if int(summary.get("paymentCostSat") or 0) <= 0:
        raise AssertionError(f"profitability report missing payment costs: {summary}")
    channels = report.get("channels") or []
    if len(channels) < 2:
        raise AssertionError(f"profitability report missing channel rows: {report}")
    for channel in channels:
        if int(channel.get("capacitySat") or 0) <= 0:
            raise AssertionError(f"channel liquidity missing: {channel}")
        if "openCostSat" not in channel or "coversOpenCost" not in channel:
            raise AssertionError(f"open-cost coverage missing: {channel}")
    text = csv_file.read_text(encoding="utf-8")
    for needle in ("routing_revenue", "payment_cost", "forward_count", "open_cost="):
        if needle not in text:
            raise AssertionError(f"CSV export missing {needle!r}: {csv_file}")


def _assert_ai_opsec(data_root: Path) -> None:
    from kassiber.daemon import (
        _lightning_node_snapshot_payload_for_ai,
        _lightning_profitability_payload_for_ai,
    )

    runtime_config = _runtime_config_for(data_root)
    conn = open_db(data_root)
    try:
        snapshot = _lightning_node_snapshot_payload_for_ai(
            conn,
            runtime_config,
            {"connection": CONNECTION_LABEL, "window_days": 30},
        )
        profitability = _lightning_profitability_payload_for_ai(
            conn,
            runtime_config,
            {"connection": CONNECTION_LABEL, "window_days": 30},
        )
    finally:
        conn.close()

    keys = _all_keys(snapshot) | _all_keys(profitability)
    leaked = sorted(FORBIDDEN_AI_KEYS & keys)
    if leaked:
        raise AssertionError(f"AI-safe Lightning payload leaked forbidden keys: {leaked}")
    if snapshot.get("pubkey") is not None:
        raise AssertionError("AI-safe snapshot exposed the operator node pubkey")
    if "channels" in profitability:
        raise AssertionError("AI-safe profitability exposed per-channel rows")


def run() -> dict[str, Any]:
    home = Path(os.environ.get("KASSIBER_LIGHTNING_BUSINESS_HOME") or _default_home())
    data_root = Path(
        os.environ.get("KASSIBER_LIGHTNING_BUSINESS_DATA_ROOT") or home / "data"
    )
    exports = home / "exports"
    csv_file = exports / "lightning-profitability.csv"
    merchant_cli = Path(
        os.environ.get("KASSIBER_LIGHTNING_BUSINESS_MERCHANT_CLI")
        or ROOT / "dev" / "regtest" / "lightning-cli-merchant.sh"
    )

    reuse_book = bool(
        os.environ.get("KASSIBER_LIGHTNING_BUSINESS_REUSE_BOOK")
        or os.environ.get("KASSIBER_REGTEST_LIGHTNING_REUSE")
    )
    if os.environ.get("KASSIBER_LIGHTNING_BUSINESS_REBUILD") or not reuse_book:
        shutil.rmtree(data_root, ignore_errors=True)
        shutil.rmtree(exports, ignore_errors=True)
    exports.mkdir(parents=True, exist_ok=True)

    if _book_exists(data_root):
        _ensure_backend(data_root, merchant_cli)
        _ensure_backup_lnd_source(data_root)
    else:
        _build_book(data_root, merchant_cli)

    _run_cli(
        data_root,
        "wallets",
        "sync",
        "--workspace",
        WORKSPACE_LABEL,
        "--profile",
        PROFILE_LABEL,
        "--wallet",
        CONNECTION_LABEL,
    )
    snapshot = _daemon_snapshot(data_root)
    lnd_snapshot = _daemon_snapshot(data_root, BACKUP_LND_CONNECTION_LABEL)
    report = _run_cli(
        data_root,
        "reports",
        "lightning-profitability",
        "--connection",
        CONNECTION_LABEL,
        "--window-days",
        "30",
    )["data"]
    _run_cli(
        data_root,
        "reports",
        "export-lightning-profitability-csv",
        "--connection",
        CONNECTION_LABEL,
        "--window-days",
        "30",
        "--file",
        str(csv_file),
    )

    _assert_snapshot(snapshot)
    _assert_lnd_snapshot(lnd_snapshot)
    _assert_profitability(report, csv_file)
    counts = _assert_db_state(data_root)
    _assert_ai_opsec(data_root)

    return {
        "data_root": str(data_root),
        "csv_file": str(csv_file),
        "business_plan": os.environ.get("KASSIBER_LIGHTNING_BUSINESS_PLAN"),
        "snapshot": {
            "alias": snapshot.get("alias"),
            "channels": len(snapshot.get("channels") or []),
            "forwards": len(snapshot.get("forwards") or []),
            "paid_invoices": snapshot.get("paidInvoiceCount"),
            "expired_invoices": snapshot.get("expiredInvoiceCount"),
            "completed_payments": snapshot.get("completedPaymentCount"),
            "failed_payments": snapshot.get("failedPaymentCount"),
            "onchain_balance_sat": snapshot.get("onchainBalanceSat"),
        },
        "lnd_snapshot": {
            "alias": lnd_snapshot.get("alias"),
            "channels": len(lnd_snapshot.get("channels") or []),
            "onchain_balance_sat": lnd_snapshot.get("onchainBalanceSat"),
        },
        "report_summary": report.get("summary"),
        "record_counts": counts,
    }


def main() -> None:
    print(json.dumps(run(), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
