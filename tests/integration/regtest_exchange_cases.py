"""Real Core movements plus synthetic Strike exports through production imports.

The fixture provider is not a live exchange connection. Explicitly reviewed
exports can explain custody, but never acquire native observer authority.
"""
from __future__ import annotations

import csv
from datetime import datetime, timezone
from decimal import Decimal
import json
from pathlib import Path
import uuid
from typing import Any, Callable, Mapping

from kassiber.db import open_db

D = Decimal
HEADERS = ["Reference", "Date & Time (UTC)", "Transaction Type", "Amount EUR", "Fee EUR",
           "Amount BTC", "Fee BTC", "BTC Price", "Cost Basis (EUR)", "Destination",
           "Description", "Transaction Hash", "Note"]
# Independent arithmetic: buy 0.01 for 1001; withdraw 0.006 + 0.0001 charge;
# return 0.002 + 0.00001 network fee; sell 0.001 for 150 - 1 EUR.
EXPECTED = {
    "platform_quantity_msat": 490_000_000,
    "self_custody_quantity_msat": 399_000_000,
    "combined_quantity_msat": 889_000_000,
    "remaining_basis_eur": "889.889",
    "sale_proceeds_eur": "149",
    "sale_basis_eur": "100.1",
}


def _iso(timestamp: int) -> str:
    return datetime.fromtimestamp(timestamp, timezone.utc).isoformat().replace("+00:00", "Z")


def write_exports(
    artifact_dir: Path,
    *,
    txids: Mapping[str, str],
    times: Mapping[str, str],
) -> dict[str, Path]:
    artifact_dir.mkdir(parents=True, exist_ok=True)
    rows = [
        {
            "Reference": "demo-buy", "Date & Time (UTC)": times["buy_time"],
            "Transaction Type": "Buy", "Amount EUR": "-1000",
            "Fee EUR": "1", "Amount BTC": "0.01000000",
        },
        {
            "Reference": "demo-withdraw", "Date & Time (UTC)": times["withdrawal_time"],
            "Transaction Type": "Send", "Amount BTC": "-0.00600000",
            "Fee BTC": "0.00010000", "BTC Price": "100100",
            "Transaction Hash": txids["withdrawal"],
        },
        {
            "Reference": "demo-deposit", "Date & Time (UTC)": times["deposit_time"],
            "Transaction Type": "Receive", "Amount BTC": "0.00200000",
            "BTC Price": "100100", "Transaction Hash": txids["deposit"],
        },
        {
            "Reference": "demo-sell", "Date & Time (UTC)": times["sell_time"],
            "Transaction Type": "Sell", "Amount EUR": "150",
            "Fee EUR": "1", "Amount BTC": "-0.00100000",
        },
    ]
    files = {}
    for name, selected in (("incomplete", rows[1: ]), ("complete", rows)):
        path = artifact_dir / f"strike-{name}.csv"
        with path.open("w", newline="", encoding="utf-8") as stream:
            writer = csv.DictWriter(stream, fieldnames=HEADERS)
            writer.writeheader()
            writer.writerows(selected)
        files[name] = path
    return files


def _read_state(
    data_root: Path, profile_id: str, native_wallet_id: str,
) -> dict[str, Any]:
    conn = open_db(str(data_root))
    try:
        holdings = conn.execute(
            "SELECT wallet_id, quantity, cost_basis FROM journal_wallet_holdings WHERE profile_id=?",
            (profile_id,),
        ).fetchall()
        platform = sum(int(row["quantity"]) for row in holdings if row["wallet_id"] != native_wallet_id)
        own = sum(int(row["quantity"]) for row in holdings if row["wallet_id"] == native_wallet_id)
        sale = conn.execute(
            "SELECT j.proceeds, j.cost_basis FROM journal_entries j "
            "JOIN transactions t ON t.id=j.transaction_id "
            "WHERE j.profile_id=? AND t.external_id='strike:demo-sell' AND j.entry_type='disposal'",
            (profile_id,),
        ).fetchall()
        excluded = conn.execute(
            "SELECT count(*) FROM transactions WHERE profile_id=? AND excluded=1",
            (profile_id,),
        ).fetchone()[0]
        return {
            "platform_quantity_msat": platform, "self_custody_quantity_msat": own,
            "combined_quantity_msat": platform+own,
            "remaining_basis_eur": str(sum((D(str(row["cost_basis"])) for row in holdings), D(0))),
            "sale_proceeds_eur": str(sum((D(str(row["proceeds"])) for row in sale), D(0))),
            "sale_basis_eur": str(sum((D(str(row["cost_basis"])) for row in sale), D(0))),
            "excluded_transactions": excluded,
        }
    finally:
        conn.close()


def price_native_fixture_rows(
    data_root: Path,
    profile: Mapping[str, Any],
    native_wallet: Mapping[str, Any],
) -> None:
    """Audit row-specific synthetic valuations without touching shared rate cache."""
    from kassiber.cli.handlers import _metadata_hooks
    from kassiber.core import metadata, pricing

    conn = open_db(str(data_root))
    try:
        rows = conn.execute("SELECT id FROM transactions WHERE profile_id=? AND wallet_id=?",
                            (profile["id"], native_wallet["id"])).fetchall()
        if len(rows) != 2:
            raise RuntimeError("Exchange native wallet must contain exactly two observed movements")
        for row in rows:
            metadata.update_transaction_metadata(
                conn, profile["workspace_id"], profile["id"], row["id"], _metadata_hooks(),
                pricing_update={"fiat_rate": "100100", "source_kind": pricing.SOURCE_MANUAL_OVERRIDE,
                                "quality": pricing.QUALITY_EXACT, "method": "regtest_exchange_oracle"},
                source="cli",
                reason="Synthetic exchange fixture valuation; independent of historical market data",
            )
    finally:
        conn.close()


def reconcile_exports(
    *,
    data_root: Path,
    artifact_dir: Path,
    workspace: str,
    profile: Mapping[str, Any],
    native_wallet: Mapping[str, Any],
    txids: Mapping[str, str],
    times: Mapping[str, str],
    cli: Callable[..., dict[str, Any]],
) -> dict[str, Any]:
    """Public CLI import, missing-history hold, portable review and exact oracle."""
    scope = ("--workspace", workspace, "--profile", profile["id"])
    files = write_exports(artifact_dir, txids=txids, times=times)
    partial = cli(data_root, "wallets", "import-strike", *scope, "--file", str(files["incomplete"]))["data"]
    cli(data_root, "journals", "process", *scope)
    incomplete = cli(data_root, "review", "cases", *scope, "--limit", "100")["data"]
    partial_rows = cli(data_root, "transactions", "list", *scope, "--limit", "100")["data"]
    withdrawal_ids = {row["id"] for row in partial_rows if row["external_id"] == txids["withdrawal"]
                      and row["wallet"] != native_wallet["label"]}
    if not any(case["transaction_id"] in withdrawal_ids and case["reason"] == "insufficient_lots"
               for case in incomplete["cases"]):
        raise RuntimeError(
            "Missing purchase export must hold the platform withdrawal for insufficient_lots: "
            f"{incomplete['cases']}"
        )
    complete = cli(data_root, "wallets", "import-strike", *scope, "--file", str(files["complete"]))["data"]
    rows = cli(data_root, "transactions", "list", *scope, "--limit", "100")["data"]
    def anchor(txid: str, owned: bool) -> str:
        matches = [row for row in rows if row["external_id"] == txid
                   and (row["wallet"] == native_wallet["label"]) == owned]
        if len(matches) != 1:
            raise RuntimeError(f"Expected one {'native' if owned else 'platform'} anchor for {txid}")
        return matches[0]["id"]
    components = []
    routes = (
        ("withdrawal", False, 600_000_000, 10_000_000),
        ("deposit", True, 200_000_000, 1_000_000),
    )
    for key, source_owned, principal, fee in routes:
        components.append({
            "component_type": "manual_bridge", "evidence_kind": "manual_claim", "evidence_grade": "reviewed",
            "change_reason": "Synthetic Strike export reference and regtest transaction reviewed",
            "notes": f"Exchange {key}; reviewed export evidence, not native observer authority.",
            "legs": [
                {
                    "id": f"exchange-{key}-source", "role": "source",
                    "transaction": anchor(txids[key], source_owned),
                    "amount_msat": str(principal + fee),
                },
                {
                    "id": f"exchange-{key}-destination", "role": "destination",
                    "transaction": anchor(txids[key], not source_owned),
                    "amount_msat": str(principal),
                },
                {"id": f"exchange-{key}-fee", "role": "fee", "amount_msat": str(fee)},
            ],
        })
    operations = [{
        "type": "custody_component",
        "request": {"action": "create", "components": components, "activate": True},
    }]
    operations_file = artifact_dir / "review-operations.json"
    operations_file.write_text(json.dumps(operations, indent=2))
    cases = cli(data_root, "review", "cases", *scope)["data"]
    plan = cli(data_root, "review", "plan", *scope, "--operations-file", str(operations_file),
               "--expected-input-version", str(cases["input_version"]))
    plan_file = artifact_dir / "review-plan.json"
    plan_file.write_text(json.dumps(plan, indent=2))
    receipt = cli(data_root, "review", "apply", *scope, "--artifact-file", str(plan_file),
                  "--idempotency-key", "regtest-exchange-lifecycle")["data"]
    if not receipt["verification"]["report_ready"]:
        raise RuntimeError(
            f"Complete exchange export and reviewed custody remain blocked: {receipt['verification']}"
        )
    actual = _read_state(data_root, profile["id"], native_wallet["id"])
    for key, expected in EXPECTED.items():
        matches = (
            actual[key] == expected
            if isinstance(expected, int)
            else abs(D(str(actual[key])) - D(expected)) <= D("0.00000001")
        )
        if not matches:
            raise RuntimeError(f"Independent exchange oracle {key}: {actual[key]} != {expected}")
    if actual["excluded_transactions"]:
        raise RuntimeError("Exchange scenario must never clear quarantine by exclusion")
    cli(data_root, "wallets", "update", *scope, "--wallet", complete["wallet"],
        "--config", json.dumps({"source_file": str(files["complete"]), "source_format": "strike_csv"}))
    repeated = cli(data_root, "wallets", "import-strike", *scope, "--file", str(files["complete"]))["data"]
    if int(repeated.get("imported") or 0) != 0:
        raise RuntimeError("Repeated exchange export imported duplicate transactions")
    file_sync = cli(data_root, "wallets", "sync", *scope, "--wallet", complete["wallet"])["data"]
    if any(int(item.get("imported") or 0) for item in file_sync):
        raise RuntimeError("Refreshing the exchange CSV connection imported duplicate transactions")
    cli(data_root, "journals", "process", *scope)
    if _read_state(data_root, profile["id"], native_wallet["id"]) != actual:
        raise RuntimeError("Repeated exchange export changed accounting")
    return {
        "profile": profile, "connection_kind": "synthetic Strike CSV import", "expected": EXPECTED,
        "actual": actual, "incomplete_export_cases": incomplete["cases"],
        "imports": {
            "incomplete": partial, "complete": complete, "repeat": repeated, "file_sync": file_sync,
        },
        "review_receipt_id": receipt["id"],
        "artifacts": {**{key: str(path) for key, path in files.items()}, "review_plan": str(plan_file)},
        "case_cards": [
            {
                "id": "exchange-missing-purchase", "title": "Missing exchange purchase export",
                "before": "unresolved", "after": "resolved",
                "evidence": "Complete synthetic Strike CSV through production importer",
            },
            {
                "id": "exchange-custody-roundtrip", "title": "Exchange withdrawal and redeposit",
                "status": "verified", "evidence": "Reviewed export references; no native authority upgrade",
            },
            {
                "id": "exchange-trade-basis", "title": "Trade price and carried basis",
                "status": "verified", "evidence": "Independent decimal oracle",
            },
        ],
    }


def run_exchange_cases(
    *,
    data_root: Path,
    artifact_dir: Path,
    workspace_label: str,
    rpc_url: str,
    username: str,
    password: str,
    faucet_wallet: str,
    mining_address: str,
    current_ts: int,
    config: Mapping[str, Any],
    core_wallets: list[str] | None = None,
) -> dict[str, Any]:
    """Run the small connected exchange book alongside the large main demo.

    ``core_wallets`` is the caller's cleanup list, updated immediately after
    each Core wallet is created so a later failed assertion cannot orphan it.
    """
    from tests.integration import regtest_demo as demo

    if not config.get("enabled", True):
        return {"enabled": False, "current_ts": current_ts, "core_wallets": []}
    def call(method: str, params: list | None = None, **kwargs: Any) -> Any:
        return demo.rpc(rpc_url, username, password, method, params, **kwargs)

    if call("getblockchaininfo")["chain"] != "regtest":
        raise RuntimeError("Exchange fixture requires a regtest Core node")
    created = []
    wallets = {}
    for key, label in (("native", "Exchange self custody"), ("hot", "Fixture exchange hot wallet")):
        name = f"kassiber-exchange-{uuid.uuid4().hex[: 10]}-{key}"
        demo._ensure_wallet(rpc_url, username, password, name)
        created.append(name)
        if core_wallets is not None:
            core_wallets.append(name)
        addresses = [call("getnewaddress", [f"{label} {index}", "bech32"], wallet=name) for index in range(2)]
        wallets[key] = demo.DemoWallet(key=key, label=label, account="exchange", core_wallet=name,
                                       address=addresses[0], addresses=addresses)
    hot, native = wallets["hot"], wallets["native"]
    call("sendtoaddress", [hot.address, D("0.02")], wallet=faucet_wallet)
    current_ts = demo._mine(rpc_url, username, password, faucet_wallet, mining_address, current_ts)
    times = {"buy_time": _iso(current_ts)}
    foreign = call("getnewaddress", ["Unrelated exchange customer", "bech32"], wallet=faucet_wallet)
    withdrawal = demo._send_from_wallet(
        rpc_url, username, password, hot,
        {native.address: D("0.006"), foreign: D("0.004")}, D("0.00002"),
    )
    current_ts = demo._mine(rpc_url, username, password, faucet_wallet, mining_address, current_ts)
    times["withdrawal_time"] = _iso(current_ts)
    deposit = demo._send_from_wallet(
        rpc_url, username, password, native, {hot.address: D("0.002")}, D("0.00001"),
    )
    current_ts = demo._mine(rpc_url, username, password, faucet_wallet, mining_address, current_ts)
    times["deposit_time"] = _iso(current_ts)
    times["sell_time"] = _iso(current_ts + 60)
    txids = {"withdrawal": withdrawal, "deposit": deposit}
    # Core is an independent network oracle, not an accounting-engine result.
    batch_fee = abs(D(str(call("gettransaction", [withdrawal], wallet=hot.core_wallet)["fee"])))
    native_fee = abs(D(str(call("gettransaction", [deposit], wallet=native.core_wallet)["fee"])))
    if batch_fee != D("0.00002") or native_fee != D("0.00001"):
        raise RuntimeError("Core transaction fee differs from the independent exchange fixture")
    profile = demo.run_cli(
        data_root, "profiles", "create", config.get("profile_label", "Exchange Reconciliation"),
        "--workspace", workspace_label, "--fiat-currency", "EUR",
        "--tax-country", "generic", "--gains-algorithm", "FIFO",
    )["data"]
    scope = ("--workspace", workspace_label, "--profile", profile["id"])
    address_args = [item for address in native.addresses for item in ("--address", address)]
    native_record = demo.run_cli(
        data_root, "wallets", "create", *scope, "--label", native.label,
        "--kind", "address", "--backend", config.get("backend", "core-regtest"),
        "--chain", "bitcoin", "--network", "regtest",
        "--birthday", times["buy_time"], *address_args,
    )["data"]
    from kassiber.core.sync_backends import bitcoinrpc_wallet_name

    observer_wallet = bitcoinrpc_wallet_name(
        {"wallet_prefix": config.get("wallet_prefix", "kassiber")}, native_record,
    )
    created.append(observer_wallet)
    if core_wallets is not None:
        core_wallets.append(observer_wallet)
    synced = demo.run_cli(data_root, "wallets", "sync", *scope, "--wallet", native_record["id"])["data"]
    price_native_fixture_rows(data_root, profile, native_record)
    result = reconcile_exports(
        data_root=data_root, artifact_dir=artifact_dir, workspace=workspace_label,
        profile=profile, native_wallet=native_record, txids=txids, times=times, cli=demo.run_cli,
    )
    synced_again = demo.run_cli(data_root, "wallets", "sync", *scope, "--wallet", native_record["id"])["data"]
    if any(int(item.get("imported") or 0) for item in synced_again):
        raise RuntimeError("Repeated native sync imported duplicate transactions")
    demo.run_cli(data_root, "journals", "process", *scope)
    if _read_state(data_root, profile["id"], native_record["id"]) != result["actual"]:
        raise RuntimeError("Repeated native sync changed exchange accounting")
    native_utxos = call("listunspent", [1, 9999999, native.addresses], wallet=native.core_wallet)
    native_balance = sum(D(str(row["amount"])) for row in native_utxos)
    if native_balance != D("0.00399"):
        raise RuntimeError("Core UTXO balance differs from the independent custody oracle")
    result.update({
        "enabled": True, "current_ts": current_ts, "core_wallets": created,
        "txids": txids, "times": times, "native_sync": synced, "native_resync": synced_again,
        "fee_evidence": {
            "platform_withdrawal_charge": {"btc": "0.0001", "source": "synthetic Strike CSV"},
            "foreign_batch_network_fee": {"btc": str(batch_fee), "source": "real Bitcoin Core regtest"},
            "owned_return_network_fee": {"btc": str(native_fee), "source": "real Bitcoin Core regtest"},
        },
    })
    result_file = artifact_dir / "exchange-case-results.json"
    result["artifacts"]["results"] = str(result_file)
    result_file.write_text(json.dumps(result, indent=2, sort_keys=True))
    return result
