#!/usr/bin/env python3
"""Classify every Python test by how its coverage moves to the Rust backend.

Plan 21 (docs/plan/21-rust-backend.md, Verification 3) moves each group's
tests with its implementation. ``tests/rust_port/dispositions.json`` assigns
every test class and module-level test function one disposition and one
plan-21 group. A class whose methods differ gets entries per method
(``Class::test_name``) for the methods that differ from the class entry.

The heuristic below reads each module's AST: how a test reaches the product
(a ``python -m kassiber`` child, an in-process CLI/daemon/MCP entry, or
direct calls into ``kassiber``), what it patches, and whether it opens the
database, replays tapes, or spawns processes. Hand-reviewed decisions live in
``tests/rust_port/reviews.py`` and override the heuristic; an entry they
decide is written with ``"reviewed": true``.

Run from the repository root:

    uv run --locked python -m tests.rust_port.classify_tests update
    uv run --locked python -m tests.rust_port.classify_tests check
    uv run --locked python -m tests.rust_port.classify_tests summary
    uv run --locked python -m tests.rust_port.classify_tests explain tests/test_x.py

``update`` rewrites the inventory deterministically from the test sources
and the reviews. ``check`` (and ``tests/rust_port/test_inventory.py``)
fails when a test module, class, or module-level test function has no entry,
or an entry names a test that no longer exists; it checks method counts too,
so extending a class requires regenerating.
Flags are conservative static evidence, not runtime coverage.
"""

from __future__ import annotations

import argparse
import ast
import json
import re
import sys
from collections import Counter, defaultdict
from collections.abc import Iterable, Iterator
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
INVENTORY = ROOT / "tests" / "rust_port" / "dispositions.json"
SCHEMA_VERSION = 1

DISPOSITIONS = {
    "boundary-fixture": (
        "drives the CLI, daemon, MCP server, or broker as a `python -m kassiber` "
        "child without mocking Python internals; can become a portable recorded session"
    ),
    "subprocess-scenario": (
        "behavior is observable at the boundary, but the test reaches it in process "
        "(main(), dispatch(), handle_request(), MCP tools) or seeds through Python "
        "internals; rewrite as a black-box scenario against the binary"
    ),
    "rust-test": (
        "unit or property behavior of internals; port as a Rust test of the owning crate"
    ),
    "oracle": (
        "compares implementations or pins the Python reference (tax oracle, goldens, "
        "vectors); keep while Python is the reference"
    ),
    "python-only": (
        "tests Python machinery that disappears with Python (PyInstaller entry, import "
        "guards, sitecustomize hooks, uv lock and pytest lanes); `replacement` names what "
        "replaces its intent"
    ),
}

# Plan 21's service groups, in the plan's order.
GROUPS = {
    "store": "store",
    "custody": "custody",
    "journals": "journals",
    "imports": "imports",
    "reports": "reports",
    "chain-sync": "chain/sync",
    "network-egress": "network/egress",
    "transactions": "transactions/metadata/transfers/reviews",
    "source-funds": "source-funds",
    "secrets-backup": "secrets/backup",
    "daemon-runtime": "daemon runtime",
    "ai-mcp-operator": "AI/MCP/operator",
    "cli-shell": "CLI shell",
    "packaging-ci": "packaging/CI",
}

# Flags the inventory records per entry.
FLAGS = ("mock", "mock-internal", "sql", "sql-seed", "tapes", "subprocess")

# Module stem tokens to groups, first match wins. A token matches at the start
# of an underscore-separated word ("ai" matches `ai_core` and `review_cli_ai`,
# not `samourai`). Broad modules (below) also apply them to item names.
GROUP_TOKENS: tuple[tuple[str, str], ...] = (
    ("tax_oracle", "journals"),
    ("ci_shards", "packaging-ci"),
    ("workflow_pins", "packaging-ci"),
    ("dependency_drift", "packaging-ci"),
    ("homebrew", "packaging-ci"),
    ("linux", "packaging-ci"),
    ("rpm", "packaging-ci"),
    ("macos", "packaging-ci"),
    ("pyinstaller", "packaging-ci"),
    ("desktop_packaging", "packaging-ci"),
    ("release", "packaging-ci"),
    ("build_info", "packaging-ci"),
    ("integration_harness", "packaging-ci"),
    ("regtest_backend_stack", "packaging-ci"),
    ("connection_catalog", "daemon-runtime"),
    ("source_funds", "source-funds"),
    ("chain_analysis", "chain-sync"),
    ("custody_ai", "ai-mcp-operator"),
    ("custody_filed_report", "reports"),
    ("custody", "custody"),
    ("operator", "ai-mcp-operator"),
    ("mcp", "ai-mcp-operator"),
    ("ai", "ai-mcp-operator"),
    ("chat", "ai-mcp-operator"),
    ("agent", "ai-mcp-operator"),
    ("tool", "ai-mcp-operator"),
    ("egress", "network-egress"),
    ("no_egress", "network-egress"),
    ("proxy", "network-egress"),
    ("http", "network-egress"),
    ("retry", "network-egress"),
    ("auth_backoff", "network-egress"),
    ("backend_privacy", "network-egress"),
    ("update", "network-egress"),
    ("offline", "network-egress"),
    ("secret", "secrets-backup"),
    ("unlock", "secrets-backup"),
    ("passphrase", "secrets-backup"),
    ("backup", "secrets-backup"),
    ("restore", "secrets-backup"),
    ("sqlcipher", "secrets-backup"),
    ("encrypt", "secrets-backup"),
    ("rekey", "secrets-backup"),
    ("keyring", "secrets-backup"),
    ("redact", "secrets-backup"),
    ("log_ring", "daemon-runtime"),
    ("sync", "chain-sync"),
    ("freshness", "daemon-runtime"),
    ("daemon_core_probe", "daemon-runtime"),
    ("workspace_overview", "daemon-runtime"),
    ("migration", "store"),
    ("migrat", "store"),
    ("database", "store"),
    ("schema", "store"),
    ("data_root", "store"),
    ("project", "store"),
    ("managed_settings", "store"),
    ("settings", "store"),
    ("repo_resolution", "store"),
    ("historical", "store"),
    ("device_sync", "store"),
    ("maintenance", "store"),
    ("tax_platform", "store"),
    ("import", "imports"),
    ("document", "imports"),
    ("file_analysis", "imports"),
    ("samourai", "imports"),
    ("wasabi", "imports"),
    ("btcpay_commercial", "imports"),
    ("commercial", "imports"),
    ("ledger_preview", "imports"),
    ("kraken_csv", "imports"),
    ("bip329", "imports"),
    ("csv", "imports"),
    ("phoenix", "imports"),
    ("river", "imports"),
    ("strike", "imports"),
    ("coinfinity", "imports"),
    ("bullbitcoin", "imports"),
    ("pocket", "imports"),
    ("report", "reports"),
    ("austrian", "reports"),
    ("summary", "reports"),
    ("export", "reports"),
    ("audit_package", "reports"),
    ("filed", "reports"),
    ("saved_views", "reports"),
    ("pdf", "reports"),
    ("xlsx", "reports"),
    ("exit_tax", "reports"),
    ("balance", "reports"),
    ("capital_gains", "reports"),
    ("year_handoff", "reports"),
    ("tax", "journals"),
    ("rp2", "journals"),
    ("cost_basis", "journals"),
    ("legacy_ledger", "journals"),
    ("generic_ledger", "journals"),
    ("journal", "journals"),
    ("source_overlap", "journals"),
    ("ownership_transfer", "transactions"),
    ("ownership_policy", "transactions"),
    ("ownership_review", "transactions"),
    ("ownership", "chain-sync"),
    ("loan", "transactions"),
    ("transfer", "transactions"),
    ("swap", "transactions"),
    ("review", "transactions"),
    ("quarantine", "transactions"),
    ("acquisition", "transactions"),
    ("transaction", "transactions"),
    ("metadata", "transactions"),
    ("label", "transactions"),
    ("payout", "transactions"),
    ("attachment", "transactions"),
    ("observer", "chain-sync"),
    ("observation", "chain-sync"),
    ("wallet", "chain-sync"),
    ("descriptor", "chain-sync"),
    ("address", "chain-sync"),
    ("script", "chain-sync"),
    ("onchain", "chain-sync"),
    ("output_inventory", "chain-sync"),
    ("liquid", "chain-sync"),
    ("lightning", "chain-sync"),
    ("channel", "chain-sync"),
    ("lnd", "chain-sync"),
    ("cln", "chain-sync"),
    ("htlc", "chain-sync"),
    ("payment_hash", "chain-sync"),
    ("btcpay", "chain-sync"),
    ("silent_payments", "chain-sync"),
    ("book_network", "chain-sync"),
    ("privacy", "chain-sync"),
    ("rates", "chain-sync"),
    ("regtest", "chain-sync"),
    ("bdk", "chain-sync"),
    ("lwk", "chain-sync"),
    ("electrum", "chain-sync"),
    ("bitcoin_core", "chain-sync"),
    ("boltz", "chain-sync"),
    ("graph", "chain-sync"),
    ("backend", "chain-sync"),
    ("demo", "chain-sync"),
    ("daemon", "daemon-runtime"),
    ("termrender", "cli-shell"),
    ("command", "cli-shell"),
    ("entrypoint", "cli-shell"),
    ("cli", "cli-shell"),
)
_GROUP_PATTERNS = tuple(
    (re.compile(rf"(?:^|_){re.escape(token)}"), group) for token, group in GROUP_TOKENS
)

# Modules whose classes and methods span many groups; their item names choose
# the group before the module default does.
BROAD_MODULES = frozenset(
    {
        "tests/test_cli_smoke.py",
        "tests/test_daemon_smoke.py",
        "tests/test_review_regressions.py",
    }
)

SUBPROCESS_CALLS = frozenset(
    {
        "subprocess.run",
        "subprocess.Popen",
        "subprocess.call",
        "subprocess.check_call",
        "subprocess.check_output",
        "asyncio.create_subprocess_exec",
        "asyncio.create_subprocess_shell",
        "os.system",
        "os.spawnv",
        "os.spawnl",
        "os.execv",
        "os.execvp",
        "pty.spawn",
    }
)
PATCH_CALLS = frozenset(
    {
        "unittest.mock.patch",
        "unittest.mock.patch.object",
        "unittest.mock.patch.dict",
        "unittest.mock.patch.multiple",
        "monkeypatch.setattr",
        "monkeypatch.setitem",
        "monkeypatch.delattr",
    }
)
ENV_PATCH_CALLS = frozenset(
    {"monkeypatch.setenv", "monkeypatch.delenv", "monkeypatch.chdir"}
)
FAKE_SERVER_CALLS = frozenset(
    {
        "http.server.HTTPServer",
        "http.server.ThreadingHTTPServer",
        "socketserver.TCPServer",
        "socketserver.ThreadingTCPServer",
        "socketserver.UnixStreamServer",
    }
)
# Calls into `kassiber` that read nothing from a book and seed nothing.
PASSIVE_KASSIBER_CALLS = frozenset(
    {
        "kassiber.errors.AppError",
        "kassiber.secrets.sqlcipher.sqlcipher_available",
        "kassiber.db.native_state_root",
    }
)
CLI_ENTRY_CALLS = frozenset(
    {
        "kassiber.cli.main.main",
        "kassiber.cli.main.dispatch",
        "kassiber.cli.main.run",
        "kassiber.__main__.main",
        "kassiber.cli.chat.main",
        "kassiber.cli.chat.run_chat",
    }
)
DAEMON_ENTRY_NAMES = frozenset({"handle_request", "run", "DaemonContext"})
MCP_ENTRY_PREFIXES = ("kassiber.mcp.tools.run_tool", "kassiber.mcp.server", "kassiber.mcp.cli")
# `tests/cli_runner.py` is the one place tests start the product as a child.
# Its launchers count as a spawn; its body (candidate switch, session
# recorder) is harness plumbing, not something the calling test exercises.
CLI_RUNNER_MODULE = "tests.cli_runner"
CLI_RUNNER_LAUNCHERS = frozenset({"run", "popen", "argv", "prefix"})
SQL_KEYWORDS = ("SELECT ", "INSERT ", "UPDATE ", "DELETE ", "CREATE ", "PRAGMA ", "DROP ", "ALTER ")
SETUP_NAMES = frozenset(
    {
        "setUp",
        "setUpClass",
        "asyncSetUp",
        "setup_method",
        "setup_class",
        "setUpModule",
        "setup_module",
    }
)
TEST_CASE_BASES = frozenset(
    {
        "unittest.TestCase",
        "unittest.IsolatedAsyncioTestCase",
        "unittest.case.TestCase",
    }
)


# ---------------------------------------------------------------------------
# Module indexing


@dataclass
class ModuleIndex:
    dotted: str
    path: Path
    tree: ast.Module
    imports: dict[str, str]
    functions: dict[str, ast.FunctionDef | ast.AsyncFunctionDef]
    classes: dict[str, ast.ClassDef]


def _dotted_for(path: Path) -> str:
    return ".".join(path.relative_to(ROOT).with_suffix("").parts)


def _import_map(tree: ast.Module, dotted: str, is_package: bool) -> dict[str, str]:
    package = dotted.split(".") if is_package else dotted.split(".")[:-1]
    names: dict[str, str] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.asname:
                    names[alias.asname] = alias.name
                else:
                    head = alias.name.split(".")[0]
                    names.setdefault(head, head)
        elif isinstance(node, ast.ImportFrom):
            if node.level:
                base = package[: len(package) - node.level + 1]
                module = ".".join([*base, *([node.module] if node.module else [])])
            else:
                module = node.module or ""
            for alias in node.names:
                names[alias.asname or alias.name] = f"{module}.{alias.name}"
    # `from unittest import mock` and `import unittest.mock` read the same.
    return names


_INDEX_CACHE: dict[Path, ModuleIndex | None] = {}


def index_module(path: Path) -> ModuleIndex | None:
    path = path.resolve()
    if path in _INDEX_CACHE:
        return _INDEX_CACHE[path]
    try:
        source = path.read_text(encoding="utf-8")
    except OSError:
        _INDEX_CACHE[path] = None
        return None
    tree = ast.parse(source, filename=str(path))
    dotted = _dotted_for(path)
    if path.name == "__init__.py":
        dotted = dotted.rsplit(".", 1)[0]
    index = ModuleIndex(
        dotted=dotted,
        path=path,
        tree=tree,
        imports=_import_map(tree, dotted, path.name == "__init__.py"),
        functions={
            node.name: node
            for node in tree.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        },
        classes={node.name: node for node in tree.body if isinstance(node, ast.ClassDef)},
    )
    _INDEX_CACHE[path] = index
    return index


def _tests_module_path(dotted: str) -> Path | None:
    parts = dotted.split(".")
    if not parts or parts[0] != "tests":
        return None
    for candidate in (ROOT.joinpath(*parts).with_suffix(".py"), ROOT.joinpath(*parts, "__init__.py")):
        if candidate.is_file():
            return candidate
    return None


def resolve_symbol(index: ModuleIndex, qualified: str) -> tuple[ModuleIndex, str] | None:
    """Find a `tests.*` function or class definition for a qualified name."""
    parts = qualified.split(".")
    for cut in range(len(parts) - 1, 0, -1):
        path = _tests_module_path(".".join(parts[:cut]))
        if path is None:
            continue
        target = index_module(path)
        if target is None:
            return None
        name = parts[cut]
        if name in target.functions or name in target.classes:
            return target, name
        imported = target.imports.get(name)
        if imported and imported != qualified:
            return resolve_symbol(target, imported)
        return None
    return None


def dotted_name(node: ast.AST, imports: dict[str, str]) -> str | None:
    parts: list[str] = []
    while isinstance(node, ast.Attribute):
        parts.append(node.attr)
        node = node.value
    if isinstance(node, ast.Name):
        head = imports.get(node.id, node.id)
        if head == "mock":
            head = "unittest.mock"
        elif head.startswith("mock."):
            head = "unittest." + head
        parts.append(head)
        return ".".join(reversed(parts))
    if isinstance(node, ast.Call):
        inner = dotted_name(node.func, imports)
        if inner is not None:
            parts.append(inner + "()")
            return ".".join(reversed(parts))
    return None


# ---------------------------------------------------------------------------
# Features


@dataclass
class Features:
    tags: set[str] = field(default_factory=set)
    kassiber_calls: set[str] = field(default_factory=set)
    patch_targets: set[str] = field(default_factory=set)
    spawn_kinds: set[str] = field(default_factory=set)

    def merge(self, other: "Features") -> bool:
        before = (len(self.tags), len(self.kassiber_calls), len(self.patch_targets), len(self.spawn_kinds))
        self.tags |= other.tags
        self.kassiber_calls |= other.kassiber_calls
        self.patch_targets |= other.patch_targets
        self.spawn_kinds |= other.spawn_kinds
        return before != (
            len(self.tags),
            len(self.kassiber_calls),
            len(self.patch_targets),
            len(self.spawn_kinds),
        )

    def copy(self) -> "Features":
        result = Features()
        result.merge(self)
        return result


def _string_items(node: ast.AST) -> list[str | None]:
    if isinstance(node, (ast.List, ast.Tuple)):
        return [
            item.value if isinstance(item, ast.Constant) and isinstance(item.value, str) else None
            for item in node.elts
        ]
    return []


def _spawns_kassiber(items: list[str | None]) -> bool:
    return any(
        item == "-m" and following is not None and following.split(".")[0] == "kassiber"
        for item, following in zip(items, items[1:])
    )


def _cli_runner_reference(target: str, features: Features) -> bool:
    """Whether `target` names `tests.cli_runner`; a launcher marks a spawn."""
    if target == CLI_RUNNER_MODULE:
        return True
    module, _, symbol = target.rpartition(".")
    if module != CLI_RUNNER_MODULE:
        return False
    if symbol in CLI_RUNNER_LAUNCHERS:
        features.tags.update(("spawn-kassiber", "subprocess"))
    return True


def _spawn_kinds(strings: set[str]) -> set[str]:
    """The surfaces a function that spawns `python -m kassiber` starts.

    Helpers often build the argv prefix as a literal and append the
    subcommand later, so the function's other string constants decide.
    """
    kinds = set()
    if "daemon" in strings or "kassiber.daemon" in strings:
        kinds.add("daemon")
    if "mcp" in strings:
        kinds.add("mcp")
    if "--operator-broker-server" in strings:
        kinds.add("broker")
    if "chat" in strings:
        kinds.add("chat")
    return kinds or {"cli"}


def _patch_target(call: ast.Call, name: str, imports: dict[str, str]) -> str | None:
    if not call.args:
        for keyword in call.keywords:
            if keyword.arg in {"target", "in_dict"}:
                return _expr_target(keyword.value, imports)
        return None
    first = call.args[0]
    if name.endswith(".dict"):
        target = _expr_target(first, imports)
        return "os.environ" if target in {"os.environ", "os.environ()"} else target
    return _expr_target(first, imports)


def _expr_target(node: ast.AST, imports: dict[str, str]) -> str | None:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return dotted_name(node, imports)


def _classify_patch(target: str | None) -> str:
    if target is None:
        return "mock-other"
    if target in {"os.environ", "environ"} or target.startswith("os.environ"):
        return "env"
    root = target.split(".")[0]
    if root == "kassiber":
        return "mock-internal"
    if root in {"tests", "self", "cls"}:
        return "mock-test"
    if root in {"scripts", "dev"}:
        return "mock-script"
    if root and root[0].islower() and root in sys.stdlib_module_names:
        return "mock-stdlib"
    return "mock-other"


def _reads_constant(name: str) -> bool:
    """`kassiber.backends.DEFAULT_BACKENDS.copy()` reads a constant."""
    return any(part.isupper() and len(part) > 1 for part in name.split(".")[1:])


def direct_features(node: ast.AST, index: ModuleIndex) -> tuple[Features, set[tuple[str, str]]]:
    """Features a function's own body shows, plus the helpers it references."""
    features = Features()
    callees: set[tuple[str, str]] = set()
    imports = index.imports
    strings: set[str] = set()
    for child in ast.walk(node):
        if isinstance(child, (ast.List, ast.Tuple)):
            if _spawns_kassiber(_string_items(child)):
                features.tags.add("spawn-kassiber")
        elif isinstance(child, ast.Constant) and isinstance(child.value, str):
            text = child.value
            strings.add(text)
            if "regtest_tapes" in text:
                features.tags.add("tapes")
            upper = text.lstrip().upper()
            if upper.startswith(SQL_KEYWORDS) and len(text) > 12:
                features.tags.add("sql")
                if upper.startswith(("INSERT ", "UPDATE ", "DELETE ", "CREATE ", "DROP ", "ALTER ")):
                    features.tags.add("sql-seed")
        elif isinstance(child, ast.Name) and isinstance(child.ctx, ast.Load):
            target = imports.get(child.id)
            if child.id in index.functions or child.id in index.classes:
                callees.add((index.dotted, child.id))
            elif target is not None and target.startswith("tests."):
                if _cli_runner_reference(target, features):
                    continue
                resolved = resolve_symbol(index, target)
                if resolved is not None:
                    callees.add((resolved[0].dotted, resolved[1]))
                if "tapes" in target:
                    features.tags.add("tapes")
        elif isinstance(child, ast.Attribute) and isinstance(child.ctx, ast.Load):
            if isinstance(child.value, ast.Name) and child.value.id in {"self", "cls"}:
                callees.add((index.dotted, f"<self>.{child.attr}"))
            else:
                qualified = dotted_name(child, imports)
                if qualified and qualified.startswith("tests."):
                    if _cli_runner_reference(qualified, features):
                        continue
                    resolved = resolve_symbol(index, qualified)
                    if resolved is not None:
                        callees.add((resolved[0].dotted, resolved[1]))
        if not isinstance(child, ast.Call):
            continue
        name = dotted_name(child.func, imports)
        if name is None:
            continue
        if name in SUBPROCESS_CALLS:
            features.tags.add("subprocess")
        if name in FAKE_SERVER_CALLS:
            features.tags.add("fake-server")
        if name == "sqlite3.connect" or name.endswith("sqlcipher3.connect"):
            features.tags.add("sql")
        if name in PATCH_CALLS or name.startswith("unittest.mock.patch"):
            target = _patch_target(child, name, imports)
            kind = _classify_patch(target)
            if kind == "env":
                features.tags.add("env")
            elif kind != "mock-test":
                features.tags.add(kind)
                if target:
                    features.patch_targets.add(target)
        elif name in ENV_PATCH_CALLS:
            features.tags.add("env")
        if "Tape" in name.split(".")[-1] or "tapes" in name.split("."):
            features.tags.add("tapes")
        if not name.startswith("kassiber.") and name != "kassiber":
            continue
        if name in PASSIVE_KASSIBER_CALLS or _reads_constant(name):
            continue
        features.kassiber_calls.add(name)
        if name.startswith("kassiber.db.") or name.endswith(".open_db"):
            features.tags.add("sql")
        if name in CLI_ENTRY_CALLS:
            features.tags.add("in-process-cli")
        elif name.startswith("kassiber.daemon."):
            leaf = name.split(".")[2]
            if leaf in DAEMON_ENTRY_NAMES or (leaf.startswith("_") and "payload" in leaf):
                features.tags.add("in-process-daemon")
        elif name.startswith(MCP_ENTRY_PREFIXES):
            features.tags.add("in-process-mcp")
    if "spawn-kassiber" in features.tags:
        features.spawn_kinds |= _spawn_kinds(strings)
    # Decorators such as @mock.patch("kassiber...") are part of the function.
    features.merge(decorator_features(getattr(node, "decorator_list", ()), imports))
    return features, callees


def decorator_features(decorators: Iterable[ast.expr], imports: dict[str, str]) -> Features:
    features = Features()
    for decorator in decorators:
        if not isinstance(decorator, ast.Call):
            continue
        name = dotted_name(decorator.func, imports) or ""
        if name in PATCH_CALLS or name.startswith("unittest.mock.patch"):
            target = _patch_target(decorator, name, imports)
            kind = _classify_patch(target)
            if kind not in {"env", "mock-test"}:
                features.tags.add(kind)
                if target:
                    features.patch_targets.add(target)
    return features


class FeatureGraph:
    """Propagates features through the helpers a test reaches."""

    def __init__(self) -> None:
        self._direct: dict[tuple[str, str], Features] = {}
        self._edges: dict[tuple[str, str], set[tuple[str, str]]] = {}
        self._resolved: dict[tuple[str, str], Features] = {}
        self._indexes: dict[str, ModuleIndex] = {}

    def _index(self, dotted: str) -> ModuleIndex | None:
        if dotted in self._indexes:
            return self._indexes[dotted]
        path = _tests_module_path(dotted)
        index = index_module(path) if path else None
        if index is not None:
            self._indexes[dotted] = index
        return index

    def register(self, index: ModuleIndex) -> None:
        self._indexes[index.dotted] = index

    def _nodes_for(self, key: tuple[str, str], owner: str | None) -> list[tuple[tuple[str, str], ast.AST, str | None]]:
        dotted, name = key
        index = self._index(dotted)
        if index is None:
            return []
        if name.startswith("<self>."):
            attr = name.split(".", 1)[1]
            if owner is None:
                return []
            found = self.lookup_method(index, owner, attr)
            return [found] if found else []
        if "." in name:
            class_name, method = name.split(".", 1)
            cls = index.classes.get(class_name)
            if cls is None:
                return []
            for item in cls.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name == method:
                    return [(key, item, class_name)]
            return []
        if name in index.functions:
            return [(key, index.functions[name], None)]
        if name in index.classes:
            cls = index.classes[name]
            return [
                ((dotted, f"{name}.{item.name}"), item, name)
                for item in cls.body
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))
                and not item.name.startswith("test")
            ]
        return []

    def lookup_method(self, index: ModuleIndex, class_name: str, attr: str):
        for owner_index, cls in self.mro(index, class_name):
            for item in cls.body:
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name == attr:
                    return ((owner_index.dotted, f"{cls.name}.{attr}"), item, cls.name)
        return None

    def mro(self, index: ModuleIndex, class_name: str) -> Iterator[tuple[ModuleIndex, ast.ClassDef]]:
        seen: set[tuple[str, str]] = set()
        pending = [(index, class_name)]
        while pending:
            current, name = pending.pop(0)
            if (current.dotted, name) in seen:
                continue
            seen.add((current.dotted, name))
            cls = current.classes.get(name)
            if cls is None:
                imported = current.imports.get(name)
                resolved = resolve_symbol(current, imported) if imported else None
                if resolved is None:
                    continue
                current, name = resolved
                cls = current.classes.get(name)
                if cls is None:
                    continue
            yield current, cls
            for base in cls.bases:
                base_name = dotted_name(base, {})
                if base_name and "." not in base_name:
                    pending.append((current, base_name))

    def features(self, key: tuple[str, str], node: ast.AST, owner: str | None) -> Features:
        return self._resolve(key, node, owner, frozenset())

    def _resolve(self, key, node, owner, stack) -> Features:
        if key in self._resolved:
            return self._resolved[key]
        index = self._index(key[0])
        assert index is not None
        if key not in self._direct:
            direct, callees = direct_features(node, index)
            self._direct[key] = direct
            self._edges[key] = callees
        result = self._direct[key].copy()
        stack = stack | {key}
        complete = True
        for callee in sorted(self._edges[key]):
            for callee_key, callee_node, callee_owner in self._nodes_for(callee, owner):
                if callee_key in stack:
                    complete = False
                    continue
                result.merge(self._resolve(callee_key, callee_node, callee_owner, stack))
        if complete:
            self._resolved[key] = result
        return result


# ---------------------------------------------------------------------------
# Test items


@dataclass
class TestFunction:
    name: str
    features: Features


@dataclass
class TestItem:
    key: str
    kind: str  # "class" or "function"
    tests: list[TestFunction]


def _is_test_class(graph: FeatureGraph, index: ModuleIndex, cls: ast.ClassDef) -> bool:
    if cls.name.startswith("Test") and not any(
        isinstance(item, ast.FunctionDef) and item.name == "__init__" for item in cls.body
    ):
        return True
    for owner, ancestor in graph.mro(index, cls.name):
        for base in ancestor.bases:
            name = dotted_name(base, owner.imports) or ""
            if name in TEST_CASE_BASES or name.endswith("TestCase"):
                return True
    return False


def _fixture_names(node: ast.AST) -> list[str]:
    args = getattr(node, "args", None)
    if args is None:
        return []
    return [arg.arg for arg in args.args if arg.arg not in {"self", "cls"}]


def _is_fixture(node: ast.AST, imports: dict[str, str]) -> tuple[bool, bool]:
    for decorator in getattr(node, "decorator_list", ()):
        target = decorator.func if isinstance(decorator, ast.Call) else decorator
        name = dotted_name(target, imports) or ""
        if name in {"pytest.fixture", "fixture"}:
            autouse = isinstance(decorator, ast.Call) and any(
                keyword.arg == "autouse"
                and isinstance(keyword.value, ast.Constant)
                and keyword.value.value is True
                for keyword in decorator.keywords
            )
            return True, autouse
    return False, False


def collect_items(path: Path, graph: FeatureGraph) -> list[TestItem]:
    index = index_module(ROOT / path)
    assert index is not None
    graph.register(index)
    fixtures = {}
    autouse: list[str] = []
    for name, node in index.functions.items():
        is_fixture, is_autouse = _is_fixture(node, index.imports)
        if is_fixture:
            fixtures[name] = node
            if is_autouse:
                autouse.append(name)
    module_setup = Features()
    for name in ("setUpModule", "setup_module", *autouse):
        node = index.functions.get(name) or fixtures.get(name)
        if node is not None:
            module_setup.merge(graph.features((index.dotted, name), node, None))

    def with_fixtures(features: Features, node: ast.AST, owner: str | None) -> Features:
        result = features.copy()
        result.merge(module_setup)
        for argument in _fixture_names(node):
            fixture = fixtures.get(argument)
            if fixture is not None:
                result.merge(graph.features((index.dotted, argument), fixture, None))
        return result

    items: list[TestItem] = []
    for node in index.tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test"):
            features = graph.features((index.dotted, node.name), node, None)
            items.append(
                TestItem(node.name, "function", [TestFunction(node.name, with_fixtures(features, node, None))])
            )
        elif isinstance(node, ast.ClassDef) and _is_test_class(graph, index, node):
            setup = Features()
            methods: dict[str, tuple[ModuleIndex, ast.AST, str]] = {}
            for owner_index, cls in graph.mro(index, node.name):
                for item in cls.body:
                    if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
                        continue
                    if item.name in SETUP_NAMES:
                        setup.merge(
                            graph.features((owner_index.dotted, f"{cls.name}.{item.name}"), item, cls.name)
                        )
                    elif item.name.startswith("test") and item.name not in methods:
                        methods[item.name] = (owner_index, item, cls.name)
                setup.merge(decorator_features(cls.decorator_list, owner_index.imports))
            if not methods:
                continue
            tests = []
            for method_name, (owner_index, item, owner_class) in methods.items():
                features = graph.features((owner_index.dotted, f"{owner_class}.{method_name}"), item, owner_class)
                merged = with_fixtures(features, item, owner_class)
                merged.merge(setup)
                tests.append(TestFunction(method_name, merged))
            items.append(TestItem(node.name, "class", tests))
    return items


# ---------------------------------------------------------------------------
# Heuristic dispositions


@dataclass(frozen=True)
class Verdict:
    disposition: str
    group: str
    reason: str
    flags: tuple[str, ...]


def module_group(path: Path) -> str:
    stem = path.stem
    if path.parts[:2] == ("tests", "tax_oracle"):
        return "journals"
    return _group_from_tokens(stem.removeprefix("test_")) or "journals"


def _group_from_tokens(text: str) -> str | None:
    text = text.lower()
    for pattern, group in _GROUP_PATTERNS:
        if pattern.search(text):
            return group
    return None


def _snake(name: str) -> str:
    out = []
    for position, char in enumerate(name):
        if char.isupper() and position and not name[position - 1].isupper():
            out.append("_")
        out.append(char.lower())
    return "".join(out)


def _flags(features: Features) -> tuple[str, ...]:
    flags = []
    if features.tags & {"mock-internal", "mock-stdlib", "mock-other", "mock-script"}:
        flags.append("mock")
    if "mock-internal" in features.tags:
        flags.append("mock-internal")
    if "sql" in features.tags:
        flags.append("sql")
    if "sql-seed" in features.tags:
        flags.append("sql-seed")
    if "tapes" in features.tags:
        flags.append("tapes")
    if "subprocess" in features.tags or "spawn-kassiber" in features.tags:
        flags.append("subprocess")
    return tuple(flags)


def _kassiber_modules(features: Features) -> list[str]:
    modules = Counter()
    for call in features.kassiber_calls:
        parts = call.split("()")[0].split(".")
        # Drop the called symbol; keep up to kassiber.<pkg>.<module>.
        module = ".".join(parts[: min(len(parts) - 1, 3)]) or call
        modules[module] += 1
    return [name for name, _ in sorted(modules.items(), key=lambda pair: (-pair[1], pair[0]))]


def heuristic(path: Path, test: TestFunction, group: str) -> Verdict:
    features = test.features
    tags = features.tags
    flags = _flags(features)
    internal_mock = "mock" in flags
    seeds = [call for call in features.kassiber_calls if call not in CLI_ENTRY_CALLS]
    modules = _kassiber_modules(features)
    if "snapshot_matches_fixture" in test.name or path.parts[:2] == ("tests", "tax_oracle") or any(
        token in path.stem for token in ("golden", "parity", "vectors")
    ):
        return Verdict("oracle", group, "pins reference results or compares implementations", flags)
    if "spawn-kassiber" in tags:
        surface = "/".join(sorted(features.spawn_kinds))
        if internal_mock or seeds or "sql-seed" in tags:
            via = ("patches product/test-process dependencies" if internal_mock else
                   f"also calls {', '.join(modules[:2])}" if seeds else "seeds raw SQLite state")
            return Verdict(
                "subprocess-scenario",
                group,
                f"spawns `python -m kassiber` ({surface}) but {via}",
                flags,
            )
        return Verdict(
            "boundary-fixture",
            group,
            f"spawns `python -m kassiber` ({surface}) and checks its output only",
            flags,
        )
    entry = sorted(tags & {"in-process-cli", "in-process-daemon", "in-process-mcp"})
    if entry:
        surface = ", ".join(tag.removeprefix("in-process-") for tag in entry)
        extra = "; patches internals" if internal_mock else ""
        return Verdict(
            "subprocess-scenario",
            group,
            f"drives the {surface} entry in process{extra}",
            flags,
        )
    if modules:
        return Verdict("rust-test", group, f"calls {', '.join(modules[:2])} directly", flags)
    if "subprocess" in tags:
        return Verdict("rust-test", group, "runs helper processes without the product entry", flags)
    return Verdict("rust-test", group, "checks Python-level behavior without a product entry", flags)


# ---------------------------------------------------------------------------
# Inventory


def discover() -> tuple[Path, ...]:
    from scripts.python_test_shards import discover_test_files

    return discover_test_files(ROOT)


def _reviews():
    from tests.rust_port import reviews

    return reviews


@dataclass
class Entry:
    disposition: str
    group: str
    reason: str
    flags: tuple[str, ...]
    tests: int
    reviewed: bool
    replacement: str | None = None

    def as_json(self) -> dict:
        payload = {
            "disposition": self.disposition,
            "flags": list(self.flags),
            "group": self.group,
            "reason": self.reason,
            "reviewed": self.reviewed,
            "tests": self.tests,
        }
        if self.replacement:
            payload["replacement"] = self.replacement
        return payload


def _apply_review(verdict: Verdict, review: dict | None) -> tuple[Verdict, bool, str | None]:
    if not review:
        return verdict, False, None
    return (
        Verdict(
            review.get("disposition", verdict.disposition),
            review.get("group", verdict.group),
            review.get("reason", verdict.reason),
            tuple(review.get("flags", verdict.flags)),
        ),
        True,
        review.get("replacement"),
    )


def _review_for(reviews: dict, path: str, item: str, method: str | None) -> dict | None:
    module = reviews.get(path)
    if not module:
        return None
    merged: dict = {}
    for key in ("*", item, f"{item}::{method}" if method else None):
        if key and key in module:
            merged.update(module[key])
    return merged or None


def classify_module(path: Path, graph: FeatureGraph, reviews: dict) -> dict[str, Entry]:
    posix = path.as_posix()
    default_group = module_group(path)
    entries: dict[str, Entry] = {}
    module_reviews = reviews.get(posix, {})
    for item in collect_items(path, graph):
        verdicts: list[tuple[str, Verdict, bool, str | None]] = []
        for test in item.tests:
            group = default_group
            if posix in BROAD_MODULES:
                group = (
                    _group_from_tokens(test.name.removeprefix("test_"))
                    or _group_from_tokens(_snake(item.key))
                    or default_group
                )
            verdict = heuristic(path, test, group)
            method = test.name if item.kind == "class" else None
            reviewed_verdict, reviewed, replacement = _apply_review(
                verdict, _review_for(reviews, posix, item.key, method)
            )
            verdicts.append((test.name, reviewed_verdict, reviewed, replacement))
        if item.kind == "function":
            name, verdict, reviewed, replacement = verdicts[0]
            entries[item.key] = Entry(
                verdict.disposition, verdict.group, verdict.reason, verdict.flags, 1, reviewed, replacement
            )
            continue
        # The class entry carries the most common (disposition, group); methods
        # that differ get their own entries. Explicitly reviewed methods
        # always keep theirs.
        common = Counter((v.disposition, v.group) for _, v, _, _ in verdicts).most_common()
        top = sorted(common, key=lambda pair: (-pair[1], pair[0]))[0][0]
        class_review = _review_for(reviews, posix, item.key, None)
        if class_review:
            top = (
                class_review.get("disposition", top[0]),
                class_review.get("group", top[1]),
            )
        class_members = [v for _, v, _, _ in verdicts if (v.disposition, v.group) == top]
        flags = tuple(flag for flag in FLAGS if any(flag in v.flags for v in class_members))
        reasons = Counter(v.reason for v in class_members)
        reason = (
            class_review.get("reason")
            if class_review and "reason" in class_review
            else (sorted(reasons.items(), key=lambda pair: (-pair[1], pair[0]))[0][0] if reasons else "")
        )
        if not reason:
            reason = "every method differs from the class majority; see the method entries"
        split = []
        for name, verdict, reviewed, replacement in verdicts:
            method_key = f"{item.key}::{name}"
            explicit = method_key in module_reviews
            if (verdict.disposition, verdict.group) != top or explicit:
                split.append((name, verdict, reviewed, replacement))
        entries[item.key] = Entry(
            top[0],
            top[1],
            reason,
            flags,
            len(verdicts) - len(split),
            bool(class_review) or all(r for _, v, r, _ in verdicts if (v.disposition, v.group) == top),
            class_review.get("replacement") if class_review else None,
        )
        for name, verdict, reviewed, replacement in split:
            entries[f"{item.key}::{name}"] = Entry(
                verdict.disposition, verdict.group, verdict.reason, verdict.flags, 1, reviewed, replacement
            )
    return entries


def build_inventory() -> dict:
    reviews = _reviews().REVIEWS
    graph = FeatureGraph()
    modules = {}
    for path in discover():
        modules[path.as_posix()] = classify_module(path, graph, reviews)
    stale = sorted(set(reviews) - set(modules))
    if stale:
        raise SystemExit(f"reviews name modules that no longer exist: {', '.join(stale)}")
    for posix, module_reviews in reviews.items():
        names = set(modules[posix])
        items = {key.split("::")[0] for key in names}
        for key in module_reviews:
            if key == "*":
                continue
            if key not in names and key not in items:
                raise SystemExit(f"{posix}: review names unknown test {key}")
    return {"schema_version": SCHEMA_VERSION, "modules": modules}


def dump_inventory(inventory: dict) -> str:
    """One entry per line, keys sorted, so diffs stay per test."""
    lines = ["{", f'  "schema_version": {inventory["schema_version"]},', '  "modules": {']
    module_names = sorted(inventory["modules"])
    for module_position, module in enumerate(module_names):
        entries = inventory["modules"][module]
        lines.append(f"    {json.dumps(module)}: {{")
        keys = list(entries)
        for position, key in enumerate(keys):
            entry = entries[key]
            payload = entry.as_json() if isinstance(entry, Entry) else entry
            comma = "," if position < len(keys) - 1 else ""
            lines.append(
                f"      {json.dumps(key)}: {json.dumps(payload, sort_keys=True, ensure_ascii=False)}{comma}"
            )
        comma = "," if module_position < len(module_names) - 1 else ""
        lines.append(f"    }}{comma}")
    lines.extend(["  }", "}"])
    return "\n".join(lines) + "\n"


def load_inventory(path: Path = INVENTORY) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


# ---------------------------------------------------------------------------
# Coverage check


def test_names(path: Path) -> dict[str, set[str] | None]:
    """Item keys a module defines: classes map to their test method names."""
    graph = FeatureGraph()
    index = index_module(ROOT / path)
    assert index is not None
    graph.register(index)
    names: dict[str, set[str] | None] = {}
    for node in index.tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test"):
            names[node.name] = None
        elif isinstance(node, ast.ClassDef) and _is_test_class(graph, index, node):
            methods = {
                item.name
                for _, cls in graph.mro(index, node.name)
                for item in cls.body
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)) and item.name.startswith("test")
            }
            if methods:
                names[node.name] = methods
    return names


def check_inventory(inventory: dict, files: Iterable[Path]) -> list[str]:
    problems = []
    if inventory.get("schema_version") != SCHEMA_VERSION:
        problems.append(f"schema_version must be {SCHEMA_VERSION}")
    modules = inventory.get("modules", {})
    discovered = {path.as_posix(): path for path in files}
    for missing in sorted(set(discovered) - set(modules)):
        problems.append(f"{missing}: no disposition entry (run classify_tests update)")
    for stale in sorted(set(modules) - set(discovered)):
        problems.append(f"{stale}: entry for a module that no longer exists")
    for posix in sorted(set(discovered) & set(modules)):
        entries = modules[posix]
        names = test_names(discovered[posix])
        for item, methods in names.items():
            if item not in entries:
                problems.append(f"{posix}::{item}: no disposition entry (run classify_tests update)")
        for item, methods in names.items():
            actual = sum(e.get("tests", 0) for k, e in entries.items() if k.split("::")[0] == item)
            expected = len(methods) if methods is not None else 1
            if actual != expected:
                problems.append(f"{posix}::{item}: inventory counts {actual}, source defines {expected} tests")
        for key, entry in entries.items():
            item, _, method = key.partition("::")
            if item not in names:
                problems.append(f"{posix}::{key}: entry for a test that no longer exists")
            elif method and (names[item] is None or method not in names[item]):
                problems.append(f"{posix}::{key}: entry for a test that no longer exists")
            if entry.get("disposition") not in DISPOSITIONS:
                problems.append(f"{posix}::{key}: unknown disposition {entry.get('disposition')!r}")
            if entry.get("group") not in GROUPS:
                problems.append(f"{posix}::{key}: unknown group {entry.get('group')!r}")
            if not str(entry.get("reason") or "").strip():
                problems.append(f"{posix}::{key}: missing reason")
            if set(entry.get("flags", ())) - set(FLAGS):
                problems.append(f"{posix}::{key}: unknown flags {entry.get('flags')}")
            if entry.get("disposition") == "python-only" and not entry.get("replacement"):
                problems.append(f"{posix}::{key}: python-only entries must name a replacement")
    return problems


# ---------------------------------------------------------------------------
# Summary


def counts(inventory: dict) -> dict[tuple[str, str], int]:
    table: dict[tuple[str, str], int] = defaultdict(int)
    for entries in inventory["modules"].values():
        for entry in entries.values():
            table[(entry["group"], entry["disposition"])] += entry["tests"]
    return table


def summary_markdown(inventory: dict) -> str:
    table = counts(inventory)
    columns = list(DISPOSITIONS)
    header = "| Group | " + " | ".join(columns) + " | Total |"
    rule = "| --- | " + " | ".join("---:" for _ in columns) + " | ---: |"
    lines = [header, rule]
    totals = Counter()
    for group, label in GROUPS.items():
        row = [table.get((group, disposition), 0) for disposition in columns]
        if not any(row):
            continue
        for disposition, value in zip(columns, row):
            totals[disposition] += value
        lines.append(f"| {label} | " + " | ".join(str(v) for v in row) + f" | {sum(row)} |")
    lines.append(
        "| **Total** | "
        + " | ".join(f"**{totals[d]}**" for d in columns)
        + f" | **{sum(totals.values())}** |"
    )
    return "\n".join(lines) + "\n"


def flag_counts(inventory: dict) -> dict[str, int]:
    result = Counter()
    for entries in inventory["modules"].values():
        for entry in entries.values():
            for flag in entry["flags"]:
                result[flag] += entry["tests"]
    return dict(result)


def reviewed_counts(inventory: dict) -> tuple[int, int, int]:
    modules_reviewed = sum(
        1 for entries in inventory["modules"].values() if all(e["reviewed"] for e in entries.values())
    )
    tests_reviewed = sum(
        e["tests"] for entries in inventory["modules"].values() for e in entries.values() if e["reviewed"]
    )
    tests_total = sum(e["tests"] for entries in inventory["modules"].values() for e in entries.values())
    return modules_reviewed, tests_reviewed, tests_total


# ---------------------------------------------------------------------------
# CLI


def _explain(path: Path) -> None:
    """Print each test's heuristic verdict and evidence, before reviews."""
    graph = FeatureGraph()
    default_group = module_group(path)
    for item in collect_items(path, graph):
        for test in item.tests:
            group = default_group
            if path.as_posix() in BROAD_MODULES:
                group = (
                    _group_from_tokens(test.name.removeprefix("test_"))
                    or _group_from_tokens(_snake(item.key))
                    or default_group
                )
            verdict = heuristic(path, test, group)
            tags = sorted(test.features.tags)
            modules = _kassiber_modules(test.features)[:4]
            patches = sorted(test.features.patch_targets)[:4]
            print(
                f"{item.key}::{test.name}: {verdict.disposition} [{verdict.group}] | "
                f"{verdict.reason} | tags={tags} modules={modules} patches={patches}"
            )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("update", help="Rewrite the inventory from the sources and reviews")
    sub.add_parser("check", help="Fail when a test lacks an entry or an entry is stale")
    sub.add_parser("summary", help="Print the disposition x group table")
    explain = sub.add_parser("explain", help="Print the heuristic evidence for one module")
    explain.add_argument("path", type=Path)
    args = parser.parse_args(argv)
    if args.command == "update":
        INVENTORY.write_text(dump_inventory(build_inventory()), encoding="utf-8")
        reference = ROOT / "docs/reference/rust-port.md"
        if reference.exists():
            source = reference.read_text(encoding="utf-8")
            start, end = "<!-- disposition-counts:start -->\n", "<!-- disposition-counts:end -->"
            before, body = source.split(start, 1)
            _, after = body.split(end, 1)
            reference.write_text(before + start + summary_markdown(load_inventory()) + end + after, encoding="utf-8")
        print(f"wrote {INVENTORY.relative_to(ROOT)}")
        return 0
    if args.command == "check":
        problems = check_inventory(load_inventory(), discover())
        for problem in problems:
            print(problem)
        return 1 if problems else 0
    if args.command == "summary":
        inventory = load_inventory()
        print(summary_markdown(inventory), end="")
        modules_reviewed, tests_reviewed, tests_total = reviewed_counts(inventory)
        print(
            f"\nflags (tests): {json.dumps(flag_counts(inventory), sort_keys=True)}\n"
            f"reviewed: {modules_reviewed} modules, {tests_reviewed} of {tests_total} tests"
        )
        return 0
    _explain(args.path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
