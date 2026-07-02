from __future__ import annotations

import argparse
import base64
import binascii
import csv
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any
from urllib import error, request

DEFAULT_API_URL = "http://127.0.0.1:9001"
ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCENARIO = ROOT / "dev" / "regtest" / "scenarios" / "full_accounting.json"
_DOCKER_BASE_CMD: list[str] | None = None


class BoltzProbeError(RuntimeError):
    pass


def _api_url(value: str | None = None) -> str:
    return str(value or os.environ.get("KASSIBER_BOLTZ_API_URL") or DEFAULT_API_URL).rstrip("/")


def _get_json(api_url: str, path: str) -> dict[str, Any]:
    url = f"{api_url}{path}"
    try:
        with request.urlopen(url, timeout=10) as response:
            payload = response.read().decode("utf-8")
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise BoltzProbeError(f"Boltz API {path} failed with HTTP {exc.code}: {body}") from exc
    except OSError as exc:
        raise BoltzProbeError(f"Boltz API {path} is not reachable at {api_url}: {exc}") from exc
    try:
        decoded = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise BoltzProbeError(f"Boltz API {path} returned non-JSON: {payload!r}") from exc
    if not isinstance(decoded, dict):
        raise BoltzProbeError(f"Boltz API {path} returned {type(decoded).__name__}, expected object")
    return decoded


def _post_json(api_url: str, path: str, payload: dict[str, Any]) -> dict[str, Any]:
    url = f"{api_url}{path}"
    body = json.dumps(payload).encode("utf-8")
    req = request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with request.urlopen(req, timeout=30) as response:
            response_body = response.read().decode("utf-8")
    except error.HTTPError as exc:
        body_text = exc.read().decode("utf-8", errors="replace")
        raise BoltzProbeError(f"Boltz API {path} failed with HTTP {exc.code}: {body_text}") from exc
    except OSError as exc:
        raise BoltzProbeError(f"Boltz API {path} is not reachable at {api_url}: {exc}") from exc
    try:
        decoded = json.loads(response_body)
    except json.JSONDecodeError as exc:
        raise BoltzProbeError(f"Boltz API {path} returned non-JSON: {response_body!r}") from exc
    if not isinstance(decoded, dict):
        raise BoltzProbeError(f"Boltz API {path} returned {type(decoded).__name__}, expected object")
    return decoded


def _command_text(args: list[str], *, cwd: Path = ROOT, timeout: int = 60) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    if result.returncode != 0:
        raise BoltzProbeError(
            f"Command failed ({result.returncode}): {' '.join(args)}\n"
            f"stdout: {result.stdout.strip()}\n"
            f"stderr: {result.stderr.strip()}"
        )
    return result.stdout.strip()


def _json_from_text(text: str, label: str) -> dict[str, Any]:
    raw = text.strip()
    if not raw:
        raise BoltzProbeError(f"{label} returned no output")
    try:
        decoded = json.loads(raw)
    except json.JSONDecodeError:
        start = raw.find("{")
        end = raw.rfind("}")
        if start < 0 or end <= start:
            raise BoltzProbeError(f"{label} returned non-JSON: {raw!r}")
        decoded = json.loads(raw[start : end + 1])
    if not isinstance(decoded, dict):
        raise BoltzProbeError(f"{label} returned {type(decoded).__name__}, expected object")
    return decoded


def _docker_exec(*args: str, timeout: int = 60) -> str:
    container = os.environ.get("KASSIBER_BOLTZ_SCRIPTS_CONTAINER", "boltz-scripts")
    command = " ".join(shlex.quote(arg) for arg in args)
    return _command_text(
        [*_docker_base_cmd(), "exec", container, "bash", "-lc", command],
        timeout=timeout,
    )


def _docker_base_cmd() -> list[str]:
    global _DOCKER_BASE_CMD
    if _DOCKER_BASE_CMD is not None:
        return _DOCKER_BASE_CMD
    override = os.environ.get("KASSIBER_BOLTZ_DOCKER_CMD")
    if override:
        _DOCKER_BASE_CMD = shlex.split(override)
        return _DOCKER_BASE_CMD
    probe = subprocess.run(["docker", "info"], capture_output=True, text=True, check=False)
    _DOCKER_BASE_CMD = ["docker"] if probe.returncode == 0 else ["sudo", "-n", "docker"]
    return _DOCKER_BASE_CMD


def _docker_json(*args: str, timeout: int = 60) -> dict[str, Any]:
    return _json_from_text(_docker_exec(*args, timeout=timeout), " ".join(args))


def _elements_cli(*args: str, timeout: int = 60) -> str:
    return _docker_exec("elements-cli-sim-client", *args, timeout=timeout)


def _elements_address() -> str:
    errors = []
    for tool in ("elements-cli-sim-server", "elements-cli-sim-client"):
        try:
            return _docker_exec(tool, "getnewaddress").strip()
        except BoltzProbeError as exc:
            errors.append(str(exc))
    raise BoltzProbeError("Could not get a Liquid regtest address:\n" + "\n".join(errors))


def _mine_liquid_blocks(count: int = 1) -> None:
    address = _elements_cli("getnewaddress").strip()
    try:
        _elements_cli("generatetoaddress", str(int(count)), address, timeout=120)
    except BoltzProbeError:
        _elements_cli("-generate", str(int(count)), timeout=120)


def _lncli_json(*args: str, timeout: int = 60) -> dict[str, Any]:
    return _docker_json("lncli-sim-client", *args, timeout=timeout)


def _sats_to_btc_text(sats: int) -> str:
    value = int(sats)
    whole, frac = divmod(value, 100_000_000)
    return f"{whole}.{frac:08d}"


def _payment_hash_hex(value: Any) -> str:
    text = str(value or "").strip()
    lowered = text.lower()
    if len(lowered) == 64:
        try:
            bytes.fromhex(lowered)
            return lowered
        except ValueError:
            pass
    try:
        decoded = base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise BoltzProbeError(f"Could not decode Lightning payment hash: {value!r}") from exc
    if len(decoded) != 32:
        raise BoltzProbeError(f"Lightning payment hash has {len(decoded)} bytes, expected 32")
    return decoded.hex()


def _swap_address(swap: dict[str, Any]) -> str:
    for key in ("address", "lockupAddress", "swapAddress"):
        value = str(swap.get(key) or "").strip()
        if value:
            return value
    bip21 = str(swap.get("bip21") or "").strip()
    if bip21:
        address = bip21.split(":", 1)[-1].split("?", 1)[0]
        if address:
            return address
    raise BoltzProbeError(f"Boltz submarine swap response has no lockup address: {swap}")


def _swap_status(api_url: str, swap_id: str) -> dict[str, Any]:
    errors = []
    for path in (f"/v2/swap/{swap_id}", f"/v2/swap/submarine/{swap_id}"):
        try:
            return _get_json(api_url, path)
        except BoltzProbeError as exc:
            errors.append(str(exc))
    raise BoltzProbeError(f"Could not fetch Boltz swap status for {swap_id}:\n" + "\n".join(errors))


def _wait_for_swap_status(api_url: str, swap_id: str, *, timeout_seconds: int) -> tuple[dict[str, Any], list[str]]:
    paid_statuses = {
        "invoice.paid",
        "transaction.claim.pending",
        "transaction.claimed",
        "swap.completed",
    }
    failure_statuses = {
        "swap.expired",
        "invoice.failedToPay",
        "transaction.failed",
        "transaction.refunded",
    }
    history: list[str] = []
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        status_payload = _swap_status(api_url, swap_id)
        status = str(status_payload.get("status") or "").strip()
        if status and (not history or history[-1] != status):
            history.append(status)
        if status in paid_statuses:
            return status_payload, history
        if status in failure_statuses:
            raise BoltzProbeError(f"Boltz swap {swap_id} failed with status {status}: {status_payload}")
        time.sleep(3)
    raise BoltzProbeError(
        f"Timed out waiting for Boltz swap {swap_id} to pay the invoice; "
        f"last statuses: {history or ['<none>']}"
    )


def _pair(pairs: dict[str, Any], from_asset: str, to_asset: str) -> dict[str, Any]:
    raw = pairs.get(from_asset, {})
    if not isinstance(raw, dict):
        return {}
    pair = raw.get(to_asset, {})
    return pair if isinstance(pair, dict) else {}


def _summarize_pair(pair: dict[str, Any]) -> dict[str, Any]:
    limits = pair.get("limits") if isinstance(pair.get("limits"), dict) else {}
    fees = pair.get("fees") if isinstance(pair.get("fees"), dict) else {}
    return {
        "hash": pair.get("hash"),
        "rate": pair.get("rate"),
        "limits": {
            "minimal": limits.get("minimal"),
            "maximal": limits.get("maximal"),
            "maximal_zero_conf": limits.get("maximalZeroConf"),
        },
        "fees": fees,
    }


def _require_pair(pairs: dict[str, Any], from_asset: str, to_asset: str, label: str) -> dict[str, Any]:
    pair = _pair(pairs, from_asset, to_asset)
    if not pair:
        raise BoltzProbeError(f"Boltz {label} pairs do not include {from_asset} -> {to_asset}")
    if not pair.get("hash"):
        raise BoltzProbeError(f"Boltz {label} pair {from_asset} -> {to_asset} has no pair hash")
    return _summarize_pair(pair)


def probe_boltz_liquid(api_url: str | None = None) -> dict[str, Any]:
    """Probe the local Boltz regtest API for Liquid-capable swap pairs.

    The default probe stays lightweight so the harness can use it as a startup
    readiness check before the opt-in execution path runs.
    """

    base = _api_url(api_url)
    version = _get_json(base, "/version")
    submarine_pairs = _get_json(base, "/v2/swap/submarine")
    reverse_pairs = _get_json(base, "/v2/swap/reverse")
    chain_pairs = _get_json(base, "/v2/swap/chain")
    liquid_height = _get_json(base, "/v2/chain/L-BTC/height")
    bitcoin_height = _get_json(base, "/v2/chain/BTC/height")

    return {
        "api_url": base,
        "version": version.get("version"),
        "heights": {
            "BTC": bitcoin_height.get("height"),
            "L-BTC": liquid_height.get("height"),
        },
        "pairs": {
            "liquid_to_lightning": _require_pair(submarine_pairs, "L-BTC", "BTC", "submarine"),
            "lightning_to_liquid": _require_pair(reverse_pairs, "BTC", "L-BTC", "reverse"),
            "bitcoin_to_liquid": _require_pair(chain_pairs, "BTC", "L-BTC", "chain"),
        },
    }


def boltz_bridge_specs(scenario: dict[str, Any]) -> list[dict[str, Any]]:
    return [
        dict(bridge)
        for bridge in (scenario.get("stress") or {}).get("swap_bridges") or []
        if str(bridge.get("provider") or "").strip().lower() == "boltz"
    ]


def load_demo_scenario_metadata(path: Path = DEFAULT_SCENARIO) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def verify_demo_boltz_coverage(
    probe: dict[str, Any],
    scenario: dict[str, Any] | None = None,
) -> list[dict[str, Any]]:
    if scenario is None:
        scenario = load_demo_scenario_metadata()
    bridges = boltz_bridge_specs(scenario)
    if not bridges:
        raise BoltzProbeError("The full-accounting scenario has no provider=boltz swap bridge")

    available = {
        ("chain-swap", "BTC", "L-BTC"): probe.get("pairs", {}).get("bitcoin_to_liquid"),
        ("submarine-swap", "L-BTC", "BTC"): probe.get("pairs", {}).get("liquid_to_lightning"),
        ("reverse-submarine-swap", "BTC", "L-BTC"): probe.get("pairs", {}).get("lightning_to_liquid"),
    }
    covered = []
    for bridge in bridges:
        key = (
            str(bridge.get("boltz_flow") or "").strip(),
            str(bridge.get("boltz_from") or "").strip(),
            str(bridge.get("boltz_to") or "").strip(),
        )
        if key not in available or not available[key]:
            raise BoltzProbeError(f"Boltz demo bridge {bridge.get('id')} is not covered by live pair metadata: {key}")
        covered.append(
            {
                "id": bridge.get("id"),
                "flow": key[0],
                "from": key[1],
                "to": key[2],
                "pair_hash": available[key].get("hash"),
            }
        )
    return covered


REFUND_PUBKEY = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"


def execute_liquid_payment(*, amount_sats: int | None = None) -> dict[str, Any]:
    sats = int(amount_sats or os.environ.get("KASSIBER_BOLTZ_PAYMENT_SATS", "77777"))
    address = _elements_address()
    txid = _elements_cli("sendtoaddress", address, _sats_to_btc_text(sats), timeout=120).strip()
    _mine_liquid_blocks(1)
    return {
        "txid": txid,
        "address": address,
        "amount_sats": sats,
        "amount": _sats_to_btc_text(sats),
        "asset": "LBTC",
    }


def execute_liquid_submarine_swap(
    probe: dict[str, Any],
    *,
    api_url: str | None = None,
    invoice_sats: int | None = None,
    timeout_seconds: int | None = None,
) -> dict[str, Any]:
    base = _api_url(api_url)
    pair = probe.get("pairs", {}).get("liquid_to_lightning") or {}
    limits = pair.get("limits") if isinstance(pair.get("limits"), dict) else {}
    minimum = int(limits.get("minimal") or 0)
    requested_invoice_sats = int(
        invoice_sats
        or os.environ.get("KASSIBER_BOLTZ_SUBMARINE_INVOICE_SATS", "100000")
    )
    invoice_amount = max(requested_invoice_sats, minimum + 1000 if minimum else 0)

    invoice = _lncli_json("addinvoice", "--amt", str(invoice_amount))
    bolt11 = str(invoice.get("payment_request") or invoice.get("pay_req") or "").strip()
    if not bolt11:
        raise BoltzProbeError(f"lncli addinvoice returned no payment_request: {invoice}")
    payment_hash = _payment_hash_hex(invoice.get("r_hash") or invoice.get("r_hash_str"))
    swap = _post_json(
        base,
        "/v2/swap/submarine",
        {
            "invoice": bolt11,
            "from": "L-BTC",
            "to": "BTC",
            "refundPublicKey": os.environ.get("KASSIBER_BOLTZ_REFUND_PUBKEY", REFUND_PUBKEY),
        },
    )
    swap_id = str(swap.get("id") or "").strip()
    if not swap_id:
        raise BoltzProbeError(f"Boltz submarine swap response has no id: {swap}")
    expected_amount = int(swap.get("expectedAmount") or 0)
    if expected_amount <= 0:
        raise BoltzProbeError(f"Boltz submarine swap response has no expectedAmount: {swap}")
    lockup_address = _swap_address(swap)
    lockup_txid = _elements_cli(
        "sendtoaddress",
        lockup_address,
        _sats_to_btc_text(expected_amount),
        timeout=120,
    ).strip()
    _mine_liquid_blocks(int(os.environ.get("KASSIBER_BOLTZ_LIQUID_CONFIRMATIONS", "2")))
    status, history = _wait_for_swap_status(
        base,
        swap_id,
        timeout_seconds=int(
            timeout_seconds
            or os.environ.get("KASSIBER_BOLTZ_SWAP_TIMEOUT_SECONDS", "180")
        ),
    )
    return {
        "id": swap_id,
        "payment_hash": payment_hash,
        "invoice": bolt11,
        "invoice_sats": invoice_amount,
        "expected_amount_sats": expected_amount,
        "expected_amount": _sats_to_btc_text(expected_amount),
        "lockup_address": lockup_address,
        "lockup_txid": lockup_txid,
        "status": status.get("status"),
        "status_history": history,
    }


def _run_kassiber(data_root: Path, *args: str) -> dict[str, Any]:
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
        capture_output=True,
        text=True,
        check=False,
    )
    if not result.stdout.strip():
        raise BoltzProbeError(
            f"Kassiber CLI produced no stdout for {args!r}; stderr={result.stderr.strip()!r}"
        )
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise BoltzProbeError(
            f"Kassiber CLI returned non-JSON for {args!r}: {result.stdout!r}"
        ) from exc
    if result.returncode != 0:
        raise BoltzProbeError(
            f"Kassiber CLI failed for {args!r}: {json.dumps(payload, sort_keys=True)}"
        )
    return payload


def _require_kassiber_cli_ready() -> None:
    result = subprocess.run(
        [sys.executable, "-m", "kassiber", "--help"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise BoltzProbeError(
            "Kassiber CLI is not importable in this Python environment; "
            "install the project dependencies before running the live Boltz "
            f"execution lane.\nstderr: {result.stderr.strip()}"
        )


def _write_liquid_ledger(path: Path, *, payment: dict[str, Any], swap: dict[str, Any]) -> None:
    rows = [
        {
            "Type": "Withdrawal",
            "Date": "2026-07-02T10:00:00Z",
            "Sent Amount": payment["amount"],
            "Sent Asset": "LBTC",
            "Counterparty": "regtest-recipient",
            "Note": "Executed Liquid regtest payment",
            "Tx-ID": payment["txid"],
        },
        {
            "Type": "Withdrawal",
            "Date": "2026-07-02T10:02:00Z",
            "Sent Amount": swap["expected_amount"],
            "Sent Asset": "LBTC",
            "Counterparty": "Boltz regtest",
            "Note": "Executed Boltz Liquid submarine lockup",
            "Tx-ID": swap["lockup_txid"],
            "Payment Hash": swap["payment_hash"],
            "Payment Hash Source": "boltz-regtest",
        },
    ]
    fieldnames = [
        "Type",
        "Date",
        "Received Amount",
        "Received Asset",
        "Sent Amount",
        "Sent Asset",
        "Fee Amount",
        "Fee Asset",
        "Fiat Value",
        "Counterparty",
        "Note",
        "Tx-ID",
        "Payment Hash",
        "Payment Hash Source",
        "Swap Refund Funding Tx-ID",
    ]
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def _write_lightning_csv(path: Path, *, swap: dict[str, Any]) -> None:
    amount_msat = int(swap["invoice_sats"]) * 1000
    with path.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(
            handle,
            fieldnames=[
                "date",
                "id",
                "type",
                "amount_msat",
                "amount_fiat",
                "fee_credit_msat",
                "mining_fee_sat",
                "mining_fee_fiat",
                "service_fee_msat",
                "service_fee_fiat",
                "payment_hash",
                "tx_id",
                "destination",
                "description",
            ],
        )
        writer.writeheader()
        writer.writerow(
            {
                "date": "2026-07-02T10:04:00Z",
                "id": f"boltz-ln-{swap['id']}",
                "type": "lightning_received",
                "amount_msat": str(amount_msat),
                "amount_fiat": "0 USD",
                "fee_credit_msat": "0",
                "mining_fee_sat": "0",
                "mining_fee_fiat": "0 USD",
                "service_fee_msat": "0",
                "service_fee_fiat": "0 USD",
                "payment_hash": swap["payment_hash"],
                "destination": "boltz-regtest",
                "description": "Executed Boltz Liquid submarine settlement",
            }
        )


def _build_accounting_book(data_root: Path, *, payment: dict[str, Any], swap: dict[str, Any]) -> dict[str, Any]:
    import_dir = data_root / "imports"
    import_dir.mkdir(parents=True, exist_ok=True)
    liquid_csv = import_dir / "liquid.csv"
    lightning_csv = import_dir / "lightning.csv"
    _write_liquid_ledger(liquid_csv, payment=payment, swap=swap)
    _write_lightning_csv(lightning_csv, swap=swap)

    _run_kassiber(data_root, "init")
    _run_kassiber(data_root, "workspaces", "create", "Boltz")
    _run_kassiber(
        data_root,
        "profiles",
        "create",
        "--workspace",
        "Boltz",
        "--fiat-currency",
        "USD",
        "--tax-country",
        "at",
        "LiquidSwap",
    )
    _run_kassiber(
        data_root,
        "wallets",
        "create",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--label",
        "liquid-regtest",
        "--kind",
        "custom",
    )
    _run_kassiber(
        data_root,
        "wallets",
        "create",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--label",
        "lnd-regtest",
        "--kind",
        "lnd",
    )
    _run_kassiber(
        data_root,
        "wallets",
        "import-ledger",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--wallet",
        "liquid-regtest",
        "--file",
        str(liquid_csv),
    )
    _run_kassiber(
        data_root,
        "wallets",
        "import-phoenix",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--wallet",
        "lnd-regtest",
        "--file",
        str(lightning_csv),
    )

    suggested = _run_kassiber(
        data_root,
        "transfers",
        "suggest",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--confidence",
        "exact",
        "--method",
        "payment_hash",
        "--asset-pair",
        "LBTC-BTC",
    )
    candidates = suggested["data"]["candidates"]
    if len(candidates) != 1:
        raise BoltzProbeError(
            f"Expected one exact Boltz LBTC-BTC candidate, got {len(candidates)}: {candidates}"
        )
    candidate = candidates[0]
    if candidate["default_kind"] != "submarine-swap":
        raise BoltzProbeError(f"Expected submarine-swap candidate, got {candidate}")
    if candidate["in_wallet_kind"] != "lnd" or candidate["out_wallet_kind"] != "custom":
        raise BoltzProbeError(f"Expected Liquid on-chain -> Lightning route, got {candidate}")

    paired = _run_kassiber(
        data_root,
        "transfers",
        "bulk-pair",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
        "--confidence",
        "exact",
        "--method",
        "payment_hash",
        "--asset-pair",
        "LBTC-BTC",
    )
    if int(paired["data"]["summary"]["count"]) != 1:
        raise BoltzProbeError(f"Expected one exact Boltz pair, got {paired['data']}")

    listed = _run_kassiber(
        data_root,
        "transfers",
        "list",
        "--workspace",
        "Boltz",
        "--profile",
        "LiquidSwap",
    )
    pair = listed["data"][0]

    import sqlite3

    conn = sqlite3.connect(data_root / "kassiber.sqlite3")
    conn.row_factory = sqlite3.Row
    try:
        plain = conn.execute(
            """
            SELECT t.id, t.external_id, t.direction, t.asset, t.payment_hash
            FROM transactions t
            WHERE t.external_id = ?
            """,
            (payment["txid"],),
        ).fetchone()
        if plain is None:
            raise BoltzProbeError("Liquid payment did not import into the accounting book")
        plain_pair = conn.execute(
            """
            SELECT COUNT(*) AS count
            FROM transaction_pairs
            WHERE deleted_at IS NULL
              AND (out_transaction_id = ? OR in_transaction_id = ?)
            """,
            (plain["id"], plain["id"]),
        ).fetchone()["count"]
        swap_lockup = conn.execute(
            """
            SELECT payment_hash, payment_hash_source
            FROM transactions
            WHERE external_id = ?
            """,
            (swap["lockup_txid"],),
        ).fetchone()
    finally:
        conn.close()
    if plain is None or plain["direction"] != "outbound" or plain["asset"] != "LBTC":
        raise BoltzProbeError(f"Liquid payment did not import as outbound LBTC payment: {plain}")
    if int(plain_pair) != 0:
        raise BoltzProbeError("Plain Liquid payment was unexpectedly paired as a transfer")
    if not swap_lockup or swap_lockup["payment_hash"] != swap["payment_hash"]:
        raise BoltzProbeError(f"Liquid swap lockup lost payment_hash linkage: {swap_lockup}")

    return {
        "candidate": candidate,
        "pair": pair,
        "plain_payment": {
            "txid": plain["external_id"],
            "asset": plain["asset"],
            "direction": plain["direction"],
            "paired": False,
            "payment_hash": plain["payment_hash"],
        },
        "swap_lockup": {
            "txid": swap["lockup_txid"],
            "payment_hash": swap_lockup["payment_hash"],
            "payment_hash_source": swap_lockup["payment_hash_source"],
        },
        "imports": {
            "liquid_rows": 2,
            "lightning_rows": 1,
        },
    }


def run_boltz_liquid_scenario(api_url: str | None = None) -> dict[str, Any]:
    probe = probe_boltz_liquid(api_url)
    coverage = verify_demo_boltz_coverage(probe)
    _require_kassiber_cli_ready()
    payment = execute_liquid_payment()
    swap = execute_liquid_submarine_swap(probe, api_url=api_url)
    keep_root = os.environ.get("KASSIBER_BOLTZ_ACCOUNTING_ROOT")
    if keep_root:
        data_root = Path(keep_root)
        data_root.mkdir(parents=True, exist_ok=True)
        accounting = _build_accounting_book(data_root, payment=payment, swap=swap)
        data_root_value = str(data_root)
    else:
        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-liquid-") as tmp:
            data_root = Path(tmp) / "data"
            accounting = _build_accounting_book(data_root, payment=payment, swap=swap)
        data_root_value = None
    return {
        "probe": probe,
        "demo_coverage": coverage,
        "executed": {
            "liquid_payment": payment,
            "liquid_submarine_swap": swap,
        },
        "accounting": accounting,
        "data_root": data_root_value,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Exercise a local Boltz regtest API for Liquid swap coverage.")
    parser.add_argument("--api-url", default=None, help=f"Boltz API URL, default {DEFAULT_API_URL}")
    parser.add_argument(
        "--execute",
        action="store_true",
        help="Execute a Liquid payment and Boltz L-BTC -> BTC Lightning submarine swap, then assert Kassiber pairing.",
    )
    parser.add_argument("--json", action="store_true", help="Print the full JSON summary.")
    args = parser.parse_args(argv)

    if args.execute:
        summary = run_boltz_liquid_scenario(args.api_url)
        probe = summary["probe"]
        covered = summary["demo_coverage"]
    else:
        probe = probe_boltz_liquid(args.api_url)
        covered = verify_demo_boltz_coverage(probe)
        summary = {"probe": probe, "demo_coverage": covered}
    if args.json:
        print(json.dumps(summary, indent=2, sort_keys=True))
    elif args.execute:
        accounting = summary["accounting"]
        swap = summary["executed"]["liquid_submarine_swap"]
        payment = summary["executed"]["liquid_payment"]
        print(
            "Boltz Liquid regtest executed: "
            f"version={probe.get('version')} "
            f"payment_txid={payment['txid']} "
            f"swap_id={swap['id']} "
            f"swap_status={swap.get('status')} "
            f"pair_kind={accounting['pair'].get('kind')} "
            f"demo_bridges={len(covered)}"
        )
    else:
        print(
            "Boltz Liquid regtest ready: "
            f"version={probe.get('version')} "
            f"btc_height={probe['heights'].get('BTC')} "
            f"lbtc_height={probe['heights'].get('L-BTC')} "
            f"demo_bridges={len(covered)}"
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except BoltzProbeError as exc:
        print(str(exc), file=sys.stderr)
        raise SystemExit(1)
