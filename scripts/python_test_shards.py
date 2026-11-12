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
import functools
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
# Shards whose classes must stay on one worker; the rest pass per-test
# distribution at 16 workers, where `--dist worksteal` keeps every worker busy.
_GROUPED_SHARDS = frozenset({"daemon-cli"})

_PREFLIGHT_MODULES = frozenset(
    {
        "test_ci_shards",
        "test_connection_catalog_drift",
        "test_dependency_drift",
        "test_homebrew_cask",
        "test_inventory",
        "test_contracts",
        "test_report_contract_drift",
        "test_test_data_lock",
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
        "test_rust_cln_acquisition",
        "test_rust_lightning_acquisition",
        "test_rust_lightning_service",
        "test_rust_desktop_lightning_test",
        "test_rust_desktop_settings_egress",
        "test_sync_backends",
        "test_freshness_prefetch_cancellation",
    }
)
_DAEMON_SERIAL_MODULES = frozenset(
    {
        "test_cli_chat",
        "test_daemon_smoke",
        "test_mcp_server",
        "test_rust_hybrid_candidate",
        "test_rust_grouped_metadata_services",
        "test_rust_transaction_desktop",
        "test_rust_transaction_resolve",
        "test_rust_transaction_dashboard",
        "test_rust_report_capital_gains",
        "test_rust_rates_rebuild",
        "test_rust_audit_projections",
        "test_rust_custody_projections",
        "test_rust_transfer_review_context",
        "test_rust_export_preview",
        "test_rust_desktop_settings",
        "test_rust_desktop_backends",
        "test_rust_desktop_utxos",
        "test_rust_project_desktop",
        "test_rust_workspace_shell",
        "test_rust_daemon_diagnostics",
        "test_rust_release_cli",
        "test_rust_review_workflow",
        "test_rust_report_blockers",
        "test_rust_maintenance_run",
        "test_rust_report_read_sync",
        "test_rust_evidence_summary",
        "test_rust_transaction_graph",
        "test_rust_transaction_review_context",
        "test_rust_desktop_projections_egress",
        "test_rust_desktop_settings",
        "test_rust_document_services",
        "test_rust_wallet_onboarding",
        "test_rust_wallet_identify_onchain",
        "test_rust_wallet_import_desktop",
        "test_rust_wallet_document_import",
        "test_rust_wallet_reveal",
        "test_rust_document_ingestion",
        "test_rust_document_cli",
        "test_rust_chain_analysis_reads",
        "test_rust_chain_analysis_psbt_reads",
        "test_rust_chain_analysis_dataset_preview",
        "test_rust_reports",
        "test_rust_report_exports",
        "test_rust_xlsx_exports",
        "test_rust_austrian_xlsx",
        "test_rust_full_xlsx",
        "test_rust_exit_tax",
        "test_rust_gain_reads",
        "test_rust_cross_check",
        "test_rust_year_handoff",
        "test_rust_platform_csv",
        "test_rust_source_funds_reads",
        "test_rust_source_funds_case_writes",
        "test_rust_source_funds_mutations",
        "test_rust_source_funds_recipients",
        "test_rust_source_funds_links",
        "test_rust_source_funds_diagnostics",
        "test_rust_source_funds_batch",
        "test_rust_source_funds_saves",
        "test_rust_source_funds_report",
        "test_rust_shell_parity",
        "test_rust_wallet_discovery",
        "test_rust_wallet_acquisition",
        "test_rust_observer_state",
        "test_rust_bdk_execution",
        "test_rust_lwk_execution",
        "test_rust_lwk_sync_service",
        "test_rust_bitcoin_records",
        "test_rust_wallet_publication",
        "test_rust_wallet_fetch",
        "test_rust_wallet_sync_service",
        "test_rust_wallet_sync_cli",
        "test_rust_wallet_sync_gaps",
        "test_rust_freshness_queue",
        "test_rust_freshness_admission",
        "test_rust_freshness_execution",
        "test_rust_freshness_native",
        "test_rust_freshness_lifecycle",
        "test_rust_freshness_graph",
        "test_rust_freshness_product",
        "test_rust_connections",
        "test_rust_wallet_sync_sources",
        "test_rust_wallet_sync_btcpay",
        "test_rust_freshness_journals",
        "test_rust_source_funds_review",
        "test_rust_source_funds_coverage",
        "test_rust_source_funds_suggest",
        "test_rust_source_funds_passes",
        "test_rust_source_funds_routes",
        "test_rust_journal_desktop",
    }
)
_REGRESSION_SERIAL_MODULES = frozenset({"test_review_regressions"})
_PROCESS_SERIAL_MODULES = frozenset(
    {
        "test_backup_recovery",
        "test_rust_backup_install",
        "test_rust_backup_desktop",
        "test_custody_component_surfaces",
        "test_daemon_swap_matching",
        "test_integration_harness_safety",
        "test_lightning_business_plan",
        "test_lightning_business_regtest_helpers",
        "test_remembered_unlock",
        "test_rust_store_interop",
        "test_rust_store_migrations",
        "test_rust_store_partition_reference",
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
        "test_coarse_price_acceptance",
        "test_review_batch_operations",
        # Their AI read tools reach the auto-maintained journals below.
        "test_chat_visuals",
        "test_mcp_server",
        # Its Rust vector exporters call the Python review service in children.
        "test_rust_review_cases",
        # It plans and applies through Python's review service for its references.
        "test_rust_review_workflow",
        # The daemon processes stale journals before its AI read tools and
        # its `_DIRECT_AUTO_JOURNAL_REFRESH_KINDS` reads
        # (`daemon_freshness._auto_maintain_for_read`).
        "test_chain_analysis_consent",
        "test_daemon_swap_matching",
        # It runs scripts/check-journal-process-campaign.py, whose
        # `journal_native_process_oracle.capture` processes journals through
        # Python's handlers; the script is loaded by path, not imported.
        "test_journal_process_campaign",
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
    package = path.relative_to(root).with_suffix("").parts[:-1]
    names = _imported_names(path.read_text(encoding="utf-8"), package, str(path))
    found = set()
    for name in names:
        parts = name.split(".")
        if parts[0] != "tests":
            continue
        for candidate in (root.joinpath(*parts[:-1], f"{parts[-1]}.py"), root.joinpath(*parts, "__init__.py")):
            if candidate.is_file():
                found.add(candidate.relative_to(root))
    return frozenset(found)


@functools.lru_cache(maxsize=None)
def _imported_names(source: str, package: tuple[str, ...], filename: str) -> tuple[str, ...]:
    """Every module name ``source`` imports; keyed by content, so an edit misses."""
    tree = ast.parse(source, filename=filename)
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
    return tuple(names)


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
    custody_mode: str | None = None,
    workers: int = 2,
) -> int:
    args = [str(ROOT / path) for path in paths]
    if distribution is not None:
        args.extend(("-n", str(workers), "--dist", distribution, "--max-worker-restart=0"))
    args.extend(pytest_args)
    command = [sys.executable, "-m", "pytest", *args]
    env = None
    prefix = ""
    if tax_mode is not None:
        env = {**os.environ, "KASSIBER_TEST_TAX_ENGINE": tax_mode}
        prefix = f"KASSIBER_TEST_TAX_ENGINE={tax_mode} "
    if custody_mode is not None:
        env = {**(env or os.environ), "KASSIBER_TEST_CUSTODY_ENGINE": custody_mode}
        prefix += f"KASSIBER_TEST_CUSTODY_ENGINE={custody_mode} "
    print(">", prefix + " ".join(command), flush=True)
    return subprocess.call(command, cwd=ROOT, env=env)


def _run_shard(
    shard: str,
    *,
    ci: bool,
    pytest_args: Sequence[str],
    workers: int | None = None,
    dist: str | None = None,
) -> int:
    manifest = validate_manifest()
    if (workers is not None or dist is not None) and shard not in PARALLEL_SHARDS:
        raise SystemExit(f"{shard} runs without xdist; --workers and --dist apply to parallel shards")
    if not ci or shard not in PARALLEL_SHARDS:
        return _run_pytest(manifest[shard], distribution=None, pytest_args=pytest_args)
    if workers is None and dist is None:
        return _run_pytest(manifest[shard], distribution=PARALLEL_SHARD_DISTRIBUTION[shard], pytest_args=pytest_args)
    if dist is not None and shard in _GROUPED_SHARDS:
        raise SystemExit(f"{shard} keeps unittest classes together; run it without --dist")
    return _run_pytest(
        manifest[shard],
        distribution=dist or PARALLEL_SHARD_DISTRIBUTION[shard],
        pytest_args=pytest_args,
        workers=workers or 2,
    )


def _run_shadow(lane: str | None, *, ci: bool, pytest_args: Sequence[str]) -> int:
    paths = _shadow_paths(lane)
    if not paths:
        # An empty argument list would make pytest collect the whole suite.
        print(f"{lane}: no test module reaches the tax engine", flush=True)
        return 0
    distribution = PARALLEL_SHARD_DISTRIBUTION.get(lane) if ci and lane is not None else None
    return _run_pytest(paths, distribution=distribution, pytest_args=pytest_args, tax_mode=SHADOW_MODE)


def _part(value: str) -> str:
    """`K/N` with 1 <= K <= N; tests/conftest.py keeps that part's tests."""
    index, slash, count = value.partition("/")
    if not (slash and index.isdigit() and count.isdigit() and 1 <= int(index) <= int(count)):
        raise argparse.ArgumentTypeError(f"--part must be K/N with 1 <= K <= N, not {value!r}")
    return f"{int(index)}/{int(count)}"


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="Validate and summarize the manifest")

    list_parser = subparsers.add_parser("list", help="List files in one shard")
    list_parser.add_argument("shard", choices=ALL_SHARDS)

    run_parser = subparsers.add_parser("run", help="Run one shard with pytest")
    run_parser.add_argument("shard", choices=ALL_SHARDS)
    run_parser.add_argument("--ci", action="store_true", help="Enable safe xdist workers")
    run_parser.add_argument(
        "--workers", type=int, help="xdist workers for a parallel shard with --ci (default 2, as in CI)"
    )
    run_parser.add_argument(
        "--dist",
        choices=("loadscope", "load", "worksteal"),
        help="Override a per-test-safe parallel shard's distribution with --ci (local runs; CI keeps its policy)",
    )
    run_parser.add_argument("--part", type=_part, help="Run part K of N (K/N): tests split by a stable hash of their node id, for CI lanes too long for one job")

    list_shadow_parser = subparsers.add_parser(
        "list-shadow", help="List the tax-shadow overlay, or its part in one shard"
    )
    list_shadow_parser.add_argument("shard", nargs="?", choices=ALL_SHARDS)

    custody_shadow_parser = subparsers.add_parser("run-custody-shadow", help="Audit the frozen SQL flow against the pure production stages")
    custody_shadow_parser.add_argument("shard", nargs="?", choices=ALL_SHARDS)
    custody_shadow_parser.add_argument("--ci", action="store_true", help="Apply the existing per-shard distribution policy")
    custody_shadow_parser.add_argument("--part", type=_part, help="Run part K of N (K/N): tests split by a stable hash of their node id, for CI lanes too long for one job")

    run_shadow_parser = subparsers.add_parser(
        "run-shadow",
        help="Run the tax-shadow overlay, or its part in one shard, with KASSIBER_TEST_TAX_ENGINE=shadow",
    )
    run_shadow_parser.add_argument("shard", nargs="?", choices=ALL_SHARDS)
    run_shadow_parser.add_argument(
        "--ci", action="store_true", help="Enable the shard's safe xdist workers"
    )
    run_shadow_parser.add_argument("--part", type=_part, help="Run part K of N (K/N): tests split by a stable hash of their node id, for CI lanes too long for one job")
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
    if getattr(args, "part", None):
        # tests/conftest.py's option; xdist workers receive it, a pytest that
        # a test starts does not.
        pytest_args = ["--kassiber-part", args.part, *pytest_args]
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
    if args.command == "run-custody-shadow":
        # The census lane owns single-build read budgets. Shadow intentionally
        # performs a second frozen SQL build; exclude the exact budget test.
        budget = "tests/test_custody_native_reconciliation.py::test_disjoint_reviewed_routes_limit_observation_reads"
        paths = _shadow_paths(args.shard)
        if not paths:
            print(f"{args.shard}: no custody shadow overlay modules", flush=True)
            return 0
        distribution = PARALLEL_SHARD_DISTRIBUTION.get(args.shard) if args.ci and args.shard is not None else None
        return _run_pytest(paths, distribution=distribution, pytest_args=["--deselect", budget, *pytest_args], custody_mode="shadow")
    if args.command == "run-shadow":
        return _run_shadow(args.shard, ci=args.ci, pytest_args=pytest_args)
    if (args.workers is not None or args.dist is not None) and not args.ci:
        parser.error("--workers and --dist apply with --ci")
    return _run_shard(args.shard, ci=args.ci, pytest_args=pytest_args, workers=args.workers, dist=args.dist)


if __name__ == "__main__":
    raise SystemExit(main())
