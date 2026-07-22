#!/usr/bin/env python3
"""Inventory Tauri route daemon kinds and presentation/action contracts."""

from __future__ import annotations

import json
import re
from pathlib import Path


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = PACKAGE_ROOT.parent
UI_ROOT = REPO_ROOT / "ui-tauri" / "src"
ROUTE_TREE = UI_ROOT / "routeTree.tsx"
MANIFEST = PACKAGE_ROOT / "Generated" / "DaemonKinds.generated.json"
APP_SHELL = UI_ROOT / "components" / "kb" / "AppShell.tsx"

# A few routes consume daemon data through an application-wide bridge rather
# than importing the transport at the route boundary. Keep those source-mapped
# here so the parity inventory represents what the screen actually displays.
# `Logs` reads the RAM-only app log buffer, which is populated by
# `lib/daemonLogBridge.ts` polling `ui.logs.snapshot`.
INDIRECT_ROUTE_KINDS: dict[str, set[str]] = {
    "Logs": {"ui.logs.snapshot"},
}

# UI presentation and stateful behavior that cannot be proven by a set of
# daemon-kind literals alone. These patterns are evaluated across the route's
# reachable source graph and become named contracts in the inventory, so the
# native checker cannot accept an unused view-model method.
SCREEN_CONTRACT_PATTERNS: dict[str, dict[str, str]] = {
    "Transactions": {
        "transactions.new_transaction_draft": (
            r"(?s)(?=.*<NewTransactionDialog\b)"
            r"(?=.*onDraftChange=\{setNewTransactionDraft\})"
            r"(?=.*onSaveDraft=)"
        ),
    },
    "Activity": {
        "activity.load_more": r"\bfetchNextPage\b",
    },
    "SwapMatching": {
        "swaps.edit_pair": r'["\']ui\.transfers\.update["\']',
    },
    "Assistant": {
        "assistant.send_prompt": r"onSubmit=\{sendPrompt\}",
        "assistant.queue_prompt": r"setQueuedPrompts\(\(current\)\s*=>\s*\[\.\.\.current,\s*trimmed\]\)",
        "assistant.incognito": r"setIncognito\(",
        "assistant.thinking_effort": r"setThinkingEffort",
        "assistant.fork_seed_history": r"seedHistoryPendingRef\.current\s*=\s*true",
        "assistant.cancel_stream": r'["\']ai\.chat\.cancel["\']',
        "assistant.tool_consent": r'["\']ai\.tool_call\.consent["\']',
    },
}

# The same transaction-detail workstation is reachable from five Tauri
# routes. Keep its graph plus Privacy Mirror presentation contract attached to
# every route that inventories both backing daemon kinds; one unrelated native
# callsite must not satisfy all five surfaces accidentally.
for component in (
    "Overview", "Transactions", "SourceFunds", "Quarantine", "ConnectionDetail"
):
    SCREEN_CONTRACT_PATTERNS.setdefault(component, {})[
        "transaction_detail.graph_privacy"
    ] = (
        r"(?s)(?=.*<TransactionGraphPanel\b)"
        r"(?=.*<TransactionPrivacyMirrorPanel\b)"
    )

INDIRECT_ROUTE_SOURCES: dict[str, tuple[Path, ...]] = {
    # The provider is mounted above the lazy Assistant route, so the route
    # consumes its context without importing the implementation directly.
    "Assistant": (UI_ROOT / "components" / "ai" / "AssistantSessionProvider.tsx",),
}


def resolve_module(current: Path, specifier: str) -> Path | None:
    if specifier.startswith("@/"):
        base = UI_ROOT / specifier[2:]
    elif specifier.startswith("."):
        base = current.parent / specifier
    else:
        return None
    if base.suffix and base.suffix not in {".ts", ".tsx"}:
        return None
    candidates = [base, base.with_suffix(".ts"), base.with_suffix(".tsx"), base / "index.ts", base / "index.tsx"]
    for candidate in candidates:
        if candidate.is_file() and candidate.suffix in {".ts", ".tsx"}:
            return candidate.resolve()
    return None


def reachable_inventory(entry: Path, supported: set[str]) -> tuple[list[str], str]:
    stack = [entry.resolve()]
    visited: set[Path] = set()
    kinds: set[str] = set()
    sources: list[str] = []
    dotted_supported = supported - {"status"}
    literal_pattern = re.compile(
        r'["\'`](' + "|".join(re.escape(kind) for kind in sorted(dotted_supported, key=len, reverse=True)) + r')["\'`]'
    )
    while stack:
        path = stack.pop()
        if path in visited or not path.is_file():
            continue
        relative = path.relative_to(UI_ROOT)
        if "mocks" in relative.parts or path.name.endswith((".test.ts", ".test.tsx")):
            continue
        # Transport/query plumbing carries global invalidation catalogs rather
        # than screen behavior; routes may import it, but it is not their kind inventory.
        if relative.as_posix() in {"daemon/client.ts", "daemon/transport.ts"}:
            continue
        visited.add(path)
        text = path.read_text(encoding="utf-8")
        sources.append(text)
        kinds.update(literal_pattern.findall(text))
        # `status` is the one supported bare kind. Match daemon call shapes,
        # not unrelated HTML roles or object fields that also use that word.
        if "status" in supported and re.search(
            r'(?:useDaemon(?:<[^>]*>)?\s*\(\s*|invoke(?:<[^>]*>)?\s*\(\s*|kind\s*:\s*)["\']status["\']',
            text,
        ):
            kinds.add("status")
        # The stream reducer is a leaf protocol adapter. Its three supported
        # literals are real Assistant actions (send/cancel/consent), but its
        # imports are transport plumbing rather than additional screen actions.
        if relative.as_posix() == "daemon/stream.ts":
            continue
        imports = re.findall(r"(?:from\s+|import\s*\()[\"']([^\"']+)[\"']", text)
        for specifier in imports:
            resolved = resolve_module(path, specifier)
            if resolved is not None:
                stack.append(resolved)
    return sorted(kinds), "\n".join(sources)


def reachable_kinds(entry: Path, supported: set[str]) -> list[str]:
    return reachable_inventory(entry, supported)[0]


def main() -> None:
    supported = set(json.loads(MANIFEST.read_text(encoding="utf-8")))
    text = ROUTE_TREE.read_text(encoding="utf-8")
    components = {
        name: module
        for name, module in re.findall(
            r"const\s+(\w+)\s*=\s*lazyRouteComponent\([\s\S]*?import\(\"([^\"]+)\"\)",
            text,
        )
    }
    rows: list[dict[str, object]] = []
    for block in re.findall(r"const\s+\w+Route\s*=\s*createRoute\(\{([\s\S]*?)\n\}\);", text):
        path_match = re.search(r'path:\s*"([^"]+)"', block)
        component_match = re.search(r"component:\s*(\w+)", block)
        if not path_match or not component_match:
            continue
        component = component_match.group(1)
        module = components.get(component)
        if module is None:
            continue
        entry = resolve_module(ROUTE_TREE, module)
        if entry is None:
            continue
        reachable, source_graph = reachable_inventory(entry, supported)
        kinds = set(reachable)
        for indirect in INDIRECT_ROUTE_SOURCES.get(component, ()):
            indirect_kinds, indirect_sources = reachable_inventory(indirect, supported)
            kinds.update(indirect_kinds)
            source_graph += "\n" + indirect_sources
        kinds.update(INDIRECT_ROUTE_KINDS.get(component, set()) & supported)
        actions = sorted(
            action
            for action, pattern in SCREEN_CONTRACT_PATTERNS.get(component, {}).items()
            if re.search(pattern, source_graph)
        )
        rows.append(
            {
                "route": path_match.group(1),
                "component": component,
                "source": str(entry.relative_to(REPO_ROOT)),
                "kinds": sorted(kinds),
                "actions": actions,
            }
        )
    # AppShell owns cross-route state such as review badges, the project
    # catalog, lock/unlock, global refresh, and assistant/session plumbing. It
    # is deliberately inventoried as a synthetic route because limiting the
    # audit to route components would miss those user-facing desktop features.
    app_shell_kinds, app_shell_sources = reachable_inventory(APP_SHELL, supported)
    rows.append(
        {
            "route": "@app-shell",
            "component": "AppShell",
            "source": str(APP_SHELL.relative_to(REPO_ROOT)),
            "kinds": app_shell_kinds,
            "actions": sorted(
                action
                for action, pattern in SCREEN_CONTRACT_PATTERNS.get("AppShell", {}).items()
                if re.search(pattern, app_shell_sources)
            ),
        }
    )
    print(json.dumps(rows, indent=2))


if __name__ == "__main__":
    main()
