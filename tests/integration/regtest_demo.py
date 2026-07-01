from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import tempfile
import uuid
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any
from urllib import error, request

from kassiber.core.sync_backends import sanitize_wallet_segment


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_SCENARIO = ROOT / "dev" / "regtest" / "scenarios" / "full_accounting.json"
SAT = Decimal("0.00000001")
TRANSACTION_LIST_LIMIT = "1500"
SECONDS_PER_DAY = 86_400


@dataclass
class DemoWallet:
    key: str
    label: str
    account: str
    core_wallet: str
    address: str
    kassiber_id: str | None = None
    watchonly_wallet: str | None = None


def load_scenario(path: Path = DEFAULT_SCENARIO) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        scenario = json.load(handle)
    validate_scenario(scenario)
    return scenario


def validate_scenario(scenario: dict[str, Any]) -> None:
    required = {"schema_version", "id", "base_time", "workspace", "profile", "wallets", "operations", "expected"}
    missing = sorted(required.difference(scenario))
    if missing:
        raise ValueError(f"Scenario manifest is missing: {', '.join(missing)}")
    if scenario["schema_version"] != 1:
        raise ValueError(f"Unsupported scenario schema_version: {scenario['schema_version']}")
    wallet_keys = [wallet["key"] for wallet in scenario["wallets"]]
    if len(wallet_keys) != len(set(wallet_keys)):
        raise ValueError("Scenario wallet keys must be unique")
    wallet_key_set = set(wallet_keys)
    for wallet in scenario["wallets"]:
        for field in ("key", "label", "account", "initial_btc"):
            if not wallet.get(field):
                raise ValueError(f"Scenario wallet {wallet.get('key')!r} is missing {field}")
        _btc(wallet["initial_btc"])
    for operation in scenario["operations"]:
        op_id = operation.get("id") or "<unnamed>"
        kind = operation.get("kind")
        if not kind:
            raise ValueError(f"Scenario operation {op_id} is missing kind")
        for field in ("amount_btc", "fee_btc", "payment_btc", "equal_output_btc"):
            if field in operation:
                _btc(operation[field])
        refs = []
        for ref_field in ("from", "to", "payer", "merchant", "tracked_output_wallet"):
            value = operation.get(ref_field)
            if value and value != "external":
                refs.append((ref_field, value))
        refs.extend(("signers", signer) for signer in operation.get("signers", []))
        for ref_field, value in refs:
            if value not in wallet_key_set:
                raise ValueError(f"Scenario operation {op_id} references unknown {ref_field}: {value}")
    stress = scenario.get("stress") or {}
    if stress.get("enabled"):
        cycles = int(stress.get("cycles") or 0)
        days_between_cycles = int(stress.get("days_between_cycles") or 0)
        if cycles <= 0:
            raise ValueError("Scenario stress.cycles must be positive")
        if days_between_cycles <= 0:
            raise ValueError("Scenario stress.days_between_cycles must be positive")
        for field in ("receipt_btc", "payment_btc"):
            entries = stress.get(field)
            if not isinstance(entries, dict) or not entries:
                raise ValueError(f"Scenario stress.{field} must be a non-empty object")
            for key, value in entries.items():
                if key not in wallet_key_set:
                    raise ValueError(f"Scenario stress.{field} references unknown wallet: {key}")
                _btc(value)
        if not stress.get("fee_btc"):
            raise ValueError("Scenario stress.fee_btc must be set")
        _btc(stress["fee_btc"])


def _btc(value: Any) -> Decimal:
    amount = Decimal(str(value)).quantize(SAT)
    if amount <= 0:
        raise ValueError(f"BTC amount must be positive: {value}")
    return amount


def _rpc_amount(value: Decimal) -> float:
    return float(value.quantize(SAT))


def _json_ready(value: Any) -> Any:
    if isinstance(value, Decimal):
        return _rpc_amount(value)
    if isinstance(value, dict):
        return {key: _json_ready(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_json_ready(item) for item in value]
    return value


def rpc(url: str, username: str, password: str, method: str, params=None, wallet: str | None = None):
    endpoint = url.rstrip("/")
    if wallet:
        endpoint = f"{endpoint}/wallet/{wallet}"
    payload = json.dumps(
        {
            "jsonrpc": "1.0",
            "id": f"kassiber-regtest-demo-{method}",
            "method": method,
            "params": [] if params is None else _json_ready(params),
        }
    ).encode("utf-8")
    req = request.Request(endpoint, data=payload, headers={"Content-Type": "application/json"})
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    req.add_header("Authorization", f"Basic {token}")
    try:
        with request.urlopen(req, timeout=60) as response:
            decoded = json.loads(response.read().decode("utf-8"))
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        try:
            decoded = json.loads(body)
        except json.JSONDecodeError as decode_error:
            raise RuntimeError(f"RPC {method} failed over HTTP {exc.code}: {body}") from decode_error
    if decoded.get("error"):
        raise RuntimeError(f"RPC {method} failed: {decoded['error']}")
    return decoded.get("result")


def run_cli(data_root: Path, *args: str, pass_fds: tuple[int, ...] = ()) -> dict[str, Any]:
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
        pass_fds=pass_fds,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"kassiber {' '.join(args)} failed\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return json.loads(result.stdout)


def _parse_iso_to_ts(value: str) -> int:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    return int(parsed.astimezone(timezone.utc).timestamp())


def _ensure_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    loaded = set(rpc(url, username, password, "listwallets") or [])
    if wallet_name in loaded:
        return
    try:
        rpc(url, username, password, "loadwallet", [wallet_name, True])
        return
    except RuntimeError:
        pass
    rpc(url, username, password, "createwallet", [wallet_name, False, False, "", False, True, True])


def _unload_wallet(url: str, username: str, password: str, wallet_name: str) -> None:
    try:
        rpc(url, username, password, "unloadwallet", [wallet_name])
    except RuntimeError:
        pass


def _advance_time(url: str, username: str, password: str, current_ts: int, *, seconds: int = 600) -> int:
    next_ts = current_ts + seconds
    rpc(url, username, password, "setmocktime", [next_ts])
    return next_ts


def _mine(
    url: str,
    username: str,
    password: str,
    _wallet: str,
    address: str,
    current_ts: int,
    *,
    blocks: int = 1,
) -> int:
    next_ts = _advance_time(url, username, password, current_ts)
    rpc(url, username, password, "generatetoaddress", [blocks, address])
    return next_ts


def _mine_at(
    url: str,
    username: str,
    password: str,
    address: str,
    current_ts: int,
    target_ts: int,
) -> int:
    next_ts = max(target_ts, current_ts + 600)
    rpc(url, username, password, "setmocktime", [next_ts])
    rpc(url, username, password, "generatetoaddress", [1, address])
    return next_ts


def _wallet_utxos(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    *,
    min_confirmations: int = 1,
) -> list[dict[str, Any]]:
    utxos = rpc(
        url,
        username,
        password,
        "listunspent",
        [min_confirmations, 9999999, [wallet.address], True],
        wallet=wallet.core_wallet,
    )
    return sorted(
        (utxo for utxo in utxos or [] if utxo.get("spendable", True)),
        key=lambda item: Decimal(str(item["amount"])),
        reverse=True,
    )


def _select_one_utxo(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    needed: Decimal,
) -> dict[str, Any]:
    for utxo in _wallet_utxos(url, username, password, wallet):
        if Decimal(str(utxo["amount"])) >= needed:
            return utxo
    raise RuntimeError(f"Wallet {wallet.key} has no confirmed watched UTXO >= {needed} BTC")


def _send_raw_transaction(
    url: str,
    username: str,
    password: str,
    inputs: list[dict[str, Any]],
    outputs: dict[str, Decimal],
    signers: list[str],
) -> str:
    raw = rpc(url, username, password, "createrawtransaction", [inputs, outputs])
    if len(signers) == 1:
        signed = rpc(url, username, password, "signrawtransactionwithwallet", [raw], wallet=signers[0])
        if not signed.get("complete"):
            raise RuntimeError(f"Core did not complete signing with {signers[0]}: {signed}")
        return rpc(url, username, password, "sendrawtransaction", [signed["hex"]])
    psbt = rpc(url, username, password, "converttopsbt", [raw])
    signed_psbts = []
    for signer in signers:
        processed = rpc(url, username, password, "walletprocesspsbt", [psbt], wallet=signer)
        signed_psbts.append(processed["psbt"])
    combined = rpc(url, username, password, "combinepsbt", [signed_psbts])
    finalized = rpc(url, username, password, "finalizepsbt", [combined])
    if not finalized.get("complete"):
        raise RuntimeError(f"Core did not complete PSBT finalization: {finalized}")
    return rpc(url, username, password, "sendrawtransaction", [finalized["hex"]])


def _send_from_wallet(
    url: str,
    username: str,
    password: str,
    wallet: DemoWallet,
    outputs: dict[str, Decimal],
    fee: Decimal,
) -> str:
    needed = sum(outputs.values(), Decimal("0")) + fee
    utxo = _select_one_utxo(url, username, password, wallet, needed)
    input_amount = Decimal(str(utxo["amount"])).quantize(SAT)
    change = (input_amount - needed).quantize(SAT)
    if change < 0:
        raise RuntimeError(f"Selected UTXO is too small for {wallet.key}")
    final_outputs = dict(outputs)
    if change > 0:
        final_outputs[wallet.address] = final_outputs.get(wallet.address, Decimal("0")) + change
    return _send_raw_transaction(
        url,
        username,
        password,
        [{"txid": utxo["txid"], "vout": utxo["vout"]}],
        final_outputs,
        [wallet.core_wallet],
    )


def _send_coinjoin_shape(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
    external_address: str,
) -> str:
    signer_keys = list(operation["signers"])
    if len(signer_keys) != 2:
        raise RuntimeError("coinjoin_shape currently expects exactly two signers")
    fee = _btc(operation["fee_btc"])
    equal_output = _btc(operation["equal_output_btc"])
    signer_a = wallets[signer_keys[0]]
    signer_b = wallets[signer_keys[1]]
    utxo_a = _select_one_utxo(url, username, password, signer_a, equal_output + fee)
    utxo_b = _select_one_utxo(url, username, password, signer_b, equal_output + fee)
    amount_a = Decimal(str(utxo_a["amount"])).quantize(SAT)
    amount_b = Decimal(str(utxo_b["amount"])).quantize(SAT)
    half_fee = (fee / 2).quantize(SAT)
    change_a = (amount_a - equal_output - half_fee).quantize(SAT)
    change_b = (amount_b - equal_output - (fee - half_fee)).quantize(SAT)
    if change_a <= 0 or change_b <= 0:
        raise RuntimeError("coinjoin_shape selected UTXOs leave no change")
    tracked_output_wallet = wallets[operation["tracked_output_wallet"]]
    outputs = {
        tracked_output_wallet.address: equal_output,
        external_address: equal_output,
        signer_a.address: change_a,
        signer_b.address: change_b,
    }
    return _send_raw_transaction(
        url,
        username,
        password,
        [
            {"txid": utxo_a["txid"], "vout": utxo_a["vout"]},
            {"txid": utxo_b["txid"], "vout": utxo_b["vout"]},
        ],
        outputs,
        [signer_a.core_wallet, signer_b.core_wallet],
    )


def _send_payjoin_shape(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    operation: dict[str, Any],
) -> str:
    payer = wallets[operation["payer"]]
    merchant = wallets[operation["merchant"]]
    payment = _btc(operation["payment_btc"])
    fee = _btc(operation["fee_btc"])
    payer_utxo = _select_one_utxo(url, username, password, payer, payment + fee)
    merchant_utxo = _select_one_utxo(url, username, password, merchant, SAT)
    payer_amount = Decimal(str(payer_utxo["amount"])).quantize(SAT)
    merchant_amount = Decimal(str(merchant_utxo["amount"])).quantize(SAT)
    payer_change = (payer_amount - payment - fee).quantize(SAT)
    merchant_output = (merchant_amount + payment).quantize(SAT)
    if payer_change <= 0:
        raise RuntimeError("payjoin_shape selected payer UTXO leaves no change")
    outputs = {
        merchant.address: merchant_output,
        payer.address: payer_change,
    }
    return _send_raw_transaction(
        url,
        username,
        password,
        [
            {"txid": payer_utxo["txid"], "vout": payer_utxo["vout"]},
            {"txid": merchant_utxo["txid"], "vout": merchant_utxo["vout"]},
        ],
        outputs,
        [payer.core_wallet, merchant.core_wallet],
    )


def _generate_stress_history(
    url: str,
    username: str,
    password: str,
    wallets: dict[str, DemoWallet],
    scenario: dict[str, Any],
    *,
    faucet_wallet: str,
    mining_address: str,
    external_address: str,
    current_ts: int,
    txids: dict[str, str],
) -> tuple[int, dict[str, Any]]:
    stress = scenario.get("stress") or {}
    if not stress.get("enabled"):
        return current_ts, {"cycles": 0, "rows_expected": 0, "span_days": 0}

    cycles = int(stress["cycles"])
    days_between_cycles = int(stress["days_between_cycles"])
    receipt_plan = {
        key: _btc(value)
        for key, value in sorted(stress["receipt_btc"].items())
    }
    payment_plan = [
        (key, _btc(value))
        for key, value in sorted(stress["payment_btc"].items())
    ]
    fee = _btc(stress["fee_btc"])
    first_target_ts = current_ts + (2 * SECONDS_PER_DAY)

    for cycle in range(cycles):
        cycle_number = cycle + 1
        cycle_ts = first_target_ts + (cycle * days_between_cycles * SECONDS_PER_DAY)
        receipt_outputs = {
            wallets[key].address: amount
            for key, amount in receipt_plan.items()
        }
        txids[f"stress_receipt_{cycle_number:03d}"] = rpc(
            url,
            username,
            password,
            "sendmany",
            ["", receipt_outputs],
            wallet=faucet_wallet,
        )
        current_ts = _mine_at(
            url,
            username,
            password,
            mining_address,
            current_ts,
            cycle_ts,
        )

        payer_key, amount = payment_plan[cycle % len(payment_plan)]
        txids[f"stress_payment_{cycle_number:03d}"] = _send_from_wallet(
            url,
            username,
            password,
            wallets[payer_key],
            {external_address: amount},
            fee,
        )
        current_ts = _mine_at(
            url,
            username,
            password,
            mining_address,
            current_ts,
            cycle_ts + (6 * 60 * 60),
        )

    return current_ts, {
        "cycles": cycles,
        "receipt_wallets": len(receipt_plan),
        "payment_wallets": len(payment_plan),
        "rows_expected": cycles * (len(receipt_plan) + 1),
        "span_days": (cycles - 1) * days_between_cycles,
    }


def _scope(scenario: dict[str, Any]) -> tuple[str, str, str, str]:
    return (
        "--workspace",
        scenario["workspace"],
        "--profile",
        scenario["profile"]["label"],
    )


def _create_kassiber_book(
    data_root: Path,
    scenario: dict[str, Any],
    wallets: dict[str, DemoWallet],
    *,
    url: str,
    username: str,
    password: str,
    wallet_prefix: str,
    birthday: str,
) -> None:
    run_cli(data_root, "init")
    run_cli(data_root, "workspaces", "create", scenario["workspace"])
    profile = scenario["profile"]
    run_cli(
        data_root,
        "profiles",
        "create",
        profile["label"],
        "--workspace",
        scenario["workspace"],
        "--fiat-currency",
        profile["fiat_currency"],
        "--tax-country",
        profile["tax_country"],
        "--gains-algorithm",
        profile["gains_algorithm"],
    )
    with tempfile.TemporaryFile("w+") as username_fd, tempfile.TemporaryFile("w+") as password_fd:
        username_fd.write(username)
        username_fd.flush()
        username_fd.seek(0)
        password_fd.write(password)
        password_fd.flush()
        password_fd.seek(0)
        run_cli(
            data_root,
            "backends",
            "create",
            scenario["backend"]["name"],
            "--kind",
            "bitcoinrpc",
            "--url",
            url,
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--username-fd",
            str(username_fd.fileno()),
            "--password-fd",
            str(password_fd.fileno()),
            "--wallet-prefix",
            wallet_prefix,
            "--timeout",
            "60",
            pass_fds=(username_fd.fileno(), password_fd.fileno()),
    )
    scope = _scope(scenario)
    existing_accounts = {
        account["code"]
        for account in run_cli(data_root, "accounts", "list", *scope)["data"]
    }
    for account in scenario["accounts"]:
        if account["code"] in existing_accounts:
            continue
        run_cli(
            data_root,
            "accounts",
            "create",
            *scope,
            "--code",
            account["code"],
            "--label",
            account["label"],
            "--type",
            account["type"],
            "--asset",
            account["asset"],
        )
    for wallet_spec in scenario["wallets"]:
        wallet = wallets[wallet_spec["key"]]
        created = run_cli(
            data_root,
            "wallets",
            "create",
            *scope,
            "--label",
            wallet.label,
            "--kind",
            "address",
            "--account",
            wallet.account,
            "--backend",
            scenario["backend"]["name"],
            "--chain",
            "bitcoin",
            "--network",
            "regtest",
            "--address",
            wallet.address,
            "--birthday",
            birthday,
        )["data"]
        wallet.kassiber_id = created["id"]
        wallet.watchonly_wallet = (
            f"{sanitize_wallet_segment(wallet_prefix)}-{sanitize_wallet_segment(created['id'])}"
        )


def _seed_rates_and_process(data_root: Path, scenario: dict[str, Any]) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    scope = _scope(scenario)
    transactions = run_cli(
        data_root,
        "transactions",
        "list",
        *scope,
        "--order",
        "asc",
        "--limit",
        TRANSACTION_LIST_LIMIT,
    )["data"]
    unique_times = sorted({row["occurred_at"] for row in transactions})
    pricing = scenario["pricing"]
    base_rate = Decimal(pricing["base_rate"])
    step_rate = Decimal(pricing["step_rate"])
    for index, occurred_at in enumerate(unique_times):
        rate = base_rate + (step_rate * index)
        run_cli(
            data_root,
            "rates",
            "set",
            pricing["pair"],
            occurred_at,
            str(rate),
            "--source",
            "regtest-demo",
            "--granularity",
            "exact",
            "--method",
            scenario["id"],
        )
    journal = run_cli(data_root, "journals", "process", *scope)["data"]
    return journal, transactions


def _pair_transfers(data_root: Path, scenario: dict[str, Any], txids: dict[str, str]) -> list[dict[str, Any]]:
    scope = _scope(scenario)
    paired = []
    for operation in scenario["operations"]:
        if operation["kind"] != "self_transfer":
            continue
        paired.append(
            run_cli(
                data_root,
                "transfers",
                "pair",
                *scope,
                "--tx-out",
                txids[operation["id"]],
                "--tx-in",
                txids[operation["id"]],
                "--kind",
                "manual",
                "--policy",
                "carrying-value",
                "--note",
                operation["note"],
            )["data"]
        )
    return paired


def _mark_loans(data_root: Path, scenario: dict[str, Any], txids: dict[str, str]) -> dict[str, Any]:
    scope = _scope(scenario)
    mark_as = {
        "loan_collateral_lock": "collateral",
        "loan_collateral_release": "returned",
        "loan_principal_received": "principal-received",
        "loan_principal_repaid": "principal-repaid",
    }
    marked_txids = []
    marks = []
    for operation in scenario["operations"]:
        role = mark_as.get(operation["kind"])
        if role is None:
            continue
        txid = txids[operation["id"]]
        marked_txids.append(txid)
        marks.append(
            run_cli(
                data_root,
                "loans",
                "mark",
                *scope,
                "--txid",
                txid,
                "--as",
                role,
                "--loan-id",
                operation["loan_id"],
                "--note",
                operation["note"],
            )["data"]
        )
    if len(marked_txids) >= 2:
        link_args = []
        for txid in marked_txids:
            link_args.extend(["--txid", txid])
        run_cli(
            data_root,
            "loans",
            "link",
            *scope,
            *link_args,
            "--loan-id",
            "regtest-loan-1",
        )
    listing = run_cli(data_root, "loans", "list", *scope)["data"]
    return {"marks": marks, "listing": listing}


def _exclude_collaborative_shapes(
    data_root: Path,
    scenario: dict[str, Any],
    txids: dict[str, str],
    transactions: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    scope = _scope(scenario)
    collaborative_txids = {
        txids[operation["id"]]
        for operation in scenario["operations"]
        if operation["kind"] in {"coinjoin_shape", "payjoin_shape"}
    }
    excluded = []
    for row in transactions:
        if row["external_id"] not in collaborative_txids:
            continue
        updated = run_cli(
            data_root,
            "metadata",
            "records",
            "excluded",
            "set",
            *scope,
            "--transaction",
            row["id"],
            "--reason",
            "Regtest collaborative transaction shape reviewed outside this tax-report scenario.",
        )["data"]
        excluded.append(
            {
                "transaction_id": row["id"],
                "external_id": row["external_id"],
                "wallet": row["wallet"],
                "direction": row["direction"],
                "excluded": bool(updated["excluded"]),
            }
        )
    return excluded


def _export_reports(data_root: Path, export_dir: Path, scenario: dict[str, Any]) -> dict[str, Any]:
    scope = _scope(scenario)
    export_dir.mkdir(parents=True, exist_ok=True)
    exports = {
        "report_pdf": export_dir / "full-report.pdf",
        "report_csv": export_dir / "full-report.csv",
        "report_xlsx": export_dir / "full-report.xlsx",
        "transactions_csv": export_dir / "transactions.csv",
        "transactions_xlsx": export_dir / "transactions.xlsx",
    }
    results = {
        "report_pdf": run_cli(data_root, "reports", "export-pdf", *scope, "--file", str(exports["report_pdf"]))[
            "data"
        ],
        "report_csv": run_cli(data_root, "reports", "export-csv", *scope, "--file", str(exports["report_csv"]))[
            "data"
        ],
        "report_xlsx": run_cli(
            data_root, "reports", "export-xlsx", *scope, "--file", str(exports["report_xlsx"])
        )["data"],
        "transactions_csv": run_cli(
            data_root,
            "transactions",
            "export",
            *scope,
            "--export-format",
            "csv",
            "--file",
            str(exports["transactions_csv"]),
        )["data"],
        "transactions_xlsx": run_cli(
            data_root,
            "transactions",
            "export",
            *scope,
            "--export-format",
            "xlsx",
            "--file",
            str(exports["transactions_xlsx"]),
        )["data"],
    }
    missing = [name for name, path in exports.items() if not path.exists() or path.stat().st_size <= 0]
    if missing:
        raise RuntimeError(f"Missing or empty export files: {', '.join(missing)}")
    if not results["report_xlsx"].get("verified"):
        raise RuntimeError("XLSX export did not include the self-verification sheets")
    try:
        import openpyxl
    except ImportError:
        pass
    else:
        workbook = openpyxl.load_workbook(exports["report_xlsx"], read_only=True, data_only=False)
        try:
            required_sheets = {"Verify", "Control", "Acquisitions", "Disposals"}
            missing_sheets = sorted(required_sheets.difference(workbook.sheetnames))
            if missing_sheets:
                raise RuntimeError(f"XLSX export is missing verification sheets: {missing_sheets}")
        finally:
            workbook.close()
    return {name: {"path": str(exports[name]), **results[name]} for name in exports}


def _assert_expected(
    scenario: dict[str, Any],
    *,
    transactions: list[dict[str, Any]],
    transfers: dict[str, Any],
    loans: dict[str, Any],
    journal: dict[str, Any],
    quarantines: list[dict[str, Any]],
    collaborative_excluded: list[dict[str, Any]],
    summary: dict[str, Any],
    exports: dict[str, Any],
) -> None:
    expected = scenario["expected"]
    if len(transactions) < int(expected["min_transactions"]):
        raise RuntimeError(f"Expected at least {expected['min_transactions']} transactions, got {len(transactions)}")
    if len(transfers["pairs"]) < int(expected["min_transfer_pairs"]):
        raise RuntimeError(f"Expected at least {expected['min_transfer_pairs']} transfer pairs")
    if len(collaborative_excluded) != int(expected["collaborative_excluded"]):
        raise RuntimeError(
            f"Expected {expected['collaborative_excluded']} collaborative rows to be excluded, "
            f"got {len(collaborative_excluded)}"
        )
    loan_listing = loans["listing"]
    if len(loan_listing["marks"]) != int(expected["loan_marks"]):
        raise RuntimeError(f"Expected {expected['loan_marks']} loan marks, got {len(loan_listing['marks'])}")
    if len(loan_listing["open_locks"]) != int(expected["open_collateral_locks"]):
        raise RuntimeError(
            f"Expected {expected['open_collateral_locks']} open collateral locks, got {len(loan_listing['open_locks'])}"
        )
    if int(journal.get("quarantined") or 0) != 0:
        rendered = json.dumps(quarantines[:10], indent=2, sort_keys=True)
        raise RuntimeError(f"Expected zero journal quarantines, got {journal.get('quarantined')}: {rendered}")
    metrics = summary["metrics"]
    min_active = int(expected.get("min_active_transactions") or expected["min_transactions"])
    if int(metrics.get("active_transactions") or 0) < min_active:
        raise RuntimeError("Report summary did not include the expected active transactions")
    for filename in expected["export_files"]:
        if not any(Path(item["path"]).name == filename for item in exports.values()):
            raise RuntimeError(f"Expected export file was not produced: {filename}")


def run_demo(
    *,
    scenario_path: Path = DEFAULT_SCENARIO,
    data_root: Path | None = None,
    export_dir: Path | None = None,
    run_id: str | None = None,
    keep_core_wallets: bool = False,
) -> dict[str, Any]:
    scenario = load_scenario(scenario_path)
    url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
    username = os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber")
    password = os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber")
    run_id = run_id or os.environ.get("KASSIBER_REGTEST_DEMO_RUN_ID") or uuid.uuid4().hex[:12]
    base_dir = Path(os.environ.get("KASSIBER_REGTEST_DEMO_ROOT", f"/tmp/kassiber-regtest-demo-{run_id}"))
    data_root = Path(data_root or os.environ.get("KASSIBER_REGTEST_DEMO_DATA_ROOT", base_dir / "data"))
    export_dir = Path(export_dir or os.environ.get("KASSIBER_REGTEST_DEMO_EXPORT_DIR", base_dir / "exports"))
    if data_root.exists() or export_dir.exists():
        raise RuntimeError(
            f"Demo output path already exists ({data_root} / {export_dir}). "
            "Set KASSIBER_REGTEST_DEMO_ROOT to a new directory or remove the old run."
        )

    chain = rpc(url, username, password, "getblockchaininfo")
    if chain.get("chain") != "regtest":
        raise RuntimeError(f"Refusing to run against non-regtest Core node: {chain.get('chain')}")

    manifest_ts = _parse_iso_to_ts(scenario["base_time"])
    chain_median_ts = int(chain.get("mediantime") or manifest_ts)
    current_ts = max(manifest_ts, chain_median_ts + 7200)
    rpc(url, username, password, "setmocktime", [current_ts])
    birthday_ts = current_ts - SECONDS_PER_DAY

    created_core_wallets: list[str] = []
    backend_wallet_prefix = f"{scenario['backend']['wallet_prefix']}-{run_id}"
    faucet_wallet = f"kassiber-demo-{run_id}-external"
    wallets: dict[str, DemoWallet] = {}
    txids: dict[str, str] = {}
    try:
        _ensure_wallet(url, username, password, faucet_wallet)
        created_core_wallets.append(faucet_wallet)
        mining_address = rpc(url, username, password, "getnewaddress", ["mining", "bech32"], wallet=faucet_wallet)
        external_address = rpc(url, username, password, "getnewaddress", ["external", "bech32"], wallet=faucet_wallet)
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts, blocks=101)

        for wallet_spec in scenario["wallets"]:
            core_wallet = f"kassiber-demo-{run_id}-{sanitize_wallet_segment(wallet_spec['key'])}"
            _ensure_wallet(url, username, password, core_wallet)
            created_core_wallets.append(core_wallet)
            address = rpc(url, username, password, "getnewaddress", [wallet_spec["label"], "bech32"], wallet=core_wallet)
            wallets[wallet_spec["key"]] = DemoWallet(
                key=wallet_spec["key"],
                label=wallet_spec["label"],
                account=wallet_spec["account"],
                core_wallet=core_wallet,
                address=address,
            )

        funding_outputs = {
            wallets[wallet_spec["key"]].address: _btc(wallet_spec["initial_btc"])
            for wallet_spec in scenario["wallets"]
        }
        txids["initial_funding"] = rpc(
            url,
            username,
            password,
            "sendmany",
            ["", funding_outputs],
            wallet=faucet_wallet,
        )
        current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)

        for operation in scenario["operations"]:
            kind = operation["kind"]
            if kind in {"payment", "self_transfer", "loan_collateral_lock", "loan_principal_repaid"}:
                sender = wallets[operation["from"]]
                to_address = external_address if operation["to"] == "external" else wallets[operation["to"]].address
                txids[operation["id"]] = _send_from_wallet(
                    url,
                    username,
                    password,
                    sender,
                    {to_address: _btc(operation["amount_btc"])},
                    _btc(operation["fee_btc"]),
                )
            elif kind == "coinjoin_shape":
                coinjoin_external = rpc(
                    url, username, password, "getnewaddress", ["coinjoin equal output", "bech32"], wallet=faucet_wallet
                )
                txids[operation["id"]] = _send_coinjoin_shape(
                    url, username, password, wallets, operation, coinjoin_external
                )
            elif kind == "payjoin_shape":
                txids[operation["id"]] = _send_payjoin_shape(url, username, password, wallets, operation)
            elif kind in {"loan_collateral_release", "loan_principal_received"}:
                receiver = wallets[operation["to"]]
                txids[operation["id"]] = rpc(
                    url,
                    username,
                    password,
                    "sendtoaddress",
                    [receiver.address, _btc(operation["amount_btc"])],
                    wallet=faucet_wallet,
                )
            else:
                raise RuntimeError(f"Unsupported scenario operation kind: {kind}")
            current_ts = _mine(url, username, password, faucet_wallet, mining_address, current_ts)

        current_ts, stress_result = _generate_stress_history(
            url,
            username,
            password,
            wallets,
            scenario,
            faucet_wallet=faucet_wallet,
            mining_address=mining_address,
            external_address=external_address,
            current_ts=current_ts,
            txids=txids,
        )

        birthday = datetime.fromtimestamp(birthday_ts, tz=timezone.utc)
        birthday_iso = birthday.isoformat().replace("+00:00", "Z")
        _create_kassiber_book(
            data_root,
            scenario,
            wallets,
            url=url,
            username=username,
            password=password,
            wallet_prefix=backend_wallet_prefix,
            birthday=birthday_iso,
        )
        created_core_wallets.extend(
            wallet.watchonly_wallet for wallet in wallets.values() if wallet.watchonly_wallet
        )

        scope = _scope(scenario)
        sync = run_cli(data_root, "wallets", "sync", *scope, "--all")["data"]
        transactions = run_cli(
            data_root,
            "transactions",
            "list",
            *scope,
            "--limit",
            TRANSACTION_LIST_LIMIT,
            "--order",
            "asc",
        )["data"]
        collaborative_excluded = _exclude_collaborative_shapes(data_root, scenario, txids, transactions)
        pairs = _pair_transfers(data_root, scenario, txids)
        loan_result = _mark_loans(data_root, scenario, txids)
        journal, transactions = _seed_rates_and_process(data_root, scenario)
        transfer_listing = run_cli(data_root, "transfers", "list", *scope)["data"]
        quarantines = run_cli(data_root, "journals", "quarantined", *scope)["data"]
        summary = run_cli(data_root, "reports", "summary", *scope)["data"]
        exports = _export_reports(data_root, export_dir, scenario)
        _assert_expected(
            scenario,
            transactions=transactions,
            transfers={"pairs": transfer_listing},
            loans=loan_result,
            journal=journal,
            quarantines=quarantines,
            collaborative_excluded=collaborative_excluded,
            summary=summary,
            exports=exports,
        )

        by_direction = Counter(row["direction"] for row in transactions)
        by_wallet = Counter(row["wallet"] for row in transactions)
        result = {
            "kind": "regtest.demo.full_accounting",
            "schema_version": 1,
            "data": {
                "scenario": scenario["id"],
                "run_id": run_id,
                "data_root": str(data_root),
                "export_dir": str(export_dir),
                "core_url": url,
                "base_time": datetime.fromtimestamp(current_ts, tz=timezone.utc).isoformat().replace("+00:00", "Z"),
                "wallets": {
                    key: {
                        "label": wallet.label,
                        "address": wallet.address,
                        "kassiber_id": wallet.kassiber_id,
                    }
                    for key, wallet in wallets.items()
                },
                "operations": [{"id": key, "txid": value} for key, value in sorted(txids.items())],
                "sync": sync,
                "transactions": {
                    "count": len(transactions),
                    "by_direction": dict(sorted(by_direction.items())),
                    "by_wallet": dict(sorted(by_wallet.items())),
                },
                "stress": stress_result,
                "transfers": {
                    "paired": pairs,
                    "count": len(transfer_listing),
                },
                "collaborative_excluded": collaborative_excluded,
                "loans": {
                    "marks": len(loan_result["listing"]["marks"]),
                    "open_locks": len(loan_result["listing"]["open_locks"]),
                    "loan_ids": sorted(
                        {mark["loan_id"] for mark in loan_result["listing"]["marks"] if mark.get("loan_id")}
                    ),
                },
                "journal": journal,
                "summary_metrics": summary["metrics"],
                "exports": exports,
            },
        }
        return result
    except Exception:
        if os.environ.get("KASSIBER_REGTEST_DEMO_KEEP_FAILED"):
            print(
                f"Keeping failed regtest demo data_root={data_root} export_dir={export_dir}",
                file=sys.stderr,
            )
        else:
            if data_root.exists():
                shutil.rmtree(data_root, ignore_errors=True)
            if export_dir.exists():
                shutil.rmtree(export_dir, ignore_errors=True)
        raise
    finally:
        try:
            rpc(url, username, password, "setmocktime", [0])
        except RuntimeError:
            pass
        if not keep_core_wallets:
            for wallet_name in reversed(created_core_wallets):
                if wallet_name:
                    _unload_wallet(url, username, password, wallet_name)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the full Kassiber Bitcoin Core regtest demo scenario.")
    parser.add_argument("--scenario", type=Path, default=DEFAULT_SCENARIO)
    parser.add_argument("--data-root", type=Path)
    parser.add_argument("--export-dir", type=Path)
    parser.add_argument("--run-id")
    parser.add_argument("--json-output", type=Path)
    parser.add_argument("--keep-core-wallets", action="store_true")
    args = parser.parse_args(argv)

    result = run_demo(
        scenario_path=args.scenario,
        data_root=args.data_root,
        export_dir=args.export_dir,
        run_id=args.run_id,
        keep_core_wallets=args.keep_core_wallets,
    )
    rendered = json.dumps(result, indent=2, sort_keys=True)
    if args.json_output:
        args.json_output.parent.mkdir(parents=True, exist_ok=True)
        args.json_output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
