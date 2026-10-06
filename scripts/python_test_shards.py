#!/usr/bin/env python3
"""Assign every Python test module to exactly one CI lane.

The mapping is intentionally file-based.  Pytest-xdist may distribute tests
inside the safe domain shards, while socket/process-sensitive modules and the
opt-in integration modules stay in isolated serial lanes.  New test modules fall
back to core-accounting so discovery can never silently omit them.

The tax-shadow overlay sits across the lanes instead of being one: the modules
that reach the tax engine keep their lane and run a second time with
`KASSIBER_TEST_TAX_ENGINE=shadow`, which compares production with the frozen
oracle on every ledger build.
"""

from __future__ import annotations

import argparse
import ast
import os
import re
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
    }
)
_DAEMON_SERIAL_MODULES = frozenset({"test_cli_chat", "test_daemon_smoke", "test_mcp_server"})
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
)
_DAEMON_CLI_TOKENS = (
    "ai_core",
    "chain_analysis_consent",
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

# A test module reaches the tax engine when its source, or the source of a
# `tests` module it imports (transitively), contains one of these calls
# (regular expressions; a CLI argument list may wrap between its words).
SHADOW_CALL_PATTERNS = (
    r"process_journals\(",
    r"build_ledger_state\(",
    r"KassiberTaxEngine\(",
    r"GenericRP2TaxEngine\(",
    r"CustodyJournalBuilder\(",
    r"""["']journals["'],\s*["']process["']""",
    r"ui\.journals\.process",
)
_SHADOW_CALL = re.compile("|".join(SHADOW_CALL_PATTERNS))
# Engine callers that only the census mode (`KASSIBER_TEST_TAX_ENGINE=census`)
# found, because they reach the engine through a product path none of the
# patterns name.
_SHADOW_EXTRA_MODULES = frozenset(
    {
        # `kassiber.core.review_workflow` builds the ledger for its previews.
        "test_review_cli_ai",
        "test_review_workflow",
        # The daemon processes stale journals before its AI read tools and
        # its `_DIRECT_AUTO_JOURNAL_REFRESH_KINDS` reads
        # (`daemon_freshness._auto_maintain_for_read`).
        "test_chain_analysis_consent",
        "test_daemon_swap_matching",
    }
)
SHADOW_MODE = "shadow"


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


def _imported_test_modules(path: Path, root: Path) -> frozenset[Path]:
    """The `tests` modules that ``path`` imports, as paths relative to ``root``."""
    tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    package = path.relative_to(root).with_suffix("").parts[:-1]
    names: list[str] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            names.extend(alias.name for alias in node.names)
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                base = package[: len(package) - node.level + 1]
                module = ".".join((*base, *([node.module] if node.module else [])))
            else:
                module = node.module or ""
            names.append(module)
            names.extend(f"{module}.{alias.name}" for alias in node.names)
    found = set()
    for name in names:
        parts = name.split(".")
        if parts[0] != "tests":
            continue
        for candidate in (root.joinpath(*parts[:-1], f"{parts[-1]}.py"), root.joinpath(*parts, "__init__.py")):
            if candidate.is_file():
                found.add(candidate.relative_to(root))
    return frozenset(found)


def shadow_overlay(root: Path = ROOT) -> tuple[Path, ...]:
    """The test modules that reach the tax engine, in discovery order."""
    sources = {
        path.relative_to(root): path
        for path in (root / "tests").rglob("*.py")
        if "__pycache__" not in path.parts
    }
    calls = {
        path
        for path, absolute in sources.items()
        if _SHADOW_CALL.search(absolute.read_text(encoding="utf-8"))
    }
    imports: dict[Path, frozenset[Path]] = {}

    def reaches_engine(start: Path) -> bool:
        seen, pending = {start}, [start]
        while pending:
            path = pending.pop()
            if path in calls:
                return True
            if path not in imports:
                imports[path] = _imported_test_modules(sources[path], root)
            for imported in imports[path] - seen:
                seen.add(imported)
                pending.append(imported)
        return False

    return tuple(
        path
        for path in discover_test_files(root)
        if path.stem in _SHADOW_EXTRA_MODULES or reaches_engine(path)
    )


def shadow_lanes(root: Path = ROOT) -> tuple[str, ...]:
    """The lanes that hold at least one overlay module, in lane order."""
    overlay = set(shadow_overlay(root))
    manifest = validate_manifest(root)
    return tuple(shard for shard in ALL_SHARDS if overlay.intersection(manifest[shard]))


def _shadow_paths(lane: str | None) -> tuple[Path, ...]:
    overlay = shadow_overlay()
    if lane is None:
        return overlay
    return tuple(path for path in validate_manifest()[lane] if path in overlay)


def _run_pytest(
    paths: Sequence[Path],
    *,
    distribution: str | None,
    pytest_args: Sequence[str],
    tax_mode: str | None = None,
) -> int:
    args = [str(ROOT / path) for path in paths]
    if distribution is not None:
        args.extend(("-n", "2", "--dist", distribution, "--max-worker-restart=0"))
    args.extend(pytest_args)
    command = [sys.executable, "-m", "pytest", *args]
    env = None
    prefix = ""
    if tax_mode is not None:
        env = {**os.environ, "KASSIBER_TEST_TAX_ENGINE": tax_mode}
        prefix = f"KASSIBER_TEST_TAX_ENGINE={tax_mode} "
    print(">", prefix + " ".join(command), flush=True)
    return subprocess.call(command, cwd=ROOT, env=env)


def _run_shard(shard: str, *, ci: bool, pytest_args: Sequence[str]) -> int:
    manifest = validate_manifest()
    distribution = PARALLEL_SHARD_DISTRIBUTION[shard] if ci and shard in PARALLEL_SHARDS else None
    return _run_pytest(manifest[shard], distribution=distribution, pytest_args=pytest_args)


def _run_shadow(lane: str | None, *, ci: bool, pytest_args: Sequence[str]) -> int:
    paths = _shadow_paths(lane)
    if not paths:
        # An empty argument list would make pytest collect the whole suite.
        print(f"{lane}: no test module reaches the tax engine", flush=True)
        return 0
    distribution = PARALLEL_SHARD_DISTRIBUTION.get(lane) if ci and lane is not None else None
    return _run_pytest(paths, distribution=distribution, pytest_args=pytest_args, tax_mode=SHADOW_MODE)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="Validate and summarize the manifest")

    list_parser = subparsers.add_parser("list", help="List files in one shard")
    list_parser.add_argument("shard", choices=ALL_SHARDS)

    run_parser = subparsers.add_parser("run", help="Run one shard with pytest")
    run_parser.add_argument("shard", choices=ALL_SHARDS)
    run_parser.add_argument("--ci", action="store_true", help="Enable safe xdist workers")

    list_shadow_parser = subparsers.add_parser(
        "list-shadow", help="List the tax-shadow overlay, or its part in one shard"
    )
    list_shadow_parser.add_argument("shard", nargs="?", choices=ALL_SHARDS)

    run_shadow_parser = subparsers.add_parser(
        "run-shadow",
        help="Run the tax-shadow overlay, or its part in one shard, with KASSIBER_TEST_TAX_ENGINE=shadow",
    )
    run_shadow_parser.add_argument("shard", nargs="?", choices=ALL_SHARDS)
    run_shadow_parser.add_argument(
        "--ci", action="store_true", help="Enable the shard's safe xdist workers"
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = _parser()
    argv = list(sys.argv[1:] if argv is None else argv)
    # Everything after `--` goes to pytest. Splitting here keeps an optional
    # shard argument from swallowing the first pytest argument.
    passthrough: list[str] = []
    if "--" in argv:
        split = argv.index("--")
        argv, passthrough = argv[:split], argv[split + 1 :]
    args, pytest_args = parser.parse_known_args(argv)
    pytest_args = [*pytest_args, *passthrough]
    manifest = validate_manifest()
    if args.command == "validate":
        if pytest_args:
            parser.error(f"unrecognized arguments: {' '.join(pytest_args)}")
        for shard in ALL_SHARDS:
            mode = "parallel" if shard in PARALLEL_SHARDS else "serial"
            print(f"{shard}: {len(manifest[shard])} modules ({mode})")
        return 0
    if args.command in {"list", "list-shadow"}:
        if pytest_args:
            parser.error(f"unrecognized arguments: {' '.join(pytest_args)}")
        paths = manifest[args.shard] if args.command == "list" else _shadow_paths(args.shard)
        for path in paths:
            print(path.as_posix())
        return 0
    if args.command == "run-shadow":
        return _run_shadow(args.shard, ci=args.ci, pytest_args=pytest_args)
    return _run_shard(args.shard, ci=args.ci, pytest_args=pytest_args)


if __name__ == "__main__":
    raise SystemExit(main())
