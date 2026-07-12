#!/usr/bin/env python3
"""Audit route-scoped native feature reachability against Tauri.

This is intentionally stricter than a union-of-all-call-sites check.  Every
Tauri route has an explicit native screen/surface contract and an allowlist of
Swift declarations that implement that route's features.  A daemon kind used
on one native screen therefore cannot accidentally satisfy an unrelated Tauri
route merely because both kinds occur somewhere in ``Sources``.
"""

from __future__ import annotations

import json
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REPO = ROOT.parent
POLICY = ROOT / "Sources" / "KassiberDaemonKit" / "DesktopDaemonAccessPolicy.swift"


@dataclass(frozen=True)
class NativeRouteContract:
    """Audited bridge from one Tauri route to native presentation owners."""

    route: str
    screen_case: str | None
    host_view: str
    surface_view: str
    primary_models: tuple[str, ...]
    owners: tuple[tuple[str, str], ...]
    allows_tool_result_presentation: bool = False


def owner(module: str, symbol: str) -> tuple[str, str]:
    return (f"Sources/{module}", symbol)


# These declaration scopes are the executable route map.  Shared native
# workstations are listed only for the Tauri routes that expose the same
# feature.  Moving a call to an unrelated view model no longer keeps the audit
# green: the corresponding route contract must be reviewed deliberately.
NATIVE_ROUTE_CONTRACTS: dict[str, NativeRouteContract] = {
    "Welcome": NativeRouteContract(
        route="/", screen_case=None, host_view="FullOnboardingScreen",
        surface_view="FullOnboardingScreen", primary_models=("OnboardingParityViewModel",),
        owners=(
            owner("KassiberApp/SettingsConnectionsScreens.swift", "FullOnboardingScreen"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "OnboardingParityViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "BackendSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "SecuritySettingsViewModel"),
            owner("KassiberViewModels/BooksViewModel.swift", "BooksViewModel"),
        ),
    ),
    "Overview": NativeRouteContract(
        route="/overview", screen_case="dashboard", host_view="DashboardScreen",
        surface_view="DashboardScreen", primary_models=("DashboardViewModel",),
        owners=(
            owner("KassiberApp/CoreScreens.swift", "DashboardScreen"),
            owner("KassiberViewModels/DashboardViewModel.swift", "DashboardViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionDetailViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "BackendSettingsViewModel"),
            owner("KassiberViewModels/ConnectionsParityViewModels.swift", "ConnectionsParityViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsImportViewModel"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "LedgerImportViewModel"),
        ),
    ),
    "Transactions": NativeRouteContract(
        route="/transactions", screen_case="transactions", host_view="TransactionsScreen",
        surface_view="TransactionsScreen", primary_models=("TransactionsViewModel",),
        owners=(
            owner("KassiberApp/CoreScreens.swift", "TransactionsScreen"),
            owner("KassiberViewModels/TransactionsViewModel.swift", "TransactionsViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionDetailViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/WalletsViewModel.swift", "WalletsViewModel"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
            owner("KassiberViewModels/ReviewViewModels.swift", "SwapsViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
        ),
    ),
    "Activity": NativeRouteContract(
        route="/activity", screen_case="activity", host_view="ActivityScreen",
        surface_view="ActivityScreen", primary_models=("ActivityViewModel",),
        owners=(
            owner("KassiberApp/AnalysisScreens.swift", "ActivityScreen"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "ActivityViewModel"),
        ),
    ),
    "Reports": NativeRouteContract(
        route="/reports", screen_case="reports", host_view="ReportsImportsReportsScreen",
        surface_view="ReportsImportsReportsScreen", primary_models=("ReportsImportsReportsViewModel",),
        owners=(
            owner("KassiberApp/ReportsImportsParityScreens.swift", "ReportsImportsReportsScreen"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsReportsViewModel"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsExportKind"),
            owner("KassiberViewModels/ReportsViewModel.swift", "ReportKind"),
            owner("KassiberViewModels/DashboardViewModel.swift", "DashboardViewModel"),
        ),
    ),
    "PrivacyMirror": NativeRouteContract(
        route="/privacy-mirror", screen_case="privacyMirror", host_view="PrivacyMirrorScreen",
        surface_view="PrivacyMirrorScreen", primary_models=("PrivacyMirrorViewModel",),
        owners=(
            owner("KassiberApp/AnalysisScreens.swift", "PrivacyMirrorScreen"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
        ),
    ),
    "ExitTax": NativeRouteContract(
        route="/exit-tax", screen_case="exitTax", host_view="ExitTaxScreen",
        surface_view="ExitTaxScreen", primary_models=("ExitTaxViewModel",),
        owners=(
            owner("KassiberApp/ExportScreens.swift", "ExitTaxScreen"),
            owner("KassiberViewModels/ExportViewModels.swift", "ExitTaxViewModel"),
        ),
    ),
    "SourceFunds": NativeRouteContract(
        route="/source-of-funds", screen_case="sourceFunds", host_view="ReportsImportsSourceFundsScreen",
        surface_view="ReportsImportsSourceFundsScreen", primary_models=("ReportsImportsSourceFundsViewModel",),
        owners=(
            owner("KassiberApp/ReportsImportsParityScreens.swift", "ReportsImportsSourceFundsScreen"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsSourceFundsViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionDetailViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
        ),
    ),
    "Journals": NativeRouteContract(
        route="/journals", screen_case="journals", host_view="JournalsScreen",
        surface_view="JournalsScreen", primary_models=("JournalsViewModel",),
        owners=(
            owner("KassiberApp/ReviewScreens.swift", "JournalsScreen"),
            owner("KassiberViewModels/ReviewViewModels.swift", "JournalsViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
        ),
    ),
    "SwapMatching": NativeRouteContract(
        route="/swaps", screen_case="swaps", host_view="SwapsScreen",
        surface_view="SwapsScreen", primary_models=("SwapsViewModel",),
        owners=(
            owner("KassiberApp/ReviewScreens.swift", "SwapsScreen"),
            owner("KassiberViewModels/ReviewViewModels.swift", "SwapsViewModel"),
        ),
    ),
    "Quarantine": NativeRouteContract(
        route="/quarantine", screen_case="quarantine", host_view="QuarantineScreen",
        surface_view="QuarantineScreen", primary_models=("QuarantineViewModel",),
        owners=(
            owner("KassiberApp/ReviewScreens.swift", "QuarantineScreen"),
            owner("KassiberViewModels/ReviewViewModels.swift", "QuarantineViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionDetailViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionResolverViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/DashboardViewModel.swift", "DashboardViewModel"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
        ),
    ),
    "Reconcile": NativeRouteContract(
        route="/reconcile", screen_case="reconcile", host_view="ReconcileScreen",
        surface_view="ReconcileScreen", primary_models=("ReconcileViewModel",),
        owners=(
            owner("KassiberApp/ReviewScreens.swift", "ReconcileScreen"),
            owner("KassiberViewModels/ReviewViewModels.swift", "ReconcileViewModel"),
        ),
    ),
    "Egress": NativeRouteContract(
        route="/egress", screen_case="egress", host_view="EgressScreen",
        surface_view="EgressScreen", primary_models=("EgressViewModel",),
        owners=(
            owner("KassiberApp/AnalysisScreens.swift", "EgressScreen"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "EgressViewModel"),
        ),
    ),
    "Logs": NativeRouteContract(
        route="/logs", screen_case="logs", host_view="LogsScreen",
        surface_view="LogsScreen", primary_models=("LogsViewModel",),
        owners=(
            owner("KassiberApp/LogsScreen.swift", "LogsScreen"),
            owner("KassiberViewModels/LogsViewModel.swift", "LogsViewModel"),
            owner("KassiberViewModels/AIChatViewModel.swift", "AIChatViewModel"),
        ),
    ),
    "Books": NativeRouteContract(
        route="/books", screen_case="books", host_view="BooksScreen",
        surface_view="BooksScreen", primary_models=("BooksViewModel",),
        owners=(
            owner("KassiberApp/MutationScreens.swift", "BooksScreen"),
            owner("KassiberViewModels/BooksViewModel.swift", "BooksViewModel"),
        ),
    ),
    "BirdsEye": NativeRouteContract(
        route="/books/$workspaceId/birds-eye", screen_case="birdsEye", host_view="BirdsEyeScreen",
        surface_view="BirdsEyeScreen", primary_models=("BirdsEyeViewModel",),
        owners=(
            owner("KassiberApp/AnalysisScreens.swift", "BirdsEyeScreen"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "BirdsEyeViewModel"),
        ),
    ),
    "Connections": NativeRouteContract(
        route="/connections", screen_case="connections", host_view="FullConnectionsScreen",
        surface_view="FullConnectionsScreen", primary_models=("ConnectionsParityViewModel",),
        owners=(
            owner("KassiberApp/SettingsConnectionsScreens.swift", "FullConnectionsScreen"),
            owner("KassiberViewModels/ConnectionsParityViewModels.swift", "ConnectionsParityViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "BackendSettingsViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsImportViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "LedgerImportViewModel"),
            owner("KassiberViewModels/DashboardViewModel.swift", "DashboardViewModel"),
            owner("KassiberViewModels/WalletsViewModel.swift", "WalletsViewModel"),
        ),
    ),
    "ConnectionDetail": NativeRouteContract(
        route="/connections/$connectionId", screen_case="connections", host_view="FullConnectionsScreen",
        surface_view="NativeConnectionDetailView", primary_models=("ConnectionDetailParityViewModel",),
        owners=(
            owner("KassiberApp/SettingsConnectionsScreens.swift", "NativeConnectionDetailView"),
            owner("KassiberViewModels/ConnectionsParityViewModels.swift", "ConnectionDetailParityViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionDetailViewModel"),
            owner("KassiberViewModels/TransactionDetailViewModel.swift", "TransactionResolverViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/AnalysisViewModels.swift", "PrivacyMirrorViewModel"),
        ),
    ),
    "Imports": NativeRouteContract(
        route="/imports", screen_case="imports", host_view="ReportsImportsImportScreen",
        surface_view="ReportsImportsImportScreen", primary_models=("ReportsImportsImportViewModel",),
        owners=(
            owner("KassiberApp/ReportsImportsParityScreens.swift", "ReportsImportsImportScreen"),
            owner("KassiberViewModels/ReportsImportsParityViewModels.swift", "ReportsImportsImportViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "BackendSettingsViewModel"),
            owner("KassiberViewModels/ConnectionsParityViewModels.swift", "ConnectionsParityViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "LedgerImportViewModel"),
        ),
    ),
    "Settings": NativeRouteContract(
        route="/settings", screen_case="settings", host_view="FullLayeredSettingsScreen",
        surface_view="FullLayeredSettingsScreen", primary_models=("BackendSettingsViewModel",),
        owners=(
            owner("KassiberApp/SettingsConnectionsScreens.swift", "FullLayeredSettingsScreen"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "BackendSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "AIProviderSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "MaintenanceSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "SecuritySettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "RatesSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "ReplicationSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "DestructiveSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "OnboardingParityViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "ChatHistorySettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "PrivacyHygieneSettingsViewModel"),
            owner("KassiberViewModels/AIChatViewModel.swift", "AIChatViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
        ),
    ),
    "Assistant": NativeRouteContract(
        route="/assistant", screen_case="assistant", host_view="AIChatScreen",
        surface_view="AIChatScreen", primary_models=("AIChatViewModel",),
        owners=(
            owner("KassiberApp/AIChatScreen.swift", "AIChatScreen"),
            owner("KassiberViewModels/AIChatViewModel.swift", "AIChatViewModel"),
        ),
        allows_tool_result_presentation=True,
    ),
    "AppShell": NativeRouteContract(
        route="@app-shell", screen_case=None, host_view="AppShellView",
        surface_view="AppShellView", primary_models=("AppShellViewModel",),
        owners=(
            owner("KassiberApp/AppShellView.swift", "AppShellView"),
            owner("KassiberViewModels/AppShellViewModel.swift", "AppShellViewModel"),
            owner("KassiberViewModels/AIChatViewModel.swift", "AIChatViewModel"),
            owner("KassiberViewModels/HostChromeViewModels.swift", "DaemonNativeEndpointHealthChecker"),
            owner("KassiberViewModels/ConnectionSettingsViewModel.swift", "ConnectionSettingsViewModel"),
            owner("KassiberViewModels/MutationViewModels.swift", "BookRefreshCoordinator"),
            owner("KassiberViewModels/LogsViewModel.swift", "LogsViewModel"),
            owner("KassiberViewModels/DashboardViewModel.swift", "DashboardViewModel"),
            owner("KassiberViewModels/BooksViewModel.swift", "BooksViewModel"),
            owner("KassiberViewModels/GlobalSearchViewModel.swift", "GlobalSearchViewModel"),
            owner("KassiberViewModels/ReportsViewModel.swift", "ReportKind"),
            owner("KassiberViewModels/ReviewViewModels.swift", "QuarantineViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "MaintenanceSettingsViewModel"),
            owner("KassiberViewModels/SettingsParityViewModels.swift", "RatesSettingsViewModel"),
            owner("KassiberViewModels/WalletConnectionSetupViewModel.swift", "WalletConnectionSetupViewModel"),
            owner("KassiberViewModels/TransactionsViewModel.swift", "TransactionsViewModel"),
        ),
        allows_tool_result_presentation=True,
    ),
}

# Screen-level contracts deliberately point at both SwiftUI presentation/action
# anchors and view-model operations. This closes the gap where an unused method
# can mention a daemon kind and make aggregate kind coverage look complete.
NATIVE_SCREEN_CONTRACT_PATTERNS: dict[tuple[str, str], tuple[tuple[str, str], ...]] = {
    ("Transactions", "transactions.new_transaction_draft"): (
        (
            "Sources/KassiberApp/CoreScreens.swift",
            r'Label\(localized\("transactionDraft\.trigger"\),\s*systemImage:\s*"plus"\)',
        ),
        (
            "Sources/KassiberApp/CoreScreens.swift",
            r"\.sheet\(isPresented:\s*\$showingNewDraft\)\s*\{\s*"
            r"NewTransactionDraftSheet\(draft:\s*\$newDraft,\s*wallets:\s*draftWallets\)",
        ),
        (
            "Sources/KassiberApp/NewTransactionDraftSheet.swift",
            r'Button\(draftLocalized\("transactionDraft\.save"\)\)\s*\{\s*saved\(\)\s*dismiss\(\)',
        ),
        (
            "Sources/KassiberViewModels/NewTransactionDraft.swift",
            r"mutating\s+func\s+selectNetwork\(.*?mutating\s+func\s+selectFlow\(.*?"
            r"mutating\s+func\s+updateAmount\(",
        ),
    ),
    ("Activity", "activity.load_more"): (
        ("Sources/KassiberApp/AnalysisScreens.swift", r"model\.load\(reset:\s*false\)"),
    ),
    ("SwapMatching", "swaps.edit_pair"): (
        ("Sources/KassiberApp/ReviewScreens.swift", r"await\s+model\.update\("),
    ),
    ("Assistant", "assistant.send_prompt"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"await\s+model\.send\(\)"),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r"streamSession\(\.aiChat"),
    ),
    ("Assistant", "assistant.queue_prompt"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r'chat\.queue'),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r"queuedPrompts\.append\(prompt\)"),
    ),
    ("Assistant", "assistant.incognito"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"\$model\.incognito"),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r'"persist":\s*incognito\s*&&\s*sessionID\s*==\s*nil'),
    ),
    ("Assistant", "assistant.thinking_effort"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"\$model\.thinkingEffort"),
        ("Sources/KassiberApp/AIChatScreen.swift", r"message\.thinkingSegments"),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r'"reasoning_effort"'),
    ),
    ("Assistant", "assistant.fork_seed_history"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"model\.branch\(from:"),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r'args\["seed_history"\]'),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r"seedHistoryPending\s*=\s*true"),
    ),
    ("Assistant", "assistant.cancel_stream"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"await\s+model\.stop\(\)"),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r"\.aiChatCancel"),
    ),
    ("Assistant", "assistant.tool_consent"): (
        ("Sources/KassiberApp/AIChatScreen.swift", r"await\s+model\.decideConsent\("),
        ("Sources/KassiberViewModels/AIChatViewModel.swift", r"\.aiToolCallConsent"),
    ),
}

TRANSACTION_GRAPH_PRIVACY_PATTERNS: tuple[tuple[str, str], ...] = (
    (
        "Sources/KassiberApp/CoreScreens.swift",
        r"TransactionFlowPresentation\(\s*snapshot:\s*model\.graphSnapshot,\s*"
        r"privacy:\s*model\.privacyContext,\s*"
        r"privacyIsLoading:\s*model\.privacyIsLoading",
    ),
    (
        "Sources/KassiberViewModels/TransactionDetailViewModel.swift",
        r"daemon\.invoke\(\.uiTransactionsGraph\b",
    ),
    (
        "Sources/KassiberViewModels/TransactionDetailViewModel.swift",
        r"daemon\.invoke\(\.uiReportsPrivacyMirror\b",
    ),
    (
        "Sources/KassiberApp/TransactionFlowViews.swift",
        r"if\s+snapshot\.hasFlowEvidence\s*\{.*?flowDiagram.*?privacyPanel",
    ),
    (
        "Sources/KassiberApp/TransactionFlowViews.swift",
        r"shortReference\(node\.reference\).*?textSelection\(\.enabled\).*?"
        r"kassiberSensitive\(\)",
    ),
)
for transaction_detail_component in (
    "Overview", "Transactions", "SourceFunds", "Quarantine", "ConnectionDetail"
):
    NATIVE_SCREEN_CONTRACT_PATTERNS[
        (transaction_detail_component, "transaction_detail.graph_privacy")
    ] = TRANSACTION_GRAPH_PRIVACY_PATTERNS

INVENTORY_KIND_EXPECTATIONS: dict[str, set[str]] = {
    # `status` has no dotted namespace and used to be silently omitted by the
    # literal scanner. Stream controls live in the leaf stream adapter rather
    # than the route component and need the same explicit regression lock.
    "Welcome": {"status"},
    "ConnectionDetail": {"status"},
    "Settings": {"status", "ai.chat.cancel", "ai.tool_call.consent"},
    "Assistant": {"ai.chat", "ai.chat.cancel", "ai.tool_call.consent"},
    "AppShell": {"ai.chat", "ai.chat.cancel", "ai.tool_call.consent"},
}


DECLARATION_PATTERN = re.compile(
    r"(?m)^(?:public\s+|private\s+|internal\s+|fileprivate\s+)?"
    r"(?:final\s+)?(?:struct|class|enum|actor|protocol)\s+(?P<name>\w+)\b[^\n{]*\{"
)


def swift_declaration(source: str, symbol: str, location: str) -> str:
    """Return one top-level Swift declaration, including nested closures.

    Swift interpolation and collection literals contain balanced braces, so a
    brace counter is sufficient for the checked-in sources.  Ambiguous or
    missing declarations fail rather than widening the scan to an entire file.
    """

    matches = [
        match for match in DECLARATION_PATTERN.finditer(source)
        if match.group("name") == symbol
    ]
    if len(matches) != 1:
        raise SystemExit(
            f"Native route owner must resolve exactly once: {location}::{symbol} "
            f"(found {len(matches)})"
        )
    match = matches[0]
    depth = 0
    for index in range(match.end() - 1, len(source)):
        character = source[index]
        if character == "{":
            depth += 1
        elif character == "}":
            depth -= 1
            if depth == 0:
                return source[match.start() : index + 1]
    raise SystemExit(f"Unterminated native route owner: {location}::{symbol}")


def kinds_in_declarations(source: str, raw_by_case: dict[str, str]) -> set[str]:
    cases = set(re.findall(r"\.(\w+)\b", source))
    found = {raw_by_case[case] for case in cases & raw_by_case.keys()}
    # `status` is deliberately a bare generated case and collides with many
    # ordinary model properties.  Only a daemon operation proves this kind.
    if "status" in found and not re.search(
        r"\.(?:invoke|stream|streamSession)\s*\(\s*\.status\b", source
    ):
        found.remove("status")
    return found


def app_screen_host_map() -> tuple[set[str], dict[str, str]]:
    model_source = (
        ROOT / "Sources" / "KassiberViewModels" / "AppShellViewModel.swift"
    ).read_text(encoding="utf-8")
    enum_match = re.search(
        r"public\s+enum\s+AppScreen\b.*?\{(.*?)\n\}", model_source, re.DOTALL
    )
    if not enum_match:
        raise SystemExit("Could not read native AppScreen cases")
    cases = set(re.findall(r"(?m)^\s*case\s+(\w+)\s*$", enum_match.group(1)))

    shell_source = (ROOT / "Sources" / "KassiberApp" / "AppShellView.swift").read_text(
        encoding="utf-8"
    )
    screen_host = swift_declaration(shell_source, "ScreenHost", "AppShellView.swift")
    mappings = dict(
        re.findall(r"case\s+\.(\w+)\s*:\s*\n\s*(\w+)\s*\(", screen_host)
    )
    return cases, mappings


def audit_native_route_contracts(
    inventory: list[dict[str, object]],
    raw_by_case: dict[str, str],
    renderer_allowed: set[str],
    presented: set[str],
) -> tuple[int, int]:
    inventory_components = {str(route["component"]) for route in inventory}
    contract_components = set(NATIVE_ROUTE_CONTRACTS)
    fail(
        "Tauri routes missing an explicit native route contract",
        inventory_components - contract_components,
    )
    fail(
        "Stale native route contracts no longer present in Tauri",
        contract_components - inventory_components,
    )

    app_cases, screen_hosts = app_screen_host_map()
    source_cache: dict[str, str] = {}
    declaration_cache: dict[tuple[str, str], str] = {}
    route_failures: list[str] = []
    route_kind_memberships = 0

    for route_row in inventory:
        component = str(route_row["component"])
        route = str(route_row["route"])
        required = {str(kind) for kind in route_row["kinds"]}
        contract = NATIVE_ROUTE_CONTRACTS[component]
        if route != contract.route:
            route_failures.append(
                f"{component}: Tauri route changed from {contract.route} to {route}"
            )

        if contract.screen_case is not None:
            if contract.screen_case not in app_cases:
                route_failures.append(
                    f"{component}: native AppScreen.{contract.screen_case} is missing"
                )
            mapped_view = screen_hosts.get(contract.screen_case)
            if mapped_view != contract.host_view:
                route_failures.append(
                    f"{component}: AppScreen.{contract.screen_case} hosts "
                    f"{mapped_view or '<nothing>'}, expected {contract.host_view}"
                )

        scoped_source_parts: list[str] = []
        surface_source: str | None = None
        owner_symbols: set[str] = set()
        for relative, symbol in contract.owners:
            key = (relative, symbol)
            source = source_cache.setdefault(
                relative, (ROOT / relative).read_text(encoding="utf-8")
            )
            declaration = declaration_cache.setdefault(
                key, swift_declaration(source, symbol, relative)
            )
            scoped_source_parts.append(declaration)
            owner_symbols.add(symbol)
            if symbol == contract.surface_view:
                surface_source = declaration

        if contract.surface_view not in owner_symbols or surface_source is None:
            route_failures.append(
                f"{component}: surface {contract.surface_view} is not an audited owner"
            )
        else:
            for model in contract.primary_models:
                if not re.search(rf"\b{re.escape(model)}\b", surface_source):
                    route_failures.append(
                        f"{component}: {contract.surface_view} is not wired to {model}"
                    )

        scoped_kinds = kinds_in_declarations("\n".join(scoped_source_parts), raw_by_case)
        route_presented = presented if contract.allows_tool_result_presentation else set()
        missing = required - scoped_kinds - route_presented
        if missing:
            route_failures.append(
                f"{component} ({route}): route-scoped native owners miss "
                + ", ".join(sorted(missing))
            )

        # Generic tool results may only satisfy a kind actually declared in
        # the presentation policy; all other route calls need a scoped owner.
        if contract.allows_tool_result_presentation:
            unpresentable = (required - renderer_allowed) - presented
            if unpresentable:
                route_failures.append(
                    f"{component}: AI-only kinds lack presentation policy: "
                    + ", ".join(sorted(unpresentable))
                )
        route_kind_memberships += len(required)

    if route_failures:
        raise SystemExit(
            "Native route-to-screen parity contracts failed:\n  "
            + "\n  ".join(route_failures)
        )
    return len(inventory), route_kind_memberships


def swift_set(source: str, name: str, raw_by_case: dict[str, str]) -> set[str]:
    match = re.search(
        rf"public\s+static\s+let\s+{re.escape(name)}\s*:\s*Set<DaemonKind>\s*=\s*\[(.*?)\]",
        source,
        re.DOTALL,
    )
    if not match:
        raise SystemExit(f"Could not read DesktopDaemonAccessPolicy.{name}")
    cases = re.findall(r"\.(\w+)\b", match.group(1))
    unknown = sorted(set(cases) - raw_by_case.keys())
    if unknown:
        raise SystemExit(f"Unknown DaemonKind cases in {name}: {', '.join(unknown)}")
    return {raw_by_case[case] for case in cases}


def rust_string_set(source: str, name: str) -> set[str]:
    match = re.search(
        rf"const\s+{re.escape(name)}\s*:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
        source,
        re.DOTALL,
    )
    if not match:
        raise SystemExit(f"Could not read Tauri {name}")
    return set(re.findall(r'"([^"]+)"', match.group(1)))


def fail(title: str, values: set[str]) -> None:
    if values:
        raise SystemExit(title + ":\n  " + "\n  ".join(sorted(values)))


def main() -> None:
    inventory = json.loads(
        subprocess.check_output(
            ["python3", str(ROOT / "Scripts" / "inventory_tauri.py")],
            cwd=ROOT,
            text=True,
        )
    )
    required = {kind for route in inventory for kind in route["kinds"]}
    inventory_by_component = {
        route["component"]: set(route["kinds"]) for route in inventory
    }
    inventory_regressions = {
        component: expected - inventory_by_component.get(component, set())
        for component, expected in INVENTORY_KIND_EXPECTATIONS.items()
        if expected - inventory_by_component.get(component, set())
    }
    if inventory_regressions:
        raise SystemExit(
            "Tauri screen inventory omitted bare/streaming daemon actions:\n  "
            + "\n  ".join(
                f"{component}: {', '.join(sorted(missing))}"
                for component, missing in sorted(inventory_regressions.items())
            )
        )
    tauri_actions = {
        (route["component"], action)
        for route in inventory
        for action in route.get("actions", [])
    }

    generated = (
        ROOT / "Sources" / "KassiberDaemonKit" / "Generated" / "DaemonKind.generated.swift"
    ).read_text(encoding="utf-8")
    raw_by_case = {
        case: raw
        for case, raw in re.findall(r'case\s+(\w+)\s*=\s*"([^"]+)"', generated)
    }
    case_by_raw = {raw: case for case, raw in raw_by_case.items()}
    fail("Tauri inventory contains unknown daemon kinds", required - case_by_raw.keys())

    rust = (REPO / "ui-tauri" / "src-tauri" / "src" / "lib.rs").read_text(
        encoding="utf-8"
    )
    renderer_allowed = rust_string_set(rust, "ALLOWED_DAEMON_KINDS")
    policy_source = POLICY.read_text(encoding="utf-8")
    presented = swift_set(
        policy_source, "aiToolResultPresentationKinds", raw_by_case
    )
    ai_only = swift_set(policy_source, "aiOnlyToolResultKinds", raw_by_case)

    route_count, route_kind_memberships = audit_native_route_contracts(
        inventory, raw_by_case, renderer_allowed, presented
    )

    # Privilege-policy and generated-enum mentions are not feature
    # reachability. A native feature source must reference a case directly or
    # declare it as an audited generic AI tool-result presentation.
    feature_sources: list[str] = []
    for path in (ROOT / "Sources").rglob("*.swift"):
        if "Generated" in path.parts or path == POLICY:
            continue
        feature_sources.append(path.read_text(encoding="utf-8"))
    native_source = "\n".join(feature_sources)
    referenced = {
        raw
        for raw, case in case_by_raw.items()
        if re.search(rf"\.{re.escape(case)}\b", native_source)
    }
    direct_calls = {
        raw_by_case[case]
        for case in re.findall(
            r"\.(?:invoke|stream|streamSession)\s*\(\s*\.(\w+)", native_source
        )
        if case in raw_by_case
    }

    renderer_required = required & renderer_allowed
    fail(
        "Native feature source references kinds outside the renderer allowlist",
        referenced - renderer_allowed,
    )
    fail(
        "Renderer-allowed Tauri route kinds have no reachable native use",
        renderer_required - referenced - presented,
    )
    fail(
        "AI-only route kinds are missing from the audited presentation manifest",
        (required - renderer_allowed) - ai_only,
    )
    fail(
        "AI-only presentation manifest contains renderer-allowed kinds",
        ai_only & renderer_allowed,
    )
    fail(
        "AI-only presentation kinds are invoked directly by native UI",
        ai_only & direct_calls,
    )
    fail(
        "AI-only presentation kinds are referenced outside the audited manifest",
        ai_only & referenced,
    )
    fail(
        "AI tool-result presentation manifest contains no Tauri-route use",
        presented - required,
    )
    fail(
        "AI-only kinds are not covered by generic tool-result presentation",
        ai_only - presented,
    )

    ai_model = (
        ROOT / "Sources" / "KassiberViewModels" / "AIChatViewModel.swift"
    ).read_text(encoding="utf-8")
    ai_view = (ROOT / "Sources" / "KassiberApp" / "AIChatScreen.swift").read_text(
        encoding="utf-8"
    )
    if 'case "ai.chat.tool_result"' not in ai_model or "tool.result =" not in ai_model:
        raise SystemExit("AI result manifest has no generic ai.chat.tool_result reducer")
    if "if let result = tool.result" not in ai_view or "jsonText(result)" not in ai_view:
        raise SystemExit("AI result manifest has no generic native tool-result renderer")

    native_actions = set(NATIVE_SCREEN_CONTRACT_PATTERNS)
    missing_contracts = tauri_actions - native_actions
    if missing_contracts:
        raise SystemExit(
            "Tauri screen presentation/actions lack native lockstep contracts:\n  "
            + "\n  ".join(f"{component}: {action}" for component, action in sorted(missing_contracts))
        )
    stale_contracts = native_actions - tauri_actions
    if stale_contracts:
        raise SystemExit(
            "Native screen contracts no longer exist in Tauri inventory:\n  "
            + "\n  ".join(f"{component}: {action}" for component, action in sorted(stale_contracts))
        )
    action_failures: list[str] = []
    source_cache: dict[str, str] = {}
    for component, action in sorted(tauri_actions):
        for relative, pattern in NATIVE_SCREEN_CONTRACT_PATTERNS[(component, action)]:
            source = source_cache.setdefault(
                relative, (ROOT / relative).read_text(encoding="utf-8")
            )
            if not re.search(pattern, source, re.DOTALL):
                action_failures.append(f"{component}: {action} -> {relative} / {pattern}")
    if action_failures:
        raise SystemExit(
            "Native screen presentation/actions are not wired end-to-end:\n  "
            + "\n  ".join(action_failures)
        )

    print(
        f"{route_count} native route-to-screen contracts cover "
        f"{route_kind_memberships} route/kind memberships; "
        "native frontend semantically covers "
        f"all {len(required)} Tauri route daemon kinds "
        f"({len(renderer_required)} renderer-reachable, {len(ai_only)} AI-only presented) "
        f"and {len(tauri_actions)} screen presentation/action contracts"
    )


if __name__ == "__main__":
    main()
