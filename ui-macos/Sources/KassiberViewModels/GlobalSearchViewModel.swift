import Foundation
import Observation
import KassiberDaemonKit

public enum GlobalSearchCategory: String, Sendable {
    case page
    case connection
    case transaction
    case action
    case report
    case reviewItem = "review_item"
    case setting
}

public enum GlobalSearchAction: String, Sendable {
    case addWallet
    case connectBTCPay
    case importBTCPay
    case processJournals
}

public enum GlobalSearchDestination: Equatable, Sendable {
    case screen(AppScreen)
    case connection(String)
    case transaction(String)
    case action(GlobalSearchAction)
    case settings(String)
}

public struct GlobalSearchResult: Identifiable, Equatable, Sendable {
    public let id: String
    public let category: GlobalSearchCategory
    public let titleKey: String?
    public let title: String?
    public let subtitleKey: String?
    public let subtitle: String?
    public let keywords: [String]
    public let destination: GlobalSearchDestination
    public let icon: String

    public init(
        id: String,
        category: GlobalSearchCategory,
        titleKey: String? = nil,
        title: String? = nil,
        subtitleKey: String? = nil,
        subtitle: String? = nil,
        keywords: [String] = [],
        destination: GlobalSearchDestination,
        icon: String
    ) {
        self.id = id
        self.category = category
        self.titleKey = titleKey
        self.title = title
        self.subtitleKey = subtitleKey
        self.subtitle = subtitle
        self.keywords = keywords
        self.destination = destination
        self.icon = icon
    }
}

@MainActor
@Observable
public final class GlobalSearchViewModel {
    public var query = "" {
        didSet { rebuildResults() }
    }
    public private(set) var results: [GlobalSearchResult] = []
    public private(set) var isLoading = false
    public private(set) var isResolving = false
    public private(set) var errorMessage: String?
    public var aiFeaturesEnabled = true {
        didSet { rebuildResults() }
    }
    public var developerToolsEnabled = true {
        didSet { rebuildResults() }
    }

    private let daemon: any DaemonClient
    private var loadRequested = false
    private var connections: [GlobalSearchResult] = []
    private var transactions: [GlobalSearchResult] = []
    private var reviewItems: [GlobalSearchResult] = []
    private var resolvedTransaction: GlobalSearchResult?

    public init(daemon: any DaemonClient) {
        self.daemon = daemon
    }

    public func load() async {
        loadRequested = true
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }
        while loadRequested {
            loadRequested = false
            await loadSnapshot()
        }
    }

    /// GlobalSearchChrome feeds the supervisor's broadcast host stream here.
    /// Only terminal mutations that can change indexed overview rows reload
    /// the snapshot; ordinary reads (including exact tx lookup) never recurse.
    public func handleHostEvent(_ event: DaemonRecord) async {
        guard Self.invalidatesIndex(event) else { return }
        await load()
    }

    public static func invalidatesIndex(_ event: DaemonRecord) -> Bool {
        guard event.kind == "native.request.activity",
              let data = event.data?.objectValue,
              data.string("state") == "finished",
              let kind = data.string("kind") else { return false }
        return indexInvalidatingKinds.contains(kind)
    }

    private func loadSnapshot() async {
        do {
            let result = try await daemon.invoke(.uiOverviewSnapshot, args: nil)
            if let error = result.error { throw error }
            let root = result.data?.objectValue ?? [:]
            connections = root.objects("connections").compactMap { row in
                guard let id = row.string("id", "label"),
                      let label = row.string("label"), !label.isEmpty else { return nil }
                let details = [row.string("kind"), row.string("network"), row.string("status", "sync_status")]
                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
                return GlobalSearchResult(
                    id: "connection:\(id)", category: .connection, title: label,
                    subtitle: details, keywords: [id, row.string("kind") ?? "", row.string("network") ?? ""],
                    destination: .connection(id), icon: "wallet.bifold"
                )
            }
            transactions = (root["txs"]?.arrayValue ?? []).compactMap(TransactionRow.init).map { row in
                let btcpay = ([row.wallet, row.counterparty, row.note] + row.tags)
                    .contains { $0.localizedCaseInsensitiveContains("btcpay") }
                return GlobalSearchResult(
                    id: "transaction:\(row.id)", category: .transaction,
                    title: row.transactionID ?? row.id,
                    subtitle: [row.wallet, row.counterparty, row.type].filter { !$0.isEmpty }.joined(separator: " · "),
                    keywords: [row.id, row.transactionID ?? "", row.wallet, row.counterparty, row.note]
                        + row.tags
                        + (btcpay ? ["btcpay payment", "btcpay invoice", "merchant payment", "payment request"] : []),
                    destination: .transaction(row.id), icon: "arrow.left.arrow.right"
                )
            }
            let status = root["status"]?.objectValue ?? [:]
            reviewItems = []
            if status.bool("needsJournals", "needs_journals") == true {
                reviewItems.append(GlobalSearchResult(
                    id: "review:journals", category: .reviewItem,
                    titleKey: "search.review.journals", subtitleKey: "search.review.journalsHint",
                    keywords: ["journal", "journals", "ledger", "reports", "stale", "process", "review"],
                    destination: .action(.processJournals), icon: "books.vertical"
                ))
            }
            let quarantines = Int(status.int("quarantines", "quarantine") ?? 0)
            if quarantines > 0 {
                reviewItems.append(GlobalSearchResult(
                    id: "review:quarantine", category: .reviewItem,
                    titleKey: "search.review.quarantine",
                    subtitle: "\(quarantines)",
                    keywords: ["quarantine", "review", "missing", "price", "transactions"],
                    destination: .screen(.quarantine), icon: "exclamationmark.triangle"
                ))
            }
            errorMessage = nil
            rebuildResults()
        } catch {
            errorMessage = String(describing: error)
        }
    }

    /// Resolves exact local identifiers without exposing the AI-only fuzzy
    /// transaction-search kind to the renderer.
    public func resolveExactTransactionIfNeeded() async {
        let candidate = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard Self.looksLikeTransactionIdentifier(candidate) else {
            resolvedTransaction = nil
            rebuildResults()
            return
        }
        isResolving = true
        defer { isResolving = false }
        do {
            let result = try await daemon.invoke(
                .uiTransactionsResolve,
                args: ["query": .string(candidate)]
            )
            if let error = result.error { throw error }
            guard query.trimmingCharacters(in: .whitespacesAndNewlines) == candidate,
                  let row = result.data?.objectValue?["transaction"]?.objectValue,
                  let id = row.string("id"), !id.isEmpty else {
                resolvedTransaction = nil
                rebuildResults()
                return
            }
            resolvedTransaction = GlobalSearchResult(
                id: "resolved:\(id)", category: .transaction,
                title: row.string("externalId", "external_id", "explorerId", "explorer_id") ?? id,
                subtitle: [row.string("account"), row.string("counter"), row.string("type")]
                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
                keywords: [id, candidate], destination: .transaction(id),
                icon: "arrow.left.arrow.right"
            )
            errorMessage = nil
            rebuildResults()
        } catch {
            errorMessage = String(describing: error)
        }
    }

    private func rebuildResults() {
        let normalized = Self.normalize(query)
        guard !normalized.isEmpty else { results = []; return }
        var candidates = Self.staticResults + connections + transactions + reviewItems
        if let resolvedTransaction { candidates.insert(resolvedTransaction, at: 0) }
        if !aiFeaturesEnabled {
            candidates.removeAll { $0.destination == .screen(.assistant) || $0.destination == .settings("assistant") }
        }
        if !developerToolsEnabled {
            candidates.removeAll { $0.destination == .screen(.logs) }
        }
        results = candidates.compactMap { result -> (GlobalSearchResult, Int)? in
            let fields = [result.title ?? "", result.titleKey ?? "", result.subtitle ?? "", result.subtitleKey ?? ""] + result.keywords
            let normalizedFields = fields.map(Self.normalize)
            let score: Int
            if normalizedFields.contains(normalized) { score = 120 }
            else if normalizedFields.contains(where: { $0.hasPrefix(normalized) }) { score = 95 }
            else if normalizedFields.contains(where: { $0.contains(normalized) }) { score = 70 }
            else {
                let tokens = normalized.split(separator: " ")
                guard tokens.allSatisfy({ token in normalizedFields.contains { $0.contains(token) } }) else { return nil }
                score = 45
            }
            let categoryBoost = result.category == .action ? 4 : result.category == .page ? 3 : 0
            return (result, score + categoryBoost)
        }
        .sorted { lhs, rhs in
            lhs.1 == rhs.1 ? lhs.0.id < rhs.0.id : lhs.1 > rhs.1
        }
        .prefix(10)
        .map(\.0)
    }

    private static func normalize(_ value: String) -> String {
        value.folding(options: [.caseInsensitive, .diacriticInsensitive], locale: .current)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func looksLikeTransactionIdentifier(_ value: String) -> Bool {
        guard !value.contains(where: \.isWhitespace) else { return false }
        if value.count == 64 && value.allSatisfy(\.isHexDigit) { return true }
        return value.count >= 12 && value.allSatisfy { $0.isLetter || $0.isNumber || "-_:".contains($0) }
    }

    private static let staticResults: [GlobalSearchResult] = {
        let pages: [(AppScreen, [String])] = [
            (.dashboard, ["overview", "dashboard", "home", "portfolio", "übersicht", "start"]),
            (.transactions, ["transactions", "tx", "transaktionen", "buchungen"]),
            (.wallets, ["wallets", "wallet", "brieftaschen"]),
            (.reports, ["reports", "export", "berichte", "steuer"]),
            (.journals, ["journals", "ledger", "journale", "buchhaltung"]),
            (.quarantine, ["quarantine", "review", "quarantäne", "prüfen"]),
            (.swaps, ["swaps", "transfers", "tausch", "übertragungen"]),
            (.reconcile, ["reconcile", "identify", "abgleichen", "zuordnen"]),
            (.activity, ["activity", "history", "aktivität", "verlauf"]),
            (.privacyMirror, ["privacy", "mirror", "datenschutz"]),
            (.exitTax, ["exit tax", "wegzug", "wegzugsbesteuerung"]),
            (.sourceFunds, ["source of funds", "funds", "mittelherkunft", "herkunft"]),
            (.books, ["books", "book sets", "bücher", "büchersätze"]),
            (.birdsEye, ["books overview", "birds eye", "bücherübersicht"]),
            (.connections, ["connections", "backends", "verbindungen"]),
            (.imports, ["imports", "csv", "xlsx", "importe"]),
            (.egress, ["egress", "network", "netzwerk"]),
            (.assistant, ["assistant", "chat", "ai", "ki"]),
            (.logs, ["logs", "diagnostics", "protokolle", "diagnose"]),
            (.settings, ["settings", "preferences", "einstellungen"]),
        ]
        var values = pages.map { screen, keywords in
            GlobalSearchResult(
                id: "page:\(screen.rawValue)", category: .page,
                titleKey: screen.localizationKey, keywords: keywords,
                destination: .screen(screen), icon: screen.systemImage
            )
        }
        values += [
            GlobalSearchResult(id: "action:add-wallet", category: .action, titleKey: "search.addWallet", keywords: ["add wallet", "connect wallet", "wallet hinzufügen"], destination: .action(.addWallet), icon: "plus.circle"),
            GlobalSearchResult(id: "action:sync-wallets", category: .action, titleKey: "search.syncWallets", subtitleKey: "search.syncWalletsHint", keywords: ["wallet", "wallets", "sync", "refresh", "connections", "synchronisieren"], destination: .screen(.connections), icon: "arrow.triangle.2.circlepath"),
            GlobalSearchResult(id: "action:connect-btcpay", category: .action, titleKey: "search.connectBTCPay", subtitleKey: "search.connectBTCPayHint", keywords: ["btcpay", "merchant", "store", "invoice", "api", "greenfield", "connect"], destination: .action(.connectBTCPay), icon: "cart"),
            GlobalSearchResult(id: "action:import-btcpay", category: .action, titleKey: "search.importBTCPay", subtitleKey: "search.importBTCPayHint", keywords: ["btcpay", "merchant", "csv", "export", "manual", "import"], destination: .action(.importBTCPay), icon: "square.and.arrow.down"),
            GlobalSearchResult(id: "action:process-journals", category: .action, titleKey: "search.processJournals", keywords: ["process journals", "rebuild ledger", "journale verarbeiten"], destination: .action(.processJournals), icon: "books.vertical"),
            GlobalSearchResult(id: "action:export-report", category: .report, titleKey: "search.exportReport", subtitleKey: "search.exportReportHint", keywords: ["report", "reports", "export", "pdf", "csv", "xlsx", "tax", "bericht"], destination: .screen(.reports), icon: "doc.badge.arrow.up"),
            GlobalSearchResult(id: "action:open-logs", category: .action, titleKey: "search.openLogs", subtitleKey: "search.openLogsHint", keywords: ["logs", "daemon", "debug", "diagnostics", "support", "protokolle"], destination: .screen(.logs), icon: "text.alignleft"),
            GlobalSearchResult(id: "action:change-passphrase", category: .setting, titleKey: "search.changePassphrase", subtitleKey: "search.changePassphraseHint", keywords: ["password", "passphrase", "security", "lock", "encryption", "passwort"], destination: .settings("security"), icon: "lock.rotation"),
            GlobalSearchResult(id: "setting:general", category: .setting, titleKey: "settings.parity.general", keywords: ["settings", "general", "language", "appearance", "allgemein"], destination: .settings("general"), icon: "gearshape"),
            GlobalSearchResult(id: "setting:privacy", category: .setting, titleKey: "settings.parity.privacy", keywords: ["privacy", "clipboard", "datenschutz"], destination: .settings("privacy"), icon: "eye.slash"),
            GlobalSearchResult(id: "setting:security", category: .setting, titleKey: "settings.parity.security", keywords: ["security", "touch id", "lock", "sicherheit"], destination: .settings("security"), icon: "lock.shield"),
            GlobalSearchResult(id: "setting:terminal", category: .setting, titleKey: "settings.parity.terminal", keywords: ["terminal", "cli", "command", "path"], destination: .settings("terminal"), icon: "terminal"),
            GlobalSearchResult(id: "setting:bitcoin", category: .setting, titleKey: "settings.parity.bitcoin", keywords: ["backend", "bitcoin", "electrum", "core"], destination: .settings("bitcoin"), icon: "bitcoinsign.circle"),
            GlobalSearchResult(id: "setting:lightning", category: .setting, titleKey: "settings.parity.lightning", keywords: ["lightning", "lnd", "cln", "nwc"], destination: .settings("lightning"), icon: "bolt"),
            GlobalSearchResult(id: "setting:liquid", category: .setting, titleKey: "settings.parity.liquid", keywords: ["liquid", "lbtc", "elements"], destination: .settings("liquid"), icon: "drop"),
            GlobalSearchResult(id: "setting:market", category: .setting, titleKey: "settings.parity.market", keywords: ["market", "rates", "price", "coinbase", "kraken"], destination: .settings("market"), icon: "chart.line.uptrend.xyaxis"),
            GlobalSearchResult(id: "setting:assistant", category: .setting, titleKey: "settings.parity.assistant", keywords: ["provider", "model", "api key", "ki anbieter"], destination: .settings("assistant"), icon: "sparkles"),
            GlobalSearchResult(id: "setting:automation", category: .setting, titleKey: "settings.parity.automation", keywords: ["automation", "maintenance", "sync", "journal"], destination: .settings("automation"), icon: "arrow.triangle.2.circlepath"),
            GlobalSearchResult(id: "setting:replication", category: .setting, titleKey: "settings.parity.replication", keywords: ["replication", "sync", "devices", "team", "devices"], destination: .settings("replication"), icon: "person.2.wave.2"),
            GlobalSearchResult(id: "setting:data", category: .setting, titleKey: "settings.parity.data", keywords: ["data", "reset", "delete", "backup", "database"], destination: .settings("data"), icon: "externaldrive"),
        ]
        return values
    }()

    private static let indexInvalidatingKinds: Set<String> = [
        "ui.wallets.create", "ui.wallets.update", "ui.wallets.delete", "ui.wallets.sync",
        "ui.wallets.import_file", "ui.wallets.import_samourai",
        "ui.connections.btcpay.create", "ui.connections.bullbitcoin_wallet.create",
        "ui.metadata.bip329.import", "ui.transactions.metadata.update",
        "ui.transactions.history.revert", "ui.transactions.commercial_context",
        "ui.journals.process", "ui.transfers.pair", "ui.transfers.unpair",
        "ui.transfers.update", "ui.transfers.bulk_pair", "ui.transfers.dismiss",
        "ui.profiles.create", "ui.profiles.rename", "ui.profiles.update", "ui.profiles.switch",
        "ui.profiles.reset_data", "ui.workspace.create", "ui.workspace.rename",
        "ui.workspace.delete", "ui.projects.create", "ui.projects.select",
        "ui.maintenance.run", "ui.freshness.run", "ui.workspace.freshness.run",
        "ui.rates.kraken_csv.import", "ui.rates.rebuild", "ui.onboarding.complete",
    ]
}
