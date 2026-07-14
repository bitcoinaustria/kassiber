#!/usr/bin/env python3
"""Assign every Python test module to exactly one CI lane.

The mapping is intentionally file-based.  Pytest-xdist may distribute tests
inside the safe domain shards, while socket/process-sensitive modules and the
opt-in integration modules stay in isolated serial lanes.  New test modules fall
back to core-accounting so discovery can never silently omit them.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
TEST_ROOT = ROOT / "tests"

PREFLIGHT_SHARD = "preflight"
CLI_SMOKE_SHARD = "cli-smoke"
RUNTIME_SHARDS = (
    "core-accounting",
    "wallets-sync",
    "daemon-cli",
    "security-replication",
    "reports-contracts",
    "serial-network",
    "serial-daemon",
    "serial-regressions",
    "serial-integration",
)
ALL_SHARDS = (PREFLIGHT_SHARD, CLI_SMOKE_SHARD, *RUNTIME_SHARDS)
PARALLEL_SHARD_DISTRIBUTION = {
    "core-accounting": "loadscope",
    "wallets-sync": "loadscope",
    # Keep unittest classes together: several CLI regression classes retain
    # process-local fixtures whose item-level distribution is not safe.
    "daemon-cli": "loadscope",
    "security-replication": "loadscope",
    "reports-contracts": "loadscope",
}
PARALLEL_SHARDS = frozenset(PARALLEL_SHARD_DISTRIBUTION)

_PREFLIGHT_MODULES = frozenset(
    {
        "test_ci_shards",
        "test_connection_catalog_drift",
        "test_dependency_drift",
        "test_homebrew_cask",
        "test_report_contract_drift",
        "test_workflow_pins",
    }
)
_CLI_SMOKE_MODULES = frozenset({"test_cli_entrypoint_smoke"})

# These tests bind sockets, exercise OS credential/process behavior, or inspect
# integration scripts.  Keeping the complete module serial avoids shared-host
# races while the domain shards use xdist in isolated worker processes.
_NETWORK_SERIAL_MODULES = frozenset(
    {
        "test_proxy",
        "test_regtest_backend_stack",
        "test_sync_backends",
        "test_sync_replication_s4",
        "test_sync_replication_s5",
    }
)
_DAEMON_SERIAL_MODULES = frozenset({"test_cli_chat", "test_daemon_smoke"})
_REGRESSION_SERIAL_MODULES = frozenset({"test_review_regressions"})
_PROCESS_SERIAL_MODULES = frozenset(
    {
        "test_custody_component_surfaces",
        "test_daemon_swap_matching",
        "test_integration_harness_safety",
        "test_lightning_business_plan",
        "test_lightning_business_regtest_helpers",
        "test_remembered_unlock",
        "test_repo_resolution",
        "test_transaction_edit_history",
        "test_wasabi_import",
    }
)

_SECURITY_TOKENS = (
    "audit",
    "backup",
    "egress",
    "ledger_preview_security",
    "log_ring",
    "privacy",
    "secret",
    "security",
    "sync_replication",
)
_DAEMON_CLI_TOKENS = (
    "ai_core",
    "cli_",
    "core_maintenance",
    "daemon_",
    "managed_settings",
    "projects",
    "review_regressions",
    "source_funds_cli",
    "termrender",
    "workspace_overview",
)
_CORE_ACCOUNTING_TOKENS = (
    "custody_components",
    "custody_journal",
    "generic_ledger_linkage",
    "msat_migration",
    "ownership_transfers",
    "rates_kraken_csv",
    "tax_events",
    "transaction_edit_history",
    "transfer_matching",
)
_WALLET_SYNC_TOKENS = (
    "address_scripts",
    "bdk_observer",
    "chain_observer",
    "exchange_importers",
    "freshness",
    "htlc_parser",
    "http_client",
    "import_ownership",
    "liquid_electrum",
    "lwk_observer",
    "onchain",
    "output_inventory",
    "ownership",
    "payment_hash",
    "profile_policy_label",
    "regtest_harness",
    "retry_helpers",
    "samourai",
    "self_transfer",
    "silent_payments",
    "source_overlap",
    "sync_",
    "wallet_",
    "wasabi",
)
_REPORT_CONTRACT_TOKENS = (
    "austrian",
    "btcpay_commercial",
    "capital_gains",
    "channel_lifecycle",
    "custody",
    "document_import",
    "exit_tax",
    "generic_ledger",
    "lightning",
    "loans",
    "pair_swap",
    "rates_",
    "report_",
    "rp2_",
    "saved_views",
    "source_funds",
    "summary_chart",
    "swap_",
    "tax_",
    "transaction_edit_history",
    "transaction_graph",
    "transfer_matching",
)


def discover_test_files(root: Path = ROOT) -> tuple[Path, ...]:
    tests_root = root / "tests"
    return tuple(
        sorted(
            path.relative_to(root)
            for path in tests_root.rglob("test_*.py")
            if "__pycache__" not in path.parts
        )
    )


def shard_for(path: Path) -> str:
    normalized = path.as_posix()
    module = path.stem
    if module in _PREFLIGHT_MODULES:
        return PREFLIGHT_SHARD
    if module in _CLI_SMOKE_MODULES:
        return CLI_SMOKE_SHARD
    if module in _NETWORK_SERIAL_MODULES:
        return "serial-network"
    if module in _DAEMON_SERIAL_MODULES:
        return "serial-daemon"
    if module in _REGRESSION_SERIAL_MODULES:
        return "serial-regressions"
    if normalized.startswith("tests/integration/") or module in _PROCESS_SERIAL_MODULES:
        return "serial-integration"
    if any(token in module for token in _SECURITY_TOKENS):
        return "security-replication"
    if any(token in module for token in _DAEMON_CLI_TOKENS):
        return "daemon-cli"
    if any(token in module for token in _CORE_ACCOUNTING_TOKENS):
        return "core-accounting"
    if any(token in module for token in _WALLET_SYNC_TOKENS):
        return "wallets-sync"
    if any(token in module for token in _REPORT_CONTRACT_TOKENS):
        return "reports-contracts"
    return "core-accounting"


def shard_manifest(root: Path = ROOT) -> dict[str, tuple[Path, ...]]:
    manifest: dict[str, list[Path]] = {name: [] for name in ALL_SHARDS}
    for path in discover_test_files(root):
        manifest[shard_for(path)].append(path)
    return {name: tuple(paths) for name, paths in manifest.items()}


def validate_manifest(root: Path = ROOT) -> dict[str, tuple[Path, ...]]:
    files = discover_test_files(root)
    manifest = shard_manifest(root)
    assigned = tuple(path for shard in ALL_SHARDS for path in manifest[shard])
    if sorted(assigned) != sorted(files):
        raise RuntimeError("Python test shard manifest is incomplete or overlapping")
    empty = [name for name, paths in manifest.items() if not paths]
    if empty:
        raise RuntimeError(f"Python test shards must not be empty: {', '.join(empty)}")
    return manifest


def _pytest_args(raw_args: Sequence[str]) -> list[str]:
    args = list(raw_args)
    if args and args[0] == "--":
        args.pop(0)
    return args


def _run_shard(shard: str, *, ci: bool, pytest_args: Sequence[str]) -> int:
    manifest = validate_manifest()
    args = [str(ROOT / path) for path in manifest[shard]]
    if ci and shard in PARALLEL_SHARDS:
        args.extend(
            (
                "-n",
                "2",
                "--dist",
                PARALLEL_SHARD_DISTRIBUTION[shard],
                "--max-worker-restart=0",
            )
        )
    args.extend(_pytest_args(pytest_args))
    command = [sys.executable, "-m", "pytest", *args]
    print(">", " ".join(command), flush=True)
    return subprocess.call(command, cwd=ROOT)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="Validate and summarize the manifest")

    list_parser = subparsers.add_parser("list", help="List files in one shard")
    list_parser.add_argument("shard", choices=ALL_SHARDS)

    run_parser = subparsers.add_parser("run", help="Run one shard with pytest")
    run_parser.add_argument("shard", choices=ALL_SHARDS)
    run_parser.add_argument("--ci", action="store_true", help="Enable safe xdist workers")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    args, pytest_args = parser.parse_known_args(argv)
    manifest = validate_manifest()
    if args.command == "validate":
        if pytest_args:
            parser.error(f"unrecognized arguments: {' '.join(pytest_args)}")
        for shard in ALL_SHARDS:
            mode = "parallel" if shard in PARALLEL_SHARDS else "serial"
            print(f"{shard}: {len(manifest[shard])} modules ({mode})")
        return 0
    if args.command == "list":
        if pytest_args:
            parser.error(f"unrecognized arguments: {' '.join(pytest_args)}")
        for path in manifest[args.shard]:
            print(path.as_posix())
        return 0
    return _run_shard(args.shard, ci=args.ci, pytest_args=pytest_args)


if __name__ == "__main__":
    raise SystemExit(main())
