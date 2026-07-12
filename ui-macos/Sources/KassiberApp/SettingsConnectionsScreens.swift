import SwiftUI
import AppKit
import UniformTypeIdentifiers
import Charts
import KassiberDaemonKit
import KassiberViewModels

private func parityString(_ key: String) -> String { AppLocalization.string(key) }

struct FullConnectionsScreen: View {
    let daemon: any DaemonClient
    @State private var model: ConnectionsParityViewModel
    @State private var detail: ConnectionDetailParityViewModel
    @AppStorage("connections.selectedWalletID") private var selectedWalletID: String?
    @State private var setupMode: ConnectionSetupMode?
    @State private var catalogOpen = false
    @State private var backendKindOverride: String?
    @Environment(\.locale) private var locale
    @Environment(\.kassiberDisplayCurrency) private var displayCurrency
    @Environment(\.kassiberNavigate) private var navigate

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _model = State(initialValue: ConnectionsParityViewModel(daemon: daemon))
        _detail = State(initialValue: ConnectionDetailParityViewModel(daemon: daemon))
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedWalletID) {
                ForEach(ConnectionLayer.allCases) { layer in
                    Section(parityString("connections.layer.\(layer.rawValue)")) {
                        ForEach(wallets(layer)) { wallet in
                            Label {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(wallet.label)
                                    Text([wallet.kind, wallet.network].filter { !$0.isEmpty }.map(AppLocalization.code).joined(separator: " · "))
                                        .font(.caption).foregroundStyle(.secondary)
                                    if let balance = wallet.balanceBTC {
                                        Text(connectionBalance(balance))
                                            .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                                            .kassiberSensitive()
                                    }
                                }
                            } icon: {
                                Image(systemName: layer == .lightning ? "bolt.fill" : layer == .liquid ? "drop.fill" : "bitcoinsign.circle")
                            }
                            .tag(wallet.id)
                        }
                    }
                }
            }
            .navigationTitle(parityString("nav.connections"))
            .navigationSplitViewColumnWidth(min: 220, ideal: 270)
        } detail: {
            if let wallet = model.wallets.first(where: { $0.id == selectedWalletID }) {
                NativeConnectionDetailView(
                    daemon: daemon, wallet: wallet, priceEUR: model.priceEUR, model: detail,
                    didMutate: { Task { await model.load() } }
                )
                    .id(wallet.id)
                    .task { await detail.load(walletRef: wallet.id) }
            } else {
                ContentUnavailableView(
                    parityString("connections.select"), systemImage: "point.3.connected.trianglepath.dotted",
                    description: Text(parityString("connections.selectHint"))
                )
            }
        }
        .toolbar {
            Button { catalogOpen = true } label: {
                Label(parityString("connections.add"), systemImage: "plus")
            }
            Button { Task { await model.load() } } label: { Image(systemName: "arrow.clockwise") }
                .disabled(model.isWorking)
        }
        .sheet(isPresented: $catalogOpen) {
            ConnectionCatalogSheet(sources: model.catalog.sources) { source in
                openCatalogSource(source)
            }
        }
        .sheet(item: $setupMode) { mode in
            ConnectionSetupSheet(
                mode: mode, model: model, daemon: daemon,
                backendKindOverride: backendKindOverride
            ) {
                setupMode = nil
                backendKindOverride = nil
                Task { await model.load() }
            }
        }
        .task {
            await model.load()
            if selectedWalletID == nil || !model.wallets.contains(where: { $0.id == selectedWalletID }) {
                selectedWalletID = model.wallets.first?.id
            }
            consumePendingAddWallet()
        }
        .onReceive(NotificationCenter.default.publisher(for: KassiberHostNotification.addWallet)) { _ in
            consumePendingAddWallet()
        }
    }

    private func consumePendingAddWallet() {
        guard UserDefaults.standard.bool(forKey: KassiberHostNotification.pendingAddWalletDefaultsKey) else { return }
        UserDefaults.standard.set(false, forKey: KassiberHostNotification.pendingAddWalletDefaultsKey)
        let raw = UserDefaults.standard.string(
            forKey: KassiberHostNotification.pendingConnectionSetupModeDefaultsKey
        )
        UserDefaults.standard.removeObject(
            forKey: KassiberHostNotification.pendingConnectionSetupModeDefaultsKey
        )
        guard let raw, let mode = ConnectionSetupMode.resolve(raw) else {
            catalogOpen = true
            return
        }
        setupMode = mode
    }

    private func openCatalogSource(_ source: ConnectionCatalogSource) {
        guard source.isEnabled else { return }
        catalogOpen = false
        switch source.route {
        case .descriptor:
            presentSetup(.wallet)
        case .addressList:
            presentSetup(.addressList)
        case .silentPayment:
            presentSetup(.silentPayment)
        case .liquidDescriptor:
            presentSetup(.liquidWallet)
        case let .backend(kind):
            backendKindOverride = kind
            presentSetup(kind == "bitcoinrpc" ? .bitcoinCore : .backend)
        case .btcpay:
            presentSetup(.btcpay)
        case .bullBitcoinWallet:
            presentSetup(.bullbitcoin)
        case .samourai:
            presentSetup(.samourai)
        case .bip329:
            presentSetup(.bip329)
        case let .fileImport(format):
            UserDefaults.standard.set(format, forKey: KassiberHostNotification.pendingImportFormatDefaultsKey)
            navigate(.imports)
            DispatchQueue.main.async {
                NotificationCenter.default.post(name: KassiberHostNotification.openImportFormat, object: nil)
            }
        case .planned:
            break
        }
    }

    private func presentSetup(_ mode: ConnectionSetupMode) {
        DispatchQueue.main.async { setupMode = mode }
    }

    private func wallets(_ layer: ConnectionLayer) -> [WalletRow] {
        model.wallets.filter { wallet in
            switch layer {
            case .liquid: wallet.chain == "liquid"
            case .lightning: ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(wallet.kind)
            case .base: wallet.chain != "liquid" && !["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(wallet.kind)
            }
        }
    }

    private func connectionBalance(_ btc: Double) -> String {
        if displayCurrency == .euro, let rate = model.priceEUR {
            return KassiberFormatting.fiat(btc * rate, currency: "EUR", locale: locale)
        }
        return KassiberFormatting.btc(btc, locale: locale)
    }
}

struct NativeConnectionDetailView: View {
    let daemon: any DaemonClient
    let wallet: WalletRow
    let priceEUR: Double?
    @Bindable var model: ConnectionDetailParityViewModel
    let didMutate: () -> Void
    @State private var editOpen = false
    @State private var revealOpen = false
    @State private var deleteOpen = false
    @State private var editDraft = ConnectionEditDraft()
    @State private var passphrase = ""
    @State private var revealPlaintextAck = ""
    @State private var deletePlaintextAck = ""
    @State private var confirmLabel = ""
    @State private var transactionDetail: TransactionRow?
    @State private var transactionReference: String?
    @State private var nodeWindowDays = 30
    @Environment(\.locale) private var locale
    @Environment(\.kassiberNavigate) private var navigate
    @Environment(\.kassiberDisplayCurrency) private var displayCurrency

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(wallet.label).font(.largeTitle.bold())
                        Text([wallet.kind, wallet.chain, wallet.network].filter { !$0.isEmpty }.map(AppLocalization.code).joined(separator: " · "))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button(parityString("connections.refresh")) {
                        Task {
                            if model.editMetadata?.isNode == true {
                                await model.loadNode(walletRef: wallet.id, windowDays: nodeWindowDays)
                            }
                            else { await model.sync(walletRef: wallet.id) }
                        }
                    }
                        .buttonStyle(.borderedProminent).disabled(model.isWorking || model.isNodeWorking)
                    if model.editMetadata?.isNode != true {
                        Menu {
                            Button(parityString("connections.fullRescan")) { Task { await model.sync(walletRef: wallet.id, forceFull: true) } }
                            Button(parityString("action.edit")) { prepareEdit() }
                            if model.editMetadata?.hasDescriptor == true {
                                Button(parityString("connections.reveal")) {
                                    model.clearRevealedMaterial()
                                    passphrase = ""; revealPlaintextAck = ""; revealOpen = true
                                }
                            }
                            Divider()
                            Button(parityString("action.delete"), role: .destructive) {
                                confirmLabel = ""; passphrase = ""; deletePlaintextAck = ""; deleteOpen = true
                            }
                        } label: { Image(systemName: "ellipsis.circle") }
                    }
                }
                if let progress = model.syncProgress {
                    GroupBox(parityString("connections.syncProgress")) {
                        VStack(alignment: .leading) {
                            ProgressView(value: progress.total > 0 ? Double(progress.processed) / Double(progress.total) : nil)
                            Text("\(AppLocalization.code(progress.phase)) · \(progress.processed)/\(progress.total) · +\(progress.imported) / \(progress.skipped)")
                                .font(.caption).foregroundStyle(.secondary)
                        }.padding(6)
                    }
                }
                if model.editMetadata?.isNode == true {
                    NativeLightningNodeDetailView(
                        wallet: wallet, priceEUR: priceEUR, model: model,
                        windowDays: $nodeWindowDays
                    )
                } else {
                HStack(spacing: 12) {
                    parityMetric(
                        parityString("field.balance"),
                        wallet.balanceBTC.map(connectionBalance) ?? "—",
                        "bitcoinsign.circle"
                    )
                    .kassiberSensitive()
                    parityMetric(parityString("connections.transactions"), String(max(wallet.transactionCount, model.recentTransactionCount)), "list.bullet.rectangle")
                    parityMetric(parityString("connections.utxos"), String(model.utxoCount), "shippingbox")
                    parityMetric(
                        parityString("connections.lastSync"),
                        wallet.lastSyncedAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—",
                        "clock.arrow.circlepath"
                    )
                }
                GroupBox(parityString("connections.walletDetails")) {
                    Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 7) {
                        parityGridRow(parityString("field.name"), wallet.label)
                        parityGridRow(parityString("field.type"), AppLocalization.code(wallet.kind))
                        parityGridRow(parityString("field.network"), [wallet.chain, wallet.network].filter { !$0.isEmpty }.map(AppLocalization.code).joined(separator: " · "))
                        parityGridRow(parityString("connections.account"), wallet.account.isEmpty ? "—" : wallet.account)
                        parityGridRow(parityString("connections.lastSync"), wallet.lastSyncedAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—")
                        if let metadata = model.editMetadata {
                            parityGridRow(parityString("connections.syncMode"), metadata.syncMode.isEmpty ? "—" : metadata.syncMode)
                            parityGridRow(parityString("connections.backend"), metadata.backendName.isEmpty ? "—" : metadata.backendName)
                            if let gap = metadata.gapLimit { parityGridRow(parityString("wallet.gapLimit"), String(gap)) }
                            if !metadata.sourceFormat.isEmpty { parityGridRow(parityString("importsParity.format"), metadata.sourceFormat) }
                            if !metadata.scriptTypes.isEmpty { parityGridRow(parityString("connections.scriptTypes"), metadata.scriptTypes.joined(separator: ", ")) }
                        }
                    }.padding(8)
                }
                if model.balanceHistory.count > 1 {
                    WalletBalanceHistoryChart(
                        points: model.balanceHistory,
                        currency: displayCurrency,
                        priceEUR: priceEUR
                    )
                }
                GroupBox(parityString("wallet.recentTransactions")) {
                    Table(model.recentTransactions) {
                        TableColumn(parityString("field.date"), value: \.dateLabel)
                        TableColumn(parityString("field.type")) { row in Text(AppLocalization.code(row.type)) }
                        TableColumn(parityString("field.amount")) { row in KassiberAmountText(transaction: row, rateEUR: priceEUR).monospacedDigit().kassiberSensitive() }
                        TableColumn("") { row in Button(parityString("sourceFundsParity.details")) { transactionDetail = row }.buttonStyle(.link) }.width(65)
                    }.frame(height: 210)
                }
                GroupBox(parityString("wallet.utxos")) {
                    if let message = model.inventoryMessage {
                        Text(parityString(message)).foregroundStyle(.secondary).padding()
                    } else {
                        Table(model.utxos) {
                            TableColumn(parityString("field.reference"), value: \.outpoint)
                            TableColumn(parityString("field.amount")) { row in KassiberAmountText(sats: row.amountSats, rateEUR: priceEUR).monospacedDigit().kassiberSensitive() }
                            TableColumn(parityString("field.status")) { row in Text(AppLocalization.code(row.status)) }
                            TableColumn(parityString("wallet.addressLabel"), value: \.addressLabel)
                            TableColumn("") { row in
                                HStack {
                                    Button { NativeAffordances.copy(row.outpoint) } label: { Image(systemName: "doc.on.doc") }.buttonStyle(.borderless).help(parityString("action.copy"))
                                    if !row.transactionID.isEmpty {
                                        Button(parityString("sourceFundsParity.details")) { transactionReference = row.transactionID }.buttonStyle(.link)
                                    }
                                }
                            }.width(95)
                        }.frame(height: 220)
                    }
                }
                if let metadata = model.editMetadata, !metadata.provenanceRoutes.isEmpty {
                    GroupBox(parityString("connections.provenanceRoutes")) {
                        Table(metadata.provenanceRoutes) {
                            TableColumn(parityString("connections.backend"), value: \.backend)
                            TableColumn(parityString("connections.store"), value: \.storeID)
                            TableColumn(parityString("connections.paymentMethod"), value: \.paymentMethodID)
                        }.frame(height: min(190, CGFloat(metadata.provenanceRoutes.count * 34 + 44)))
                    }
                }
                GroupBox(parityString("connections.relatedViews")) {
                    HStack {
                        Button(parityString("nav.transactions")) { navigate(.transactions) }
                        Button(parityString("nav.imports")) { navigate(.imports) }
                        Button(parityString("nav.sourceFunds")) { navigate(.sourceFunds) }
                        Button(parityString("nav.reports")) { navigate(.reports) }
                        Button(parityString("nav.journals")) { navigate(.journals) }
                    }.padding(6)
                }
                }
                if let error = model.errorMessage { Label(parityError(error), systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
            }.padding(22)
        }
        .sheet(isPresented: $editOpen) {
            ConnectionEditParitySheet(
                wallet: wallet, model: model, draft: $editDraft, passphrase: $passphrase
            ) {
                editOpen = false
                didMutate()
                Task { await model.load(walletRef: wallet.id) }
            }
        }
        .sheet(isPresented: $revealOpen, onDismiss: clearRevealState) {
            NavigationStack {
                Form {
                    if model.databaseEncrypted {
                        SecureField(parityString("security.currentPassphrase"), text: $passphrase)
                    } else {
                        Text(parityString("connections.revealPlaintextWarning")).foregroundStyle(.orange)
                        TextField(parityString("connections.confirmation.reveal"), text: $revealPlaintextAck)
                    }
                    if let material = model.revealedMaterial {
                        Text(material).font(.system(.caption, design: .monospaced)).textSelection(.enabled).kassiberSensitive()
                        Button(parityString("connections.copyRevealed")) { NativeAffordances.copy(material) }
                            .kassiberSensitive()
                    } else {
                        Text(parityString("connections.revealWarning")).font(.caption).foregroundStyle(.secondary)
                    }
                }.formStyle(.grouped).navigationTitle(parityString("connections.reveal"))
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button(parityString("action.close")) {
                                clearRevealState()
                                revealOpen = false
                            }
                        }
                        ToolbarItem(placement: .confirmationAction) {
                            Button(parityString("connections.reveal")) {
                                Task {
                                    await model.revealDescriptor(
                                        walletRef: wallet.id, passphrase: passphrase,
                                        plaintextConfirmed: revealPlaintextAck == "REVEAL LOCAL DATA"
                                    )
                                }
                            }.disabled(model.databaseEncrypted ? passphrase.isEmpty : revealPlaintextAck != "REVEAL LOCAL DATA")
                        }
                    }
            }
            .frame(width: 600, height: 340)
            .onDisappear { clearRevealState() }
        }
        .sheet(item: $transactionDetail) { row in
            TransactionDetailSheet(daemon: daemon, transaction: row) {
                transactionDetail = nil
                Task { await model.load(walletRef: wallet.id) }
            }
        }
        .sheet(isPresented: Binding(
            get: { transactionReference != nil },
            set: { if !$0 { transactionReference = nil } }
        )) {
            if let transactionReference {
                ConnectionResolvedTransactionSheet(daemon: daemon, reference: transactionReference) {
                    self.transactionReference = nil
                    Task { await model.load(walletRef: wallet.id) }
                }
            }
        }
        .sheet(isPresented: $deleteOpen) {
            NavigationStack {
                Form {
                    Text(parityString("connections.deleteWarning")).foregroundStyle(.red)
                    TextField(wallet.label, text: $confirmLabel)
                    if model.databaseEncrypted {
                        SecureField(parityString("security.currentPassphrase"), text: $passphrase)
                    } else {
                        Text(parityString("connections.deletePlaintextWarning")).foregroundStyle(.orange)
                        TextField(parityString("connections.confirmation.delete"), text: $deletePlaintextAck)
                    }
                }.formStyle(.grouped).navigationTitle(parityString("connections.delete"))
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) { Button(parityString("action.cancel")) { deleteOpen = false } }
                        ToolbarItem(placement: .confirmationAction) {
                            Button(parityString("action.delete"), role: .destructive) {
                                Task {
                                    await model.delete(
                                        walletRef: wallet.id, label: wallet.label, cascade: true,
                                        passphrase: passphrase,
                                        plaintextConfirmed: deletePlaintextAck == "DELETE LOCAL DATA"
                                    )
                                    if model.didDelete { deleteOpen = false; didMutate() }
                                }
                            }.disabled(
                                confirmLabel != wallet.label || model.isWorking
                                    || (model.databaseEncrypted ? passphrase.isEmpty : deletePlaintextAck != "DELETE LOCAL DATA")
                            )
                        }
                    }
            }.frame(width: 470, height: 300)
        }
    }

    private func prepareEdit() {
        editDraft = model.makeEditDraft(fallback: wallet)
        passphrase = ""
        editOpen = true
    }

    private func clearRevealState() {
        model.clearRevealedMaterial()
        passphrase = ""
        revealPlaintextAck = ""
    }

    private func connectionBalance(_ btc: Double) -> String {
        if displayCurrency == .euro, let priceEUR {
            return KassiberFormatting.fiat(btc * priceEUR, currency: "EUR", locale: locale)
        }
        return KassiberFormatting.btc(btc, locale: locale)
    }

    private func parityMetric(_ label: String, _ value: String, _ icon: String) -> some View {
        GroupBox { VStack(alignment: .leading) { Label(label, systemImage: icon).foregroundStyle(.secondary); Text(value).font(.title2.bold()) }.frame(maxWidth: .infinity, alignment: .leading).padding(6) }
    }

    @ViewBuilder
    private func parityGridRow(_ label: String, _ value: String, sensitive: Bool = false) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            if sensitive { Text(value).textSelection(.enabled).kassiberSensitive() }
            else { Text(value).textSelection(.enabled) }
        }
    }
}

private struct NativeLightningNodeDetailView: View {
    let wallet: WalletRow
    let priceEUR: Double?
    @Bindable var model: ConnectionDetailParityViewModel
    @Binding var windowDays: Int
    @Environment(\.locale) private var locale
    @Environment(\.kassiberNavigate) private var navigate

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Picker(parityString("connections.node.window"), selection: $windowDays) {
                    ForEach(ConnectionDetailParityViewModel.nodeWindowOptions, id: \.self) { days in
                        Text(String(format: parityString("connections.node.windowDays %lld"), Int64(days)))
                            .tag(days)
                    }
                }
                .pickerStyle(.menu)
                .disabled(model.isNodeWorking)
                if model.isNodeWorking { ProgressView().controlSize(.small) }
                Spacer()
                Text(model.nodeRouting?.window ?? "")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .onChange(of: windowDays) { _, days in
                Task { await model.loadNode(walletRef: wallet.id, windowDays: days) }
            }

            Picker(parityString("connections.node.sections"), selection: $model.selectedNodeTab) {
                ForEach(ConnectionNodeDetailTab.allCases) { tab in
                    Text(parityString("connections.node.tab.\(tab.rawValue)")).tag(tab)
                }
            }
            .pickerStyle(.segmented)

            if let node = model.nodeSummary {
                switch model.selectedNodeTab {
                case .overview: overview(node)
                case .channels: channels
                case .activity: activity(node)
                case .profitability: profitability
                case .accounting: accounting(node)
                }
            } else if model.isNodeWorking {
                HStack { Spacer(); ProgressView(parityString("connections.node.loading")); Spacer() }
                    .frame(minHeight: 220)
            } else {
                ContentUnavailableView(
                    parityString("connections.node.noSnapshot"), systemImage: "bolt.slash",
                    description: Text(parityString("connections.node.noSnapshotHint"))
                )
                .frame(minHeight: 260)
            }
        }
        .sheet(item: Binding(
            get: { model.selectedNodeChannel },
            set: { model.selectNodeChannel($0?.id) }
        )) { channel in
            NativeLightningChannelDetailSheet(channel: channel)
        }
    }

    @ViewBuilder
    private func overview(_ node: ConnectionNodeSummary) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 185), spacing: 12)], spacing: 12) {
            nodeMetric("connections.localBalance", sats(node.localBalanceSats), "bolt.fill")
            nodeMetric("connections.remoteBalance", sats(node.remoteBalanceSats), "arrow.down.left.circle")
            nodeMetric("connections.node.onchainReserve", sats(node.onchainBalanceSats), "bitcoinsign.circle")
            nodeMetric("connections.netProfit", signedSats(model.nodeRouting?.netProfitSats ?? 0), "chart.line.uptrend.xyaxis")
        }
        GroupBox(parityString("connections.node.liquidity")) {
            VStack(alignment: .leading, spacing: 8) {
                ProgressView(
                    value: node.totalCapacitySats > 0
                        ? Double(node.localBalanceSats) / Double(node.totalCapacitySats) : 0
                )
                HStack {
                    Text(String(format: parityString("connections.node.activeChannels %lld"), Int64(node.activeChannels)))
                    Spacer()
                    Text(String(format: parityString("connections.node.peers %lld"), Int64(node.peerCount)))
                    Spacer()
                    Text(sats(node.totalCapacitySats)).kassiberSensitive()
                }.font(.caption).foregroundStyle(.secondary)
            }.padding(8)
        }
        activitySummary(node)
        accountingSummary(node)
    }

    @ViewBuilder
    private var channels: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                if !model.nodeClosedChannels.isEmpty {
                    Toggle(
                        String(
                            format: parityString("connections.node.showClosed %lld"),
                            Int64(model.nodeClosedChannels.count)
                        ),
                        isOn: $model.showClosedNodeChannels
                    )
                    .toggleStyle(.switch)
                }
                if model.visibleNodeChannels.isEmpty {
                    ContentUnavailableView(
                        parityString("connections.node.noChannels"), systemImage: "bolt.slash"
                    ).frame(height: 230)
                } else {
                    Table(model.visibleNodeChannels) {
                        TableColumn(parityString("connections.peer")) { row in
                            VStack(alignment: .leading, spacing: 2) {
                                Text(row.peer.isEmpty ? "—" : row.peer)
                                Text(row.shortChannelID ?? row.id)
                                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                        }
                        TableColumn(parityString("field.status")) { Text(AppLocalization.code($0.state)) }
                        TableColumn(parityString("connections.capacity")) { Text(sats($0.capacitySats)).kassiberSensitive() }
                        TableColumn(parityString("connections.localBalance")) { Text(sats($0.localBalanceSats)).kassiberSensitive() }
                        TableColumn(parityString("connections.remoteBalance")) { Text(sats($0.remoteBalanceSats)).kassiberSensitive() }
                        TableColumn(parityString("connections.node.feePolicy")) { row in
                            Text(row.feeRatePPM.map { "\($0) ppm" } ?? "—")
                        }
                        TableColumn("") { row in
                            Button(parityString("field.details")) { model.selectNodeChannel(row.id) }
                                .buttonStyle(.link)
                        }.width(62)
                    }
                    .frame(minHeight: 330)
                }
            }.padding(8)
        } label: {
            Label(parityString("connections.node.tab.channels"), systemImage: "point.3.connected.trianglepath.dotted")
        }
    }

    @ViewBuilder
    private func activity(_ node: ConnectionNodeSummary) -> some View {
        activitySummary(node)
        GroupBox(parityString("connections.recentForwards")) {
            if model.nodeForwards.isEmpty {
                ContentUnavailableView(
                    parityString("connections.node.noForwards"), systemImage: "arrow.left.arrow.right"
                ).frame(height: 190)
            } else {
                Table(model.nodeForwards) {
                    TableColumn(parityString("field.date")) { row in
                        Text(row.occurredAt.map { KassiberFormatting.date($0, locale: locale) } ?? "—")
                    }
                    TableColumn(parityString("connections.route")) { row in
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.route.isEmpty ? "—" : row.route)
                            let ids = [row.inShortChannelID, row.outShortChannelID].compactMap { $0 }
                            if !ids.isEmpty {
                                Text(ids.joined(separator: " → "))
                                    .font(.caption.monospaced()).foregroundStyle(.secondary)
                            }
                        }
                    }
                    TableColumn(parityString("field.amount")) { Text(sats($0.amountSats)).kassiberSensitive() }
                    TableColumn(parityString("swaps.fee")) { Text(sats($0.feeSats)).kassiberSensitive() }
                    TableColumn(parityString("field.status")) { row in
                        VStack(alignment: .leading) {
                            Text(AppLocalization.code(row.status))
                            if let reason = row.failureReason {
                                Text(AppLocalization.code(reason)).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }.frame(minHeight: 300)
            }
        }
    }

    @ViewBuilder
    private var profitability: some View {
        if let report = model.nodeProfitability {
            routingSummary(report.summary)
            GroupBox(parityString("connections.node.channelEconomics")) {
                Table(report.channels) {
                    TableColumn(parityString("connections.peer"), value: \.peer)
                    TableColumn(parityString("connections.capacity")) { Text(sats($0.capacitySats)).kassiberSensitive() }
                    TableColumn(parityString("connections.node.earnedRouting")) { Text(sats($0.earnedSats)).kassiberSensitive() }
                    TableColumn(parityString("connections.node.openCost")) { Text(sats($0.openCostSats)).kassiberSensitive() }
                    TableColumn(parityString("connections.node.coversOpenCost")) { row in
                        Label(
                            row.coversOpenCost ? parityString("common.yes") : parityString("common.no"),
                            systemImage: row.coversOpenCost ? "checkmark.circle.fill" : "clock"
                        )
                        .foregroundStyle(row.coversOpenCost ? .green : .secondary)
                    }
                }.frame(minHeight: 270)
            }
        } else if let routing = model.nodeRouting {
            routingSummary(routing)
            if let error = model.nodeProfitabilityError {
                Label(parityError(error), systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
            }
        } else {
            ContentUnavailableView(
                parityString("connections.node.noProfitability"), systemImage: "chart.line.downtrend.xyaxis"
            ).frame(minHeight: 250)
        }
    }

    @ViewBuilder
    private func accounting(_ node: ConnectionNodeSummary) -> some View {
        accountingSummary(node)
        GroupBox(parityString("connections.node.identity")) {
            Grid(alignment: .leading, horizontalSpacing: 22, verticalSpacing: 8) {
                nodeGridRow("connections.nodeAlias", node.alias)
                nodeGridRow("connections.node.publicKey", node.pubkey, sensitive: true)
                nodeGridRow("field.network", AppLocalization.code(node.network))
                if let version = node.implementationVersion { nodeGridRow("connections.node.implementation", version) }
                if let height = node.blockHeight { nodeGridRow("connections.node.blockHeight", height.formatted()) }
                nodeGridRow("connections.node.peersLabel", node.peerCount.formatted())
                if let connection = model.nodeConnection {
                    nodeGridRow("connections.node.connectionID", connection.id, sensitive: true)
                    nodeGridRow("field.type", AppLocalization.code(connection.kind))
                }
                if let capabilities = model.nodeCapabilities {
                    nodeGridRow("connections.node.capabilities", capabilityLabels(capabilities).joined(separator: ", "))
                }
            }.padding(8)
        }
    }

    @ViewBuilder
    private func activitySummary(_ node: ConnectionNodeSummary) -> some View {
        GroupBox(parityString("connections.node.activity")) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 160), spacing: 10)], spacing: 10) {
                nodeCompactStat(
                    "connections.node.invoices",
                    "\(node.paidInvoiceCount.formatted()) / \(node.invoiceCount.formatted())"
                )
                nodeCompactStat(
                    "connections.node.payments",
                    "\(node.completedPaymentCount.formatted()) / \(node.paymentCount.formatted())"
                )
                nodeCompactStat("connections.forwards", (model.nodeRouting?.forwardCount ?? model.nodeForwards.count).formatted())
                nodeCompactStat("connections.node.exceptions", node.failedOrExpiredCount.formatted())
            }.padding(8)
        }
    }

    @ViewBuilder
    private func accountingSummary(_ node: ConnectionNodeSummary) -> some View {
        GroupBox(parityString("connections.node.accounting")) {
            VStack(alignment: .leading, spacing: 10) {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 175), spacing: 10)], spacing: 10) {
                    nodeCompactStat("connections.transactions", max(wallet.transactionCount, model.recentTransactionCount).formatted())
                    nodeCompactStat("connections.node.bookedInvoices", node.paidInvoiceCount.formatted())
                    nodeCompactStat(
                        "connections.node.operationalEvents",
                        (node.paidInvoiceCount + node.completedPaymentCount + (model.nodeRouting?.forwardCount ?? 0)).formatted()
                    )
                }
                HStack {
                    Button(parityString("nav.transactions")) { navigate(.transactions) }
                    Button(parityString("nav.journals")) { navigate(.journals) }
                    Button(parityString("nav.reports")) { navigate(.reports) }
                }
            }.padding(8)
        }
    }

    @ViewBuilder
    private func routingSummary(_ routing: ConnectionNodeRoutingSummary) -> some View {
        GroupBox(parityString("connections.profitability")) {
            VStack(alignment: .leading, spacing: 10) {
                Text(String(
                    format: parityString("connections.node.routingWindow %@ %lld %lld %lld"),
                    routing.window, Int64(routing.forwardCount), Int64(routing.paymentCount),
                    Int64(routing.rebalanceCount)
                ))
                .font(.caption).foregroundStyle(.secondary)
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 155), spacing: 10)], spacing: 10) {
                    nodeCompactStat("connections.routingRevenue", signedSats(routing.revenueSats))
                    nodeCompactStat("connections.paymentCosts", signedSats(-routing.paymentCostSats))
                    nodeCompactStat("connections.rebalanceCosts", signedSats(-routing.rebalanceCostSats))
                    nodeCompactStat("connections.onchainCosts", signedSats(-routing.onchainCostSats))
                    nodeCompactStat("connections.netProfit", signedSats(routing.netProfitSats))
                }
            }.padding(8)
        }
    }

    private func nodeMetric(_ key: String, _ value: String, _ icon: String) -> some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 5) {
                Label(parityString(key), systemImage: icon).font(.caption).foregroundStyle(.secondary)
                Text(value).font(.title3.bold()).monospacedDigit().kassiberSensitive()
            }.frame(maxWidth: .infinity, alignment: .leading).padding(6)
        }
    }

    private func nodeCompactStat(_ key: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(parityString(key)).font(.caption).foregroundStyle(.secondary)
            Text(value).font(.headline.monospacedDigit())
        }.frame(maxWidth: .infinity, alignment: .leading).padding(8)
            .background(.background, in: RoundedRectangle(cornerRadius: 6))
    }

    @ViewBuilder
    private func nodeGridRow(_ key: String, _ value: String, sensitive: Bool = false) -> some View {
        GridRow {
            Text(parityString(key)).foregroundStyle(.secondary)
            if sensitive { Text(value.isEmpty ? "—" : value).textSelection(.enabled).kassiberSensitive() }
            else { Text(value.isEmpty ? "—" : value).textSelection(.enabled) }
        }
    }

    private func capabilityLabels(_ value: ConnectionNodeCapabilities) -> [String] {
        [
            (value.nodeSnapshot, "connections.node.capability.snapshot"),
            (value.routingProfitability, "connections.node.capability.profitability"),
            (value.channelBalances, "connections.node.capability.balances"),
            (value.channelLifecycle, "connections.node.capability.lifecycle"),
            (value.forwardEvents, "connections.node.capability.forwards"),
            (value.invoiceActivity, "connections.node.capability.invoices"),
            (value.paymentActivity, "connections.node.capability.payments"),
            (value.onchainBalance, "connections.node.capability.onchain"),
        ].compactMap { supported, key in supported ? parityString(key) : nil }
    }

    private func sats(_ value: Int64) -> String { KassiberFormatting.sats(value, locale: locale) }
    private func signedSats(_ value: Int64) -> String {
        (value >= 0 ? "+" : "−") + KassiberFormatting.sats(abs(value), locale: locale)
    }
}

private struct NativeLightningChannelDetailSheet: View {
    let channel: ConnectionNodeChannelRow
    @Environment(\.dismiss) private var dismiss
    @Environment(\.locale) private var locale

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    GroupBox(parityString("connections.node.channelLiquidity")) {
                        VStack(alignment: .leading, spacing: 8) {
                            ProgressView(
                                value: channel.capacitySats > 0
                                    ? Double(channel.localBalanceSats) / Double(channel.capacitySats) : 0
                            )
                            detailRow("connections.localBalance", sats(channel.localBalanceSats), sensitive: true)
                            detailRow("connections.remoteBalance", sats(channel.remoteBalanceSats), sensitive: true)
                            detailRow("connections.capacity", sats(channel.capacitySats), sensitive: true)
                        }.padding(8)
                    }
                    GroupBox(parityString("connections.node.channelPeer")) {
                        VStack(alignment: .leading, spacing: 8) {
                            detailRow("connections.peer", channel.peer)
                            detailRow("connections.node.publicKey", channel.peerPubkey ?? parityString("connections.node.privatePeerHidden"), sensitive: true)
                            detailRow("connections.privacy", channel.isPrivate ? parityString("connections.private") : parityString("connections.public"))
                        }.padding(8)
                    }
                    GroupBox(parityString("connections.node.channelLifecycle")) {
                        VStack(alignment: .leading, spacing: 8) {
                            detailRow("field.status", AppLocalization.code(channel.state))
                            detailRow("connections.node.initiator", channel.isInitiator ? parityString("connections.node.local") : parityString("connections.node.remote"))
                            detailRow("connections.node.shortChannelID", channel.shortChannelID ?? parityString("connections.node.pending"), sensitive: true)
                            if let outpoint = channel.fundingOutpoint {
                                detailRow("connections.node.fundingOutpoint", outpoint, sensitive: true, copy: true)
                            }
                            if let date = channel.openedAt { detailRow("connections.node.opened", dateText(date)) }
                            if let date = channel.closedAt { detailRow("connections.node.closed", dateText(date)) }
                            if let kind = channel.closeKind { detailRow("connections.node.closeKind", AppLocalization.code(kind)) }
                            if let date = channel.lastActivityAt { detailRow("connections.node.lastActivity", dateText(date)) }
                            if let count = channel.htlcCount { detailRow("connections.node.inFlightHTLCs", count.formatted()) }
                        }.padding(8)
                    }
                    GroupBox(parityString("connections.node.channelRouting")) {
                        VStack(alignment: .leading, spacing: 8) {
                            detailRow("connections.node.feeRate", channel.feeRatePPM.map { "\($0) ppm" } ?? "—")
                            detailRow("connections.node.baseFee", channel.baseFeeMsat.map { "\($0) msat" } ?? "—")
                            if let count = channel.forwardCount { detailRow("connections.forwards", count.formatted()) }
                            if let earned = channel.earnedSats { detailRow("connections.node.earnedRouting", sats(earned), sensitive: true) }
                        }.padding(8)
                    }
                }.padding(18)
            }
            .navigationTitle(channel.peer.isEmpty ? channel.id : channel.peer)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(parityString("action.close")) { dismiss() }
                }
            }
        }
        .frame(minWidth: 520, minHeight: 650)
    }

    @ViewBuilder
    private func detailRow(
        _ key: String, _ value: String, sensitive: Bool = false, copy: Bool = false
    ) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(parityString(key)).foregroundStyle(.secondary)
            Spacer()
            Text(value.isEmpty ? "—" : value)
                .font(.body.monospacedDigit()).textSelection(.enabled)
                .modifier(ChannelSensitiveModifier(enabled: sensitive))
            if copy {
                Button { NativeAffordances.copy(value) } label: { Image(systemName: "doc.on.doc") }
                    .buttonStyle(.borderless).help(parityString("action.copy"))
            }
        }
    }

    private func sats(_ value: Int64) -> String { KassiberFormatting.sats(value, locale: locale) }
    private func dateText(_ value: Date) -> String { KassiberFormatting.date(value, locale: locale) }
}

private struct ChannelSensitiveModifier: ViewModifier {
    let enabled: Bool
    func body(content: Content) -> some View {
        if enabled { content.kassiberSensitive() } else { content }
    }
}

private struct ConnectionEditParitySheet: View {
    let wallet: WalletRow
    @Bindable var model: ConnectionDetailParityViewModel
    @Binding var draft: ConnectionEditDraft
    @Binding var passphrase: String
    let didFinish: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var plaintextAcknowledgement = ""

    var body: some View {
        NavigationStack {
            Form {
                Section(parityString("connections.editIdentity")) {
                    TextField(parityString("field.name"), text: $draft.label)
                    Toggle(parityString("wallet.archive"), isOn: $draft.archived)
                }
                if let metadata = model.editMetadata,
                   metadata.canEditLiveBackend, draft.editKind != .btcpay {
                    Section(parityString("connections.syncBackend")) {
                        Picker(parityString("connections.syncBackend"), selection: backendChoice) {
                            Text(metadata.backendName.isEmpty
                                 ? parityString("connections.keepCurrentBackend")
                                 : String(format: parityString("connections.keepNamedBackend"), metadata.backendName))
                                .tag("")
                            if metadata.canClearLiveBackend {
                                Text(parityString("connections.useDefaultBackend")).tag("__clear__")
                            }
                            ForEach(model.liveBackendOptions) { option in
                                Text([option.name, option.network].filter { !$0.isEmpty }.joined(separator: " · "))
                                    .tag(option.name)
                            }
                        }
                        Text(metadata.chain.lowercased() == "liquid"
                             ? parityString("connections.liquidBackendHint")
                             : parityString("connections.backendHint"))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
                switch draft.editKind {
                case .descriptor:
                    descriptorFields
                case .btcpay:
                    btcpayFields
                case .fileWallet:
                    fileFields
                case .basic:
                    EmptyView()
                }
                if let routes = model.editMetadata?.provenanceRoutes, !routes.isEmpty {
                    Section(parityString("connections.provenanceRoutes")) {
                        Text(parityString("connections.provenanceRoutesHint"))
                            .font(.caption).foregroundStyle(.secondary)
                        ForEach(routes) { route in
                            Toggle(isOn: keepRouteBinding(route)) {
                                VStack(alignment: .leading) {
                                    Text(parityString("connections.keepProvenanceRoute"))
                                    Text("\(route.backend) · \(route.storeID) · \(route.paymentMethodID)")
                                        .font(.caption.monospaced()).foregroundStyle(.secondary)
                                }
                            }
                        }
                        Button(parityString("connections.removeAllProvenance"), role: .destructive) {
                            draft.removedProvenanceRouteIDs = Set(routes.map(\.id))
                        }
                    }
                }
                Section(parityString("connections.localAuthorization")) {
                    if model.databaseEncrypted {
                        SecureField(parityString("security.currentPassphrase"), text: $passphrase)
                        Text(parityString("connections.editEncryptedHint"))
                            .font(.caption).foregroundStyle(.secondary)
                    } else {
                        Text(parityString("connections.editPlaintextWarning")).foregroundStyle(.orange)
                        TextField(parityString("connections.confirmation.change"), text: $plaintextAcknowledgement)
                    }
                }
                if let error = model.errorMessage {
                    Section { Label(parityError(error), systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(parityString("connections.edit"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(parityString("action.cancel")) { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(parityString("action.save")) { save() }
                        .disabled(saveDisabled)
                }
            }
        }.frame(width: 680, height: 720)
    }

    @ViewBuilder private var descriptorFields: some View {
        Section(parityString("connections.walletSource")) {
            Text(parityString("connections.walletReplacementSafety"))
                .font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $draft.walletMaterial)
                .font(.system(.body, design: .monospaced)).frame(minHeight: 100)
            Text(parityString("connections.walletMaterialKeepHint"))
                .font(.caption).foregroundStyle(.secondary)
            if isBareXpub || !(model.editMetadata?.scriptTypes.isEmpty ?? true) {
                GroupBox(parityString("connections.scriptTypes")) {
                    VStack(alignment: .leading) {
                        ForEach(["p2wpkh", "p2sh-p2wpkh", "p2pkh", "p2tr"], id: \.self) { value in
                            Toggle(parityString("connections.scriptType.\(value)"), isOn: scriptTypeBinding(value))
                        }
                    }.padding(4)
                }
            }
            TextField(parityString("wallet.gapLimit"), text: $draft.gapLimit)
            Text(parityString("connections.gapLimitHint")).font(.caption).foregroundStyle(.secondary)
            TextField(parityString("connections.walletBirthday"), text: $draft.birthday)
            Text(parityString("connections.walletBirthdayHint")).font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var btcpayFields: some View {
        Section(parityString("connections.btcpayMapping")) {
            Picker(parityString("connections.btcpayInstance"), selection: $draft.backend) {
                Text(parityString("connections.keepCurrentInstance")).tag("")
                ForEach(model.btcpayBackendOptions) { Text($0.name).tag($0.name) }
            }
            TextField(parityString("connections.storeId"), text: $draft.storeID)
            TextField(parityString("connections.paymentMethod"), text: $draft.paymentMethodID)
            Text(parityString("connections.btcpayMappingHint")).font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var fileFields: some View {
        Section(parityString("connections.fileSource")) {
            HStack {
                TextField(parityString("connections.sourceFile"), text: $draft.sourceFile)
                Button(parityString("action.browse")) {
                    if let url = NativeAffordances.chooseFile(types: [.data]) { draft.sourceFile = url.path }
                }
            }
            Text(parityString("connections.sourceFileKeepHint")).font(.caption).foregroundStyle(.secondary)
        }
    }

    private var isBareXpub: Bool {
        let value = draft.walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines)
        return value.hasPrefix("xpub") || value.hasPrefix("tpub")
    }

    private var backendChoice: Binding<String> {
        Binding(
            get: { draft.clearBackend ? "__clear__" : draft.backend },
            set: { value in
                draft.clearBackend = value == "__clear__"
                draft.backend = draft.clearBackend ? "" : value
            }
        )
    }

    private func scriptTypeBinding(_ value: String) -> Binding<Bool> {
        Binding(
            get: { draft.scriptTypes.contains(value) },
            set: { selected in
                if selected { draft.scriptTypes.insert(value) }
                else { draft.scriptTypes.remove(value) }
            }
        )
    }

    private func keepRouteBinding(_ route: ConnectionProvenanceRoute) -> Binding<Bool> {
        Binding(
            get: { !draft.removedProvenanceRouteIDs.contains(route.id) },
            set: { keep in
                if keep { draft.removedProvenanceRouteIDs.remove(route.id) }
                else { draft.removedProvenanceRouteIDs.insert(route.id) }
            }
        )
    }

    private var saveDisabled: Bool {
        draft.label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || model.isWorking
            || (model.databaseEncrypted ? passphrase.isEmpty : plaintextAcknowledgement != "CHANGE LOCAL DATA")
    }

    private func save() {
        Task {
            await model.updateConfiguration(
                walletRef: wallet.id, original: wallet, draft: draft, passphrase: passphrase,
                plaintextConfirmed: plaintextAcknowledgement == "CHANGE LOCAL DATA"
            )
            if model.didUpdate { didFinish() }
        }
    }
}

private struct ConnectionResolvedTransactionSheet: View {
    let daemon: any DaemonClient
    let reference: String
    let completed: () -> Void
    @State private var resolver: TransactionResolverViewModel

    init(daemon: any DaemonClient, reference: String, completed: @escaping () -> Void) {
        self.daemon = daemon
        self.reference = reference
        self.completed = completed
        _resolver = State(initialValue: TransactionResolverViewModel(daemon: daemon, reference: reference))
    }

    var body: some View {
        Group {
            if let transaction = resolver.transaction {
                TransactionDetailSheet(daemon: daemon, transaction: transaction, completed: completed)
            } else if let error = resolver.errorMessage {
                ContentUnavailableView(parityString("state.unavailable"), systemImage: "exclamationmark.triangle", description: Text(parityError(error)))
                    .frame(width: 600, height: 420)
            } else {
                ProgressView(parityString("state.loading")).frame(width: 600, height: 420)
            }
        }.task { await resolver.load() }
    }
}

struct ConnectionCatalogSheet: View {
    let sources: [ConnectionCatalogSource]
    let select: (ConnectionCatalogSource) -> Void
    @State private var category: ConnectionCatalogCategory = .wallets
    @State private var query = ""
    @Environment(\.dismiss) private var dismiss

    private var visibleSources: [ConnectionCatalogSource] {
        sources.filter { source in
            guard source.category == category else { return false }
            let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !needle.isEmpty else { return true }
            return parityString(source.titleLocalizationKey).localizedCaseInsensitiveContains(needle)
                || parityString(source.descriptionLocalizationKey).localizedCaseInsensitiveContains(needle)
                || source.formatLabel?.localizedCaseInsensitiveContains(needle) == true
        }
    }

    var body: some View {
        NavigationSplitView {
            List(ConnectionCatalogCategory.allCases, selection: $category) { item in
                Label(parityString("connections.catalog.category.\(item.rawValue)"), systemImage: icon(item))
                    .tag(item)
            }
            .navigationTitle(parityString("connections.catalog.categories"))
            .navigationSplitViewColumnWidth(min: 170, ideal: 190)
        } detail: {
            List(visibleSources) { source in
                Button {
                    select(source)
                } label: {
                    HStack(alignment: .top, spacing: 12) {
                        Image(systemName: icon(source.category))
                            .font(.title2)
                            .frame(width: 30)
                            .foregroundStyle(source.isEnabled ? Color.accentColor : Color.secondary)
                        VStack(alignment: .leading, spacing: 5) {
                            HStack {
                                Text(parityString(source.titleLocalizationKey)).font(.headline)
                                if source.status == .planned {
                                    Text(parityString("connections.catalog.planned"))
                                        .font(.caption2).padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(.quaternary, in: Capsule())
                                } else if !source.daemonSupported {
                                    Text(parityString("connections.catalog.unavailable"))
                                        .font(.caption2).padding(.horizontal, 6).padding(.vertical, 2)
                                        .background(.orange.opacity(0.15), in: Capsule())
                                }
                                Spacer()
                                if source.isEnabled { Image(systemName: "chevron.right").foregroundStyle(.tertiary) }
                            }
                            Text(parityString(source.descriptionLocalizationKey))
                                .font(.subheadline).foregroundStyle(.secondary)
                            HStack(spacing: 8) {
                                Text(parityString(source.pathLocalizationKey))
                                if let format = source.formatLabel { Text(format).monospaced() }
                            }
                            .font(.caption).foregroundStyle(.tertiary)
                        }
                    }
                    .padding(.vertical, 5)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(!source.isEnabled)
            }
            .navigationTitle(parityString("connections.add"))
            .searchable(text: $query, prompt: parityString("connections.catalog.search"))
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(parityString("action.cancel")) { dismiss() }
                }
            }
        }
        .frame(minWidth: 820, minHeight: 610)
    }

    private func icon(_ category: ConnectionCatalogCategory) -> String {
        switch category {
        case .wallets: "wallet.bifold"
        case .nodes: "server.rack"
        case .lightning: "bolt.fill"
        case .merchant: "cart"
        case .exchanges: "building.columns"
        case .files: "doc.badge.arrow.up"
        }
    }
}

enum ConnectionSetupMode: String, CaseIterable, Identifiable {
    case wallet, addressList, silentPayment, liquidWallet
    case backend, bitcoinCore, btcpay, bullbitcoin, samourai, bip329
    var id: String { rawValue }
    var localizationKey: String { "connections.setup.\(rawValue)" }
    var icon: String {
        switch self {
        case .wallet: "wallet.pass"
        case .addressList: "list.bullet.rectangle"
        case .silentPayment: "eye.slash"
        case .liquidWallet: "drop.fill"
        case .backend: "network"
        case .bitcoinCore: "server.rack"
        case .btcpay: "cart"
        case .bullbitcoin: "bitcoinsign.square"
        case .samourai: "person.crop.square"
        case .bip329: "tag"
        }
    }

    var walletSetupKind: WalletConnectionSetupKind? {
        switch self {
        case .wallet: .descriptor
        case .addressList: .addressList
        case .silentPayment: .silentPayment
        case .liquidWallet: .liquidDescriptor
        default: nil
        }
    }

    var specialSetupMode: SpecialConnectionSetupMode? {
        switch self {
        case .btcpay: .btcpay
        case .bullbitcoin: .bullBitcoin
        case .samourai: .samourai
        case .bip329: .bip329
        default: nil
        }
    }

    static func resolve(_ raw: String) -> Self? {
        if let exact = Self(rawValue: raw) { return exact }
        switch raw.lowercased() {
        case "descriptor", "xpub": return .wallet
        case "address-list", "address": return .addressList
        case "silent-payment", "bip352": return .silentPayment
        case "liquid-descriptor", "liquid": return .liquidWallet
        default: return nil
        }
    }
}

struct ConnectionSetupSheet: View {
    let mode: ConnectionSetupMode
    @Bindable var model: ConnectionsParityViewModel
    let daemon: any DaemonClient
    let didFinish: () -> Void
    @State private var backendModel: BackendSettingsViewModel
    @State private var walletModel: WalletConnectionSetupViewModel
    @State private var specialModel: SpecialConnectionSetupViewModel
    @State private var backend = BackendDraft()
    @Environment(\.dismiss) private var dismiss

    init(
        mode: ConnectionSetupMode,
        model: ConnectionsParityViewModel,
        daemon: any DaemonClient,
        backendKindOverride: String? = nil,
        didFinish: @escaping () -> Void
    ) {
        self.mode = mode; self.model = model; self.daemon = daemon; self.didFinish = didFinish
        _backendModel = State(initialValue: BackendSettingsViewModel(daemon: daemon))
        _walletModel = State(initialValue: WalletConnectionSetupViewModel(
            kind: mode.walletSetupKind ?? .descriptor,
            daemon: daemon
        ))
        _specialModel = State(initialValue: SpecialConnectionSetupViewModel(
            mode: mode.specialSetupMode ?? .btcpay,
            operations: model
        ))
        var seed = BackendDraft()
        if let backendKindOverride { seed.kind = backendKindOverride }
        if mode == .bitcoinCore || backendKindOverride == "bitcoinrpc" {
            seed.kind = "bitcoinrpc"
            seed.url = "http://127.0.0.1:8332"
        }
        _backend = State(initialValue: seed)
    }

    var body: some View {
        NavigationStack {
            Form {
                switch mode {
                case .wallet, .addressList, .silentPayment, .liquidWallet: walletFields
                case .backend, .bitcoinCore: backendFields
                case .btcpay: btcpayFields
                case .bullbitcoin: bullFields
                case .samourai: samouraiFields
                case .bip329: bip329Fields
                }
                if let result = model.resultMessage { Text(AppLocalization.code(result)).foregroundStyle(.secondary) }
                if let issue = walletModel.validationIssue, mode.walletSetupKind != nil {
                    Text(parityString("connections.validation.\(issue.rawValue)"))
                        .font(.caption).foregroundStyle(.secondary)
                }
                if let error = model.errorMessage ?? backendModel.errorMessage ?? walletModel.errorMessage ?? specialModel.errorMessage {
                    Text(parityError(error)).foregroundStyle(.red)
                }
            }.formStyle(.grouped).navigationTitle(parityString(mode.localizationKey))
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button(parityString("action.cancel")) { dismiss() } }
                    ToolbarItem(placement: .confirmationAction) {
                        Button(mode == .bip329 && !model.canImportBIP329(file: specialModel.sourceFile) ? parityString("action.preview") : parityString("action.save")) {
                            Task { await submit() }
                        }.disabled(submitDisabled)
                    }
                }
        }
        .frame(width: 690, height: 690)
        .task { walletModel.configure(backends: model.safeBackendOptions) }
        .onChange(of: walletModel.silentPaymentScanMode) { _, _ in
            walletModel.configure(backends: model.safeBackendOptions)
        }
    }

    @ViewBuilder private var walletFields: some View {
        TextField(parityString("field.name"), text: $walletModel.label)
        LabeledContent(parityString("field.network")) {
            Text(AppLocalization.code(walletModel.setupKind.network))
        }
        Picker(parityString("connections.backend"), selection: $walletModel.backend) {
            Text(parityString("connections.chooseBackend")).tag("")
            ForEach(walletModel.availableBackends) { row in
                Text([row.name, row.kind, row.network].filter { !$0.isEmpty }.joined(separator: " · ")).tag(row.name)
            }
        }
        if walletModel.availableBackends.isEmpty {
            Label(
                walletModel.setupKind == .silentPayment
                    ? parityString("connections.silentPayment.noBackend")
                    : walletModel.setupKind == .liquidDescriptor
                        ? parityString("connections.liquidDescriptor.noBackend")
                        : parityString("connections.wallet.noBackend"),
                systemImage: "exclamationmark.triangle"
            ).foregroundStyle(.orange)
        } else if !model.publicDefaultBackends.isEmpty && walletModel.setupKind == .descriptor {
            Text(parityString("connections.publicDefaults")).font(.caption).foregroundStyle(.secondary)
            ForEach(model.publicDefaultBackends) { row in
                LabeledContent(row.name, value: [row.kind, row.url].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption)
            }
        }
        switch walletModel.setupKind {
        case .descriptor, .liquidDescriptor:
            Text(parityString(walletModel.setupKind == .liquidDescriptor
                              ? "connections.liquidDescriptor.helper"
                              : "connections.descriptor.helper"))
                .font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $walletModel.walletMaterial)
                .font(.system(.body, design: .monospaced)).frame(minHeight: 120)
            if isBareWalletXpub {
                GroupBox(parityString("connections.scriptTypes")) {
                    ForEach(["p2wpkh", "p2sh-p2wpkh", "p2pkh", "p2tr"], id: \.self) { value in
                        Toggle(parityString("connections.scriptType.\(value)"), isOn: Binding(
                            get: { walletModel.scriptTypes.contains(value) },
                            set: { selected in
                                if selected { walletModel.scriptTypes.insert(value) }
                                else { walletModel.scriptTypes.remove(value) }
                            }
                        ))
                    }
                }
                Text(parityString("connections.bareXpubScriptHint")).font(.caption).foregroundStyle(.secondary)
            }
            Stepper("\(parityString("wallet.gapLimit")): \(walletModel.gapLimit)", value: $walletModel.gapLimit, in: 1...5_000)
            TextField(parityString("connections.walletBirthday"), text: $walletModel.birthday)
            Button(parityString("wallet.detectPreview")) { Task { await walletModel.detectAndPreview() } }
                .disabled(walletModel.walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || walletModel.isWorking)
            ForEach(walletModel.preview.prefix(5)) { row in
                Text(row.address).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
            }
            Toggle(parityString("connections.syncAfterCreate"), isOn: $walletModel.syncAfterCreate)
        case .addressList:
            Text(parityString("connections.addressList.helper")).font(.caption).foregroundStyle(.secondary)
            TextEditor(text: Binding(
                get: { walletModel.addressInput },
                set: { walletModel.setAddressInput($0) }
            )).font(.system(.body, design: .monospaced)).frame(minHeight: 150)
            HStack {
                Button(parityString("connections.addressList.loadFile")) { loadAddressFile() }
                Spacer()
                Text(String(format: parityString("connections.addressList.summary %lld %lld %lld"),
                            Int64(walletModel.addressSummary.valid.count),
                            Int64(walletModel.addressSummary.duplicates),
                            Int64(walletModel.addressSummary.invalid.count)))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if walletModel.purgedPrivateKeys + walletModel.purgedPublicKeys > 0 {
                Label(
                    String(format: parityString("connections.addressList.keysPurged %lld %lld"),
                           Int64(walletModel.purgedPrivateKeys), Int64(walletModel.purgedPublicKeys)),
                    systemImage: "key.slash"
                ).font(.caption).foregroundStyle(.red)
            }
            if !walletModel.addressSummary.invalid.isEmpty {
                Text(String(format: parityString("connections.addressList.invalid %@"),
                            walletModel.addressSummary.invalid.prefix(3).joined(separator: ", ")))
                    .font(.caption).foregroundStyle(.orange)
            }
            Toggle(parityString("connections.syncAfterCreate"), isOn: $walletModel.syncAfterCreate)
        case .silentPayment:
            Text(parityString("connections.silentPayment.backendHelper"))
                .font(.caption).foregroundStyle(.secondary)
            TextEditor(text: $walletModel.silentPaymentMaterial)
                .font(.system(.body, design: .monospaced)).frame(minHeight: 100)
            Text(parityString("connections.silentPayment.materialHelper"))
                .font(.caption).foregroundStyle(.secondary)
            Picker(parityString("connections.silentPayment.scanMode"), selection: $walletModel.silentPaymentScanMode) {
                Text(parityString("connections.silentPayment.localIndex")).tag(SilentPaymentScanMode.localIndex)
                Text(parityString("connections.silentPayment.serverAssisted")).tag(SilentPaymentScanMode.serverAssisted)
            }.pickerStyle(.segmented)
            if !walletModel.silentPaymentFullHistory {
                TextField(parityString("connections.silentPayment.startHeight"), text: $walletModel.silentPaymentStartHeight)
                TextField(parityString("connections.silentPayment.startDate"), text: $walletModel.silentPaymentStartDate)
            }
            Toggle(parityString("connections.silentPayment.fullHistory"), isOn: $walletModel.silentPaymentFullHistory)
            if walletModel.silentPaymentFullHistory {
                Toggle(parityString("connections.silentPayment.fullHistoryAck"), isOn: $walletModel.acknowledgeFullHistory)
            }
            if walletModel.silentPaymentScanMode == .serverAssisted {
                Toggle(parityString("connections.silentPayment.serverAck"), isOn: $walletModel.acknowledgeServerPrivacy)
            }
            Text(parityString("connections.silentPayment.syncRequired"))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var backendFields: some View {
        TextField(parityString("field.name"), text: $backend.name)
        Picker(parityString("field.type"), selection: $backend.kind) {
            ForEach(["esplora", "electrum", "bitcoinrpc", "liquid-esplora", "lnd", "coreln", "nwc"], id: \.self, content: Text.init)
        }
        TextField(AppLocalization.code("url"), text: $backend.url)
        Picker(parityString("connections.chain"), selection: $backend.chain) {
            Text(AppLocalization.code("bitcoin")).tag("bitcoin")
            Text(AppLocalization.code("liquid")).tag("liquid")
        }
        Picker(parityString("field.network"), selection: $backend.network) {
            ForEach(["main", "testnet", "regtest", "liquidv1"], id: \.self) { Text(AppLocalization.code($0)).tag($0) }
        }
        TextField(parityString("connections.proxyOptional"), text: $backend.proxy)
        Stepper("\(parityString("connections.timeout")): \(backend.timeout)s", value: $backend.timeout, in: 1...120)
        TextField(parityString("connections.notesOptional"), text: $backend.notes, axis: .vertical).lineLimit(2...4)
        if backend.kind == "bitcoinrpc" {
            HStack {
                TextField(parityString("connections.cookieFile"), text: $backend.cookieFile)
                Button(parityString("action.browse")) {
                    if let url = NativeAffordances.chooseFile(types: [.data]) { backend.cookieFile = url.path }
                }
            }
            TextField(parityString("connections.rpcUser"), text: $backend.username)
            SecureField(parityString("connections.rpcPassword"), text: $backend.password)
            Button(parityString("connections.detectCore")) { Task { await backendModel.detectBitcoinCore() } }
        }
        if ["electrum", "fulcrum"].contains(backend.kind) {
            Toggle(parityString("connections.trustSelfSigned"), isOn: $backend.trustSelfSigned)
            if !backend.trustSelfSigned {
                TextField(parityString("connections.certificateOptional"), text: $backend.certificate)
            }
        }
        if backend.kind == "lnd" {
            SecureField(parityString("connections.macaroon"), text: $backend.token)
            Toggle(parityString("connections.trustSelfSigned"), isOn: $backend.trustSelfSigned)
            if !backend.trustSelfSigned {
                TextField(parityString("connections.certificateOptional"), text: $backend.certificate)
            }
        }
        if ["coreln", "core-lightning", "cln"].contains(backend.kind) {
            TextField(parityString("connections.commandoPeerID"), text: $backend.commandoPeerID)
            SecureField(parityString("connections.commandoRune"), text: $backend.token)
            TextField(parityString("connections.lightningCLI"), text: $backend.lightningCLI)
            TextField(parityString("connections.lightningDirectory"), text: $backend.lightningDirectory)
            TextField(parityString("connections.rpcFile"), text: $backend.rpcFile)
        }
        if backend.kind == "nwc" {
            SecureField(parityString("connections.nwcSecret"), text: $backend.token)
        }
        if ["esplora", "liquid-esplora"].contains(backend.kind) {
            SecureField(parityString("connections.authHeaderOptional"), text: $backend.authHeader)
            SecureField(parityString("connections.tokenOptional"), text: $backend.token)
        }
        if backend.chain == "bitcoin", ["esplora", "electrum", "bitcoinrpc"].contains(backend.kind) {
            Toggle(parityString("connections.silentPayments"), isOn: $backend.silentPayments)
        }
        HStack {
            Button(parityString("connections.test")) { Task { await backendModel.test(backend) } }
            if let probe = backendModel.probe { Label(probe.headline, systemImage: probe.ok ? "checkmark.circle.fill" : "xmark.circle.fill").foregroundStyle(probe.ok ? .green : .red) }
        }
    }

    @ViewBuilder private var btcpayFields: some View {
        TextField(parityString("field.name"), text: $specialModel.label)
        Picker(parityString("connections.savedBackend"), selection: $specialModel.selectedBackend) {
            Text(parityString("connections.newInstance")).tag("")
            ForEach(model.backends.filter { $0.kind == "btcpay" }) { Text($0.name).tag($0.name) }
        }
        if specialModel.selectedBackend.isEmpty {
            TextField(AppLocalization.code("url"), text: $specialModel.serverURL)
            SecureField(parityString("settings.apiKeyOptional"), text: $specialModel.apiKey)
        }
        Button(parityString("connections.discover")) { Task { await specialModel.discoverBTCPay() } }
        if !model.btcpayStores.isEmpty {
            Picker(parityString("connections.store"), selection: $specialModel.storeID) { ForEach(model.btcpayStores) { Text($0.name).tag($0.id) } }
            Picker(parityString("connections.setupMode"), selection: $specialModel.btcpayExistingWallets) {
                Text(parityString("connections.walletSources")).tag(false)
                Text(parityString("connections.mapExistingWallets")).tag(true)
            }.pickerStyle(.segmented)
            ForEach(availableBTCPayMethods) { method in
                Toggle(isOn: btcpayMethodBinding(method.paymentMethodID)) {
                    VStack(alignment: .leading) {
                        Text(method.label)
                        Text(method.paymentMethodID).font(.caption.monospaced()).foregroundStyle(.secondary)
                    }
                }.disabled(!method.enabled || !method.syncSupported)
                if specialModel.btcpayExistingWallets && specialModel.paymentMethods.contains(method.paymentMethodID) {
                    Picker(parityString("connections.settlementWallet"), selection: btcpayRouteBinding(method.paymentMethodID)) {
                        Text(parityString("connections.chooseWallet")).tag("")
                        ForEach(model.wallets) { Text($0.label).tag($0.id) }
                    }
                }
            }
            Toggle(parityString("connections.syncProvenance"), isOn: $specialModel.btcpaySyncProvenance)
            Button(parityString("connections.test")) {
                Task { await specialModel.testBTCPay() }
            }.disabled(specialModel.storeID.isEmpty || selectedBTCPayMethodIDs.isEmpty)
        }
    }

    @ViewBuilder private var bullFields: some View {
        TextField(parityString("field.name"), text: $specialModel.label)
        HStack {
            TextField(parityString("connections.sourceFile"), text: $specialModel.sourceFile)
            Button(parityString("action.browse")) {
                if let url = NativeAffordances.chooseFile(types: [.commaSeparatedText, .json, .data]) { specialModel.sourceFile = url.path }
            }
        }
        Picker(parityString("connections.setupMode"), selection: $specialModel.bullExistingWallets) {
            Text(parityString("connections.walletSources")).tag(false)
            Text(parityString("connections.mapExistingWallets")).tag(true)
        }.pickerStyle(.segmented)
        ForEach(["bitcoin", "liquid", "lightning"], id: \.self) { value in
            Toggle(AppLocalization.code(value), isOn: Binding(
                get: { specialModel.networks.contains(value) },
                set: { specialModel.setBullNetwork(value, selected: $0) }
            ))
            if specialModel.bullExistingWallets && specialModel.networks.contains(value) {
                Picker(parityString("connections.settlementWallet"), selection: bullRouteBinding(value)) {
                    Text(parityString("connections.chooseWallet")).tag("")
                    ForEach(model.wallets) { Text($0.label).tag($0.id) }
                }
            }
        }
    }

    @ViewBuilder private var samouraiFields: some View {
        TextField(parityString("field.name"), text: $specialModel.label)
        Picker(parityString("connections.savedBackend"), selection: $specialModel.selectedBackend) { Text("—").tag(""); ForEach(model.backends) { Text($0.name).tag($0.name) } }
        Picker(parityString("field.network"), selection: $specialModel.network) {
            Text(AppLocalization.code("main")).tag("main")
            Text(AppLocalization.code("testnet")).tag("testnet")
        }
        Stepper("\(parityString("wallet.gapLimit")): \(specialModel.gapLimit)", value: $specialModel.gapLimit, in: 1...5_000)
        TextField(parityString("importsParity.samouraiDeposit"), text: $specialModel.deposit)
        TextField(parityString("importsParity.samouraiBadbank"), text: $specialModel.badbank)
        TextField(parityString("connections.samouraiPremix"), text: $specialModel.premix)
        TextField(parityString("connections.samouraiPostmix"), text: $specialModel.postmix)
        TextField(parityString("connections.samouraiRicochet"), text: $specialModel.ricochet)
        Text(parityString("connections.samouraiPublicOnly")).font(.caption).foregroundStyle(.secondary)
    }

    @ViewBuilder private var bip329Fields: some View {
        HStack {
            TextField(parityString("connections.labelFile"), text: $specialModel.sourceFile)
            Button(parityString("action.browse")) {
                if let url = NativeAffordances.chooseFile(types: [.json, .plainText]) { specialModel.sourceFile = url.path }
            }
        }
        if let preview = model.bip329Preview {
            LabeledContent(parityString("connections.records"), value: String(preview.records))
            LabeledContent(parityString("connections.exact"), value: String(preview.exact))
            LabeledContent(parityString("connections.ambiguous"), value: String(preview.ambiguous))
            LabeledContent(parityString("connections.unmatched"), value: String(preview.unmatched))
            LabeledContent(parityString("connections.conflicts"), value: String(preview.conflicts))
        }
        Divider()
        Picker(parityString("connections.exportMode"), selection: $specialModel.bip329ExportMode) {
            Text(parityString("connections.exportAll")).tag("all")
            Text(parityString("connections.exportStored")).tag("stored")
        }
        Picker(parityString("field.wallet"), selection: $specialModel.bip329ExportWallet) {
            Text(parityString("connections.allWallets")).tag("")
            ForEach(model.wallets) { Text($0.label).tag($0.id) }
        }
        Button(parityString("connections.exportLabels")) {
            Task {
                await specialModel.exportBIP329()
                guard let artifact = model.artifact, !artifact.sourcePath.isEmpty else { return }
                _ = NativeAffordances.saveCopy(
                    source: URL(fileURLWithPath: artifact.sourcePath),
                    suggestedFilename: artifact.filename,
                    types: [.json, .plainText],
                    title: parityString("connections.exportLabels")
                )
            }
        }
        if let artifact = model.artifact {
            LabeledContent(parityString("connections.exportedFile"), value: artifact.sourcePath)
                .font(.caption).textSelection(.enabled)
        }
    }

    private func submit() async {
        switch mode {
        case .wallet, .addressList, .silentPayment, .liquidWallet:
            await walletModel.create()
            if walletModel.didSave { didFinish() }
        case .backend, .bitcoinCore:
            await backendModel.save(backend)
            if backendModel.errorMessage == nil { didFinish() }
        case .btcpay, .bullbitcoin, .samourai, .bip329:
            if await specialModel.submit() { didFinish() }
        }
    }

    private var isBareWalletXpub: Bool {
        let material = walletModel.walletMaterial.trimmingCharacters(in: .whitespacesAndNewlines)
        return material.hasPrefix("xpub") || material.hasPrefix("tpub")
    }

    private func loadAddressFile() {
        guard let url = NativeAffordances.chooseFile(types: [.plainText, .commaSeparatedText, .data]) else { return }
        do {
            walletModel.appendAddressFile(try String(contentsOf: url, encoding: .utf8))
        } catch {
            walletModel.reportAddressFileReadFailure()
        }
    }

    private var availableBTCPayMethods: [BTCPayPaymentMethodRow] {
        specialModel.availableBTCPayMethods
    }

    private var selectedBTCPayMethodIDs: [String] {
        specialModel.selectedBTCPayMethodIDs
    }

    private func btcpayMethodBinding(_ method: String) -> Binding<Bool> {
        Binding(
            get: { specialModel.paymentMethods.contains(method) },
            set: { specialModel.setBTCPayMethod(method, selected: $0) }
        )
    }

    private func btcpayRouteBinding(_ method: String) -> Binding<String> {
        Binding(
            get: { specialModel.btcpayWalletRoutes[method] ?? "" },
            set: { specialModel.btcpayWalletRoutes[method] = $0 }
        )
    }

    private func bullRouteBinding(_ network: String) -> Binding<String> {
        Binding(
            get: { specialModel.bullWalletRoutes[network] ?? "" },
            set: { specialModel.bullWalletRoutes[network] = $0 }
        )
    }

    private var submitDisabled: Bool {
        if model.isWorking || backendModel.isWorking || walletModel.isWorking || specialModel.isWorking { return true }
        switch mode {
        case .wallet, .addressList, .silentPayment, .liquidWallet:
            return !walletModel.canSubmit
        case .btcpay, .bullbitcoin, .samourai, .bip329:
            return !specialModel.canSubmit
        case .backend, .bitcoinCore:
            return false
        }
    }
}

private extension String { var nilIfEmpty: String? { isEmpty ? nil : self } }

private enum NativeSettingsSection: String, CaseIterable, Identifiable {
    case general, security, privacy, terminal, bitcoin, lightning, liquid, market, assistant, automation, replication, data
    var id: String { rawValue }
    var key: String { "settings.parity.\(rawValue)" }
    var icon: String {
        switch self {
        case .general: "gearshape"
        case .security: "lock.shield"
        case .privacy: "eye.slash"
        case .terminal: "terminal"
        case .bitcoin: "bitcoinsign.circle"
        case .lightning: "bolt"
        case .liquid: "drop"
        case .market: "chart.line.uptrend.xyaxis"
        case .assistant: "sparkles"
        case .automation: "arrow.triangle.2.circlepath"
        case .replication: "person.2.wave.2"
        case .data: "externaldrive.badge.exclamationmark"
        }
    }
}

struct FullLayeredSettingsScreen: View {
    let daemon: any DaemonClient
    @AppStorage(KassiberHostNotification.settingsSectionDefaultsKey) private var selectedSectionRaw = NativeSettingsSection.general.rawValue
    @State private var backendModel: BackendSettingsViewModel
    @State private var providerModel: AIProviderSettingsViewModel
    @State private var maintenanceModel: MaintenanceSettingsViewModel
    @State private var securityModel: SecuritySettingsViewModel
    @State private var ratesModel: RatesSettingsViewModel
    @State private var replicationModel: ReplicationSettingsViewModel
    @State private var destructiveModel: DestructiveSettingsViewModel
    @State private var historyModel: ChatHistorySettingsViewModel
    @State private var privacyModel: PrivacyHygieneSettingsViewModel
    @State private var nativePlatformModel: NativePlatformSettingsViewModel
    @State private var backendEditor: BackendEditorState?
    @State private var providerEditor: ProviderEditorState?
    @State private var onboardingOpen = false

    init(daemon: any DaemonClient) {
        self.daemon = daemon
        _backendModel = State(initialValue: BackendSettingsViewModel(daemon: daemon))
        _providerModel = State(initialValue: AIProviderSettingsViewModel(daemon: daemon))
        _maintenanceModel = State(initialValue: MaintenanceSettingsViewModel(daemon: daemon))
        _securityModel = State(initialValue: SecuritySettingsViewModel(
            daemon: daemon,
            touchIDManager: MacOSNativeServices.touchID,
            touchIDAccount: canonicalTouchIDAccount(for: nil)
        ))
        _ratesModel = State(initialValue: RatesSettingsViewModel(daemon: daemon))
        _replicationModel = State(initialValue: ReplicationSettingsViewModel(daemon: daemon))
        _destructiveModel = State(initialValue: DestructiveSettingsViewModel(daemon: daemon))
        _historyModel = State(initialValue: ChatHistorySettingsViewModel(daemon: daemon))
        _privacyModel = State(initialValue: PrivacyHygieneSettingsViewModel(daemon: daemon))
        _nativePlatformModel = State(initialValue: NativePlatformSettingsViewModel())
    }

    var body: some View {
        HSplitView {
            List(selection: selectedSectionBinding) {
                Section(parityString("settings.group.application")) { settingsRows([.general, .security, .privacy, .terminal]) }
                Section(parityString("settings.group.networks")) { settingsRows([.bitcoin, .lightning, .liquid, .market]) }
                Section(parityString("settings.group.intelligence")) { settingsRows([.assistant, .automation]) }
                Section(parityString("settings.group.collaboration")) { settingsRows([.replication]) }
                Section { settingsRows([.data]) }
            }.frame(minWidth: 215, idealWidth: 235, maxWidth: 280)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Label(parityString(selected.key), systemImage: selected.icon).font(.title.bold())
                    settingsContent
                }.padding(24).frame(maxWidth: 850, alignment: .leading)
            }
        }
        .navigationTitle(parityString("nav.settings"))
        .sheet(item: $backendEditor) { state in
            BackendEditorSheet(model: backendModel, state: state) { backendEditor = nil }
        }
        .sheet(item: $providerEditor) { state in
            ProviderEditorSheet(model: providerModel, state: state) { providerEditor = nil }
        }
        .sheet(isPresented: $onboardingOpen) { FullOnboardingScreen(daemon: daemon) { onboardingOpen = false } }
        .task {
            async let backends: Void = backendModel.load()
            async let providers: Void = providerModel.load()
            async let maintenance: Void = maintenanceModel.load()
            async let security: Void = securityModel.loadStatus()
            async let rates: Void = ratesModel.load()
            async let replication: Void = replicationModel.load()
            async let history: Void = historyModel.load()
            async let privacy: Void = privacyModel.load()
            _ = await (backends, providers, maintenance, security, rates, replication, history, privacy)
            nativePlatformModel.loadTerminalStatus()
        }
    }

    @ViewBuilder private func settingsRows(_ sections: [NativeSettingsSection]) -> some View {
        ForEach(sections) { section in Label(parityString(section.key), systemImage: section.icon).tag(section) }
    }

    private var selected: NativeSettingsSection {
        NativeSettingsSection(rawValue: selectedSectionRaw) ?? .general
    }

    private var selectedSectionBinding: Binding<NativeSettingsSection> {
        Binding(
            get: { selected },
            set: { selectedSectionRaw = $0.rawValue }
        )
    }

    @ViewBuilder private var settingsContent: some View {
        switch selected {
        case .general: GeneralParitySettings(onboardingOpen: $onboardingOpen)
        case .security: SecurityParitySettings(model: securityModel)
        case .privacy: PrivacyHygieneParitySettings(model: privacyModel)
        case .terminal: TerminalCommandParitySettings(model: nativePlatformModel)
        case .bitcoin: BackendLayerSettings(model: backendModel, layer: .base, editor: $backendEditor)
        case .lightning: BackendLayerSettings(model: backendModel, layer: .lightning, editor: $backendEditor)
        case .liquid: BackendLayerSettings(model: backendModel, layer: .liquid, editor: $backendEditor)
        case .market: MarketParitySettings(model: ratesModel)
        case .assistant: AIProviderParitySettings(model: providerModel, history: historyModel, editor: $providerEditor)
        case .automation: MaintenanceParitySettings(model: maintenanceModel)
        case .replication: ReplicationParitySettings(model: replicationModel)
        case .data: DataParitySettings(model: destructiveModel)
        }
    }
}

private struct GeneralParitySettings: View {
    @Binding var onboardingOpen: Bool
    @AppStorage("language") private var language = "en"
    @AppStorage("developerToolsEnabled") private var developerToolsEnabled = true
    @AppStorage("appearance.theme") private var appearanceTheme = "system"
    @AppStorage("appearance.currency") private var appearanceCurrency = "btc"
    @AppStorage("appearance.scale") private var appearanceScale = 0.9
    @AppStorage("aiFeaturesEnabled") private var aiFeaturesEnabled = true
    @AppStorage("assistant.panel.autoHide") private var assistantPanelAutoHide = true
    @AppStorage("assistant.panel.position") private var assistantPanelPosition = "right"
    @AppStorage("assistant.panel.startMinimized") private var assistantPanelStartMinimized = false
    var body: some View {
        parityCard(parityString("appearance.theme.title")) {
            Text(parityString("appearance.theme.description")).font(.caption).foregroundStyle(.secondary)
            Picker(parityString("appearance.theme.title"), selection: $appearanceTheme) {
                Label(parityString("appearance.theme.system"), systemImage: "display").tag("system")
                Label(parityString("appearance.theme.light"), systemImage: "sun.max").tag("light")
                Label(parityString("appearance.theme.dark"), systemImage: "moon").tag("dark")
            }.pickerStyle(.segmented)
        }
        parityCard(parityString("appearance.denomination.title")) {
            Text(parityString("appearance.denomination.description")).font(.caption).foregroundStyle(.secondary)
            Picker(parityString("appearance.denomination.title"), selection: $appearanceCurrency) {
                Text(parityString("appearance.denomination.euro")).tag("eur")
                Text(parityString("appearance.denomination.bitcoin")).tag("btc")
            }.pickerStyle(.segmented)
        }
        parityCard(parityString("appearance.scale.title")) {
            Text(parityString("appearance.scale.description")).font(.caption).foregroundStyle(.secondary)
            HStack {
                Button { appearanceScale = max(0.8, appearanceScale - 0.05) } label: { Image(systemName: "minus") }
                    .disabled(appearanceScale <= 0.8)
                Text("\(Int((appearanceScale * 100).rounded()))%")
                    .font(.body.monospacedDigit()).frame(minWidth: 54)
                Button { appearanceScale = min(1.2, appearanceScale + 0.05) } label: { Image(systemName: "plus") }
                    .disabled(appearanceScale >= 1.2)
                Button(parityString("action.reset")) { appearanceScale = 0.9 }
                    .disabled(abs(appearanceScale - 0.9) < 0.001)
            }
        }
        if aiFeaturesEnabled {
            parityCard(parityString("assistantPanel.settingsTitle")) {
                Text(parityString("assistantPanel.nativeAdaptation"))
                    .font(.caption).foregroundStyle(.secondary)
                Toggle(parityString("assistantPanel.autoHide"), isOn: $assistantPanelAutoHide)
                Toggle(parityString("assistantPanel.startMinimized"), isOn: $assistantPanelStartMinimized)
                Picker(parityString("assistantPanel.position"), selection: $assistantPanelPosition) {
                    Label(parityString("assistantPanel.positionLeft"), systemImage: "sidebar.left").tag("left")
                    Label(parityString("assistantPanel.positionCenter"), systemImage: "rectangle.bottomhalf.inset.filled").tag("center")
                    Label(parityString("assistantPanel.positionRight"), systemImage: "sidebar.right").tag("right")
                }.pickerStyle(.segmented)
            }
        }
        parityCard(parityString("settings.language")) {
            Picker(parityString("settings.language"), selection: $language) {
                Text(parityString("language.english")).tag("en")
                Text(parityString("language.germanAT")).tag("de-AT")
            }
            Text(parityString("settings.languageRestart")).font(.caption).foregroundStyle(.secondary)
        }
        parityCard(parityString("settings.setupAssistant")) {
            Text(parityString("settings.setupAssistantHint")).foregroundStyle(.secondary)
            Button(parityString("settings.openSetupAssistant")) { onboardingOpen = true }
        }
        parityCard(parityString("settings.developerTools")) {
            Toggle(parityString("settings.developerToolsEnabled"), isOn: $developerToolsEnabled)
            Text(parityString("settings.developerToolsHint")).font(.caption).foregroundStyle(.secondary)
        }
    }
}

private struct SecurityParitySettings: View {
    @Bindable var model: SecuritySettingsViewModel
    @State private var current = ""
    @State private var newPassphrase = ""
    @State private var confirmation = ""
    @State private var migrate = true
    @State private var touchIDPassphrase = ""
    @AppStorage("security.autoLockWhenIdle") private var autoLockWhenIdle = false
    @AppStorage("security.idleMinutes") private var idleMinutes = 5
    @AppStorage("security.requirePassphraseOnLaunch") private var requirePassphraseOnLaunch = false
    @AppStorage("security.lockOnWindowClose") private var lockOnWindowClose = false
    @AppStorage("security.lockOnScreenLock") private var lockOnScreenLock = true
    @AppStorage("security.touchIDUnlock") private var touchIDUnlock = false
    var body: some View {
        parityCard(parityString("security.database")) {
            Label(model.isEncrypted ? parityString("security.encrypted") : parityString("security.plaintext"), systemImage: model.isEncrypted ? "lock.fill" : "lock.open")
            if !model.isEncrypted {
                SecureField(parityString("security.newPassphrase"), text: $newPassphrase)
                SecureField(parityString("security.confirmPassphrase"), text: $confirmation)
                Toggle(parityString("security.migrateCredentials"), isOn: $migrate)
                Button(parityString("security.enableEncryption")) { Task { await model.initializeEncryption(passphrase: newPassphrase, migrateCredentials: migrate) } }
                    .disabled(newPassphrase.count < 10 || newPassphrase != confirmation || model.isWorking)
            } else {
                SecureField(parityString("security.currentPassphrase"), text: $current)
                SecureField(parityString("security.newPassphrase"), text: $newPassphrase)
                SecureField(parityString("security.confirmPassphrase"), text: $confirmation)
                Button(parityString("security.changePassphrase")) { Task { await model.changePassphrase(current: current, new: newPassphrase) } }
                    .disabled(current.isEmpty || newPassphrase.count < 10 || newPassphrase != confirmation || model.isWorking)
            }
            if let path = model.backupPath { Text(path).font(.caption.monospaced()).textSelection(.enabled) }
        }
        parityCard(parityString("security.session")) {
            HStack {
                Button(parityString("security.lockNow")) { Task { await model.lock() } }
                SecureField(parityString("security.passphrase"), text: $current).frame(maxWidth: 280)
                Button(parityString("security.unlock")) { Task { await model.unlock(passphrase: current) } }.disabled(current.isEmpty)
            }
        }
        parityCard(parityString("security.appLockHeading")) {
            Toggle(parityString("security.autoLockLabel"), isOn: $autoLockWhenIdle)
            if autoLockWhenIdle {
                Picker(parityString("security.lockAfter"), selection: $idleMinutes) {
                    ForEach([1, 5, 15, 30, 60], id: \.self) { minutes in
                        Text("\(minutes) \(parityString("security.minutes"))").tag(minutes)
                    }
                }.pickerStyle(.segmented)
            }
            Toggle(parityString("security.requireOnLaunchLabel"), isOn: Binding(
                get: { model.isEncrypted || requirePassphraseOnLaunch },
                set: { enabled in
                    if !model.isEncrypted { requirePassphraseOnLaunch = enabled }
                }
            ))
            .disabled(model.isEncrypted)
            if model.isEncrypted {
                Text(parityString("security.requireOnLaunchEncryptedHint"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Toggle(parityString("security.lockOnCloseLabel"), isOn: $lockOnWindowClose)
            Toggle(parityString("security.lockOnScreenLockLabel"), isOn: $lockOnScreenLock)
        }
        .onAppear {
            if model.isEncrypted { requirePassphraseOnLaunch = true }
        }
        parityCard(parityString("security.biometricHeading")) {
            Label(
                model.touchIDStatus.configured
                    ? parityString("security.touchIDConfigured")
                    : model.touchIDStatus.available
                        ? parityString("security.touchIDAvailable")
                        : parityString("security.touchIDUnavailable"),
                systemImage: "touchid"
            )
            if let reason = model.touchIDStatus.reason {
                Text(reason).font(.caption).foregroundStyle(.secondary)
            }
            if let warning = model.touchIDWarning {
                Label(warning, systemImage: "exclamationmark.triangle")
                    .font(.caption).foregroundStyle(.orange)
            }
            if model.touchIDStatus.configured {
                Toggle(parityString("security.offerBiometricLabel"), isOn: $touchIDUnlock)
                Button(parityString("security.forget"), role: .destructive) {
                    Task { await model.forgetTouchID(); touchIDUnlock = false }
                }
            } else if model.touchIDStatus.available && model.isEncrypted {
                SecureField(parityString("security.currentPassphrase"), text: $touchIDPassphrase)
                    .frame(maxWidth: 320)
                Button(parityString("security.setUp")) {
                    Task {
                        await model.enrollTouchID(passphrase: touchIDPassphrase)
                        if model.touchIDStatus.configured { touchIDUnlock = true; touchIDPassphrase = "" }
                    }
                }.disabled(touchIDPassphrase.isEmpty || model.isWorking)
            }
            Button(parityString("action.refresh")) { Task { await model.refreshTouchIDStatus() } }
        }
        parityStatus(model.errorMessage, model.resultMessage)
    }
}

private struct TerminalCommandParitySettings: View {
    @Bindable var model: NativePlatformSettingsViewModel
    var body: some View {
        parityCard(parityString("terminal.heading")) {
            Text(parityString("terminal.intro")).foregroundStyle(.secondary)
            if let status = model.terminalStatus {
                Label(statusText(status), systemImage: status.conflict ? "exclamationmark.triangle" : status.installed ? "checkmark.circle" : "terminal")
                LabeledContent(parityString("terminal.commandLabel"), value: status.commandURL.path)
                LabeledContent(parityString("terminal.targetLabel"), value: status.targetURL.path)
                if !status.pathOnPath {
                    LabeledContent(parityString("terminal.pathLabel")) {
                        Text(status.pathHint).font(.caption.monospaced()).textSelection(.enabled)
                    }
                }
                HStack {
                    if !status.conflict {
                        Button(status.needsRepair ? parityString("terminal.repairAction") : parityString("terminal.installAction")) {
                            model.installTerminalCommand()
                        }.disabled(status.installed && !status.needsRepair || model.isWorking)
                    }
                    if status.managed {
                        Button(parityString("terminal.removeAction"), role: .destructive) {
                            model.removeTerminalCommand()
                        }.disabled(model.isWorking)
                    }
                    Button(parityString("action.refresh")) { model.loadTerminalStatus() }
                }
            } else {
                ProgressView()
            }
            if let error = model.terminalError { Text(parityError(error)).foregroundStyle(.red) }
        }
    }

    private func statusText(_ status: TerminalCommandStatus) -> String {
        if status.conflict { return parityString("terminal.conflictState") }
        if status.needsRepair { return parityString("terminal.repairState") }
        if status.installed && status.pathOnPath { return parityString("terminal.installedOnPathState") }
        if status.installed { return parityString("terminal.installedOffPathState") }
        return parityString("terminal.missingState")
    }
}

private struct PrivacyHygieneParitySettings: View {
    @Bindable var model: PrivacyHygieneSettingsViewModel
    @AppStorage("hideSensitive") private var hideSensitive = false
    @AppStorage("clearClipboard") private var clearClipboard = true
    var body: some View {
        parityCard(parityString("settings.onScreenPrivacy")) {
            Toggle(parityString("settings.blurSensitive"), isOn: $hideSensitive)
            Text(hideSensitive ? parityString("settings.blurSensitiveOn") : parityString("settings.blurSensitiveOff"))
                .font(.caption).foregroundStyle(.secondary)
            Toggle(parityString("settings.clearClipboard"), isOn: $clearClipboard)
            Text(clearClipboard ? parityString("settings.clearClipboardOn") : parityString("settings.clearClipboardOff"))
                .font(.caption).foregroundStyle(.secondary)
        }
        parityCard(parityString("settings.privacyHygiene")) {
            HStack {
                Label(AppLocalization.code(model.riskLevel.isEmpty ? model.state : model.riskLevel), systemImage: "eye.slash")
                Spacer()
                LabeledContent(parityString("settings.riskFindings"), value: String(model.riskCount))
                LabeledContent(parityString("settings.unknownFindings"), value: String(model.unknownCount))
            }
            ProgressView(value: model.totalTransactions > 0 ? Double(model.scoredTransactions) / Double(model.totalTransactions) : 0)
            Text("\(model.scoredTransactions) / \(model.totalTransactions) \(parityString("settings.transactionsScored"))")
                .font(.caption).foregroundStyle(.secondary)
            Text(parityString("settings.privacyLocalOnly")).font(.caption).foregroundStyle(.secondary)
        }
        ForEach(model.wallets) { wallet in
            parityCard(wallet.label) {
                HStack {
                    Text(AppLocalization.code(wallet.state)); Text(AppLocalization.code(wallet.riskLevel)).foregroundStyle(.secondary); Spacer()
                    LabeledContent(parityString("settings.risks"), value: String(wallet.riskCount))
                    LabeledContent(parityString("settings.reusedAddresses"), value: String(wallet.reusedAddresses))
                }
            }
        }
        if let error = model.errorMessage { Text(parityError(error)).foregroundStyle(.red) }
        Button(parityString("action.refresh")) { Task { await model.load() } }.disabled(model.isLoading)
    }
}

private struct BackendLayerSettings: View {
    @Bindable var model: BackendSettingsViewModel
    let layer: ConnectionLayer
    @Binding var editor: BackendEditorState?
    var rows: [SettingsBackendRow] { model.backends.filter { endpointLayer($0) == layer } }
    var body: some View {
        HStack { Text(parityString("settings.backendPrivacyHint")).foregroundStyle(.secondary); Spacer(); Button(parityString("settings.addBackend")) { editor = BackendEditorState() } }
        ForEach(rows) { row in
            parityCard(row.name) {
                HStack {
                    VStack(alignment: .leading) {
                        Text([row.kind, row.network].filter { !$0.isEmpty }.map(AppLocalization.code).joined(separator: " · "))
                        if !row.url.isEmpty { Text(row.url).font(.caption.monospaced()).foregroundStyle(.secondary) }
                        if !row.walletReferences.isEmpty { Text(row.walletReferences.joined(separator: ", ")).font(.caption).foregroundStyle(.secondary) }
                    }
                    Spacer()
                    if row.isDefault { Text(parityString("settings.default")).font(.caption).padding(5).background(.quaternary, in: Capsule()) }
                    Button { editor = BackendEditorState(row: row) } label: { Image(systemName: "pencil") }
                    if !row.isDefault { Button(parityString("settings.makeDefault")) { Task { await model.setDefault(row) } } }
                }
            }
        }
        if rows.isEmpty { ContentUnavailableView(parityString("settings.noBackends"), systemImage: layer == .lightning ? "bolt.slash" : "network.slash") }
        parityStatus(model.errorMessage, model.resultMessage)
    }

    private func endpointLayer(_ row: SettingsBackendRow) -> ConnectionLayer {
        if row.chain == "liquid" || row.kind.contains("liquid") { return .liquid }
        if ["lnd", "cln", "coreln", "core-lightning", "nwc"].contains(row.kind) { return .lightning }
        return .base
    }
}

private struct BackendEditorState: Identifiable {
    let id = UUID()
    var original: SettingsBackendRow?
    var draft: BackendDraft
    init(row: SettingsBackendRow? = nil) { original = row; draft = row.map(BackendDraft.init) ?? BackendDraft() }
}

private struct BackendEditorSheet: View {
    @Bindable var model: BackendSettingsViewModel
    let original: SettingsBackendRow?
    @State private var draft: BackendDraft
    let didFinish: () -> Void
    @Environment(\.dismiss) private var dismiss
    init(model: BackendSettingsViewModel, state: BackendEditorState, didFinish: @escaping () -> Void) {
        self.model = model; original = state.original; _draft = State(initialValue: state.draft); self.didFinish = didFinish
    }
    var body: some View {
        NavigationStack {
            Form {
                TextField(parityString("field.name"), text: $draft.name).disabled(original != nil)
                Picker(parityString("field.type"), selection: $draft.kind) { ForEach(["esplora", "electrum", "bitcoinrpc", "liquid-esplora", "lnd", "coreln", "nwc", "btcpay"], id: \.self, content: Text.init) }
                TextField(AppLocalization.code("url"), text: $draft.url)
                Picker(parityString("connections.chain"), selection: $draft.chain) {
                    Text(AppLocalization.code("bitcoin")).tag("bitcoin")
                    Text(AppLocalization.code("liquid")).tag("liquid")
                }
                TextField(parityString("field.network"), text: $draft.network)
                TextField(parityString("connections.proxyOptional"), text: $draft.proxy)
                Stepper("\(parityString("connections.timeout")): \(draft.timeout)s", value: $draft.timeout, in: 1...120)
                TextField(parityString("connections.notesOptional"), text: $draft.notes, axis: .vertical).lineLimit(2...4)
                if original?.hasCredentials == true {
                    Label(parityString("connections.credentials"), systemImage: "key.fill")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if draft.kind == "bitcoinrpc" {
                    HStack { TextField(parityString("connections.cookieFile"), text: $draft.cookieFile); Button(parityString("action.browse")) { if let url = NativeAffordances.chooseFile(types: [.data]) { draft.cookieFile = url.path } } }
                    TextField(parityString("connections.rpcUser"), text: $draft.username)
                    SecureField(parityString("connections.rpcPassword"), text: $draft.password)
                }
                if draft.kind == "electrum" {
                    Toggle(parityString("connections.trustSelfSigned"), isOn: $draft.trustSelfSigned)
                    if !draft.trustSelfSigned {
                        HStack { TextField(parityString("connections.certificateOptional"), text: $draft.certificate); Button(parityString("action.browse")) { if let url = NativeAffordances.chooseFile(types: [.data]) { draft.certificate = url.path } } }
                    }
                }
                if draft.kind == "lnd" {
                    SecureField(parityString("connections.macaroon"), text: $draft.token)
                    Toggle(parityString("connections.trustSelfSigned"), isOn: $draft.trustSelfSigned)
                    if !draft.trustSelfSigned {
                        HStack { TextField(parityString("connections.certificateOptional"), text: $draft.certificate); Button(parityString("action.browse")) { if let url = NativeAffordances.chooseFile(types: [.data]) { draft.certificate = url.path } } }
                    }
                }
                if ["coreln", "core-lightning", "cln"].contains(draft.kind) {
                    TextField(parityString("connections.commandoPeerID"), text: $draft.commandoPeerID)
                    SecureField(parityString("connections.commandoRune"), text: $draft.token)
                    TextField(parityString("connections.lightningCLI"), text: $draft.lightningCLI)
                    TextField(parityString("connections.lightningDirectory"), text: $draft.lightningDirectory)
                    TextField(parityString("connections.rpcFile"), text: $draft.rpcFile)
                }
                if draft.kind == "nwc" {
                    SecureField(parityString("connections.nwcSecret"), text: $draft.token)
                }
                if ["esplora", "liquid-esplora"].contains(draft.kind) {
                    SecureField(parityString("connections.authHeaderOptional"), text: $draft.authHeader)
                    SecureField(parityString("connections.tokenOptional"), text: $draft.token)
                }
                if draft.kind == "btcpay" {
                    SecureField(parityString("connections.tokenOptional"), text: $draft.token)
                }
                if draft.chain == "bitcoin", ["esplora", "electrum", "bitcoinrpc"].contains(draft.kind) {
                    Toggle(parityString("connections.silentPayments"), isOn: $draft.silentPayments)
                }
                HStack { Button(parityString("connections.test")) { Task { await model.test(draft) } }; if let probe = model.probe { Label(probe.headline, systemImage: probe.ok ? "checkmark.circle" : "xmark.circle") } }
                if let error = model.errorMessage { Text(parityError(error)).foregroundStyle(.red) }
            }.formStyle(.grouped).navigationTitle(original == nil ? parityString("settings.addBackend") : parityString("settings.editBackend"))
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button(parityString("action.cancel")) { dismiss() } }
                    if let original { ToolbarItem { Button(parityString("action.delete"), role: .destructive) { Task { await model.delete(original); if model.errorMessage == nil { didFinish() } } } } }
                    ToolbarItem(placement: .confirmationAction) { Button(parityString("action.save")) { Task { await model.save(draft, editing: original?.name); if model.errorMessage == nil { didFinish() } } }.disabled(draft.name.isEmpty || draft.url.isEmpty || model.isWorking) }
                }
        }.frame(width: 630, height: 600)
    }
}

private struct AIProviderParitySettings: View {
    @Bindable var model: AIProviderSettingsViewModel
    @Bindable var history: ChatHistorySettingsViewModel
    @Binding var editor: ProviderEditorState?
    @AppStorage("aiFeaturesEnabled") private var aiFeaturesEnabled = true
    var body: some View {
        parityCard(parityString("settings.aiFeatures")) {
            Toggle(parityString("settings.aiFeaturesEnabled"), isOn: $aiFeaturesEnabled)
            Text(parityString("settings.aiFeaturesHint")).font(.caption).foregroundStyle(.secondary)
        }
        HStack { Text(parityString("settings.aiPrivacyHint")).foregroundStyle(.secondary); Spacer(); Button(parityString("settings.addProvider")) { editor = ProviderEditorState() } }
        if let warning = model.policyWarning { Label(warning, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
        ForEach(model.providers) { row in
            parityCard(row.displayName) {
                HStack {
                    VStack(alignment: .leading) {
                        Text(row.baseURL).font(.caption.monospaced())
                        Text([AppLocalization.code(row.kind), row.defaultModel ?? "—", AppLocalization.code(row.secretStore)].joined(separator: " · "))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    if row.isDefault { Text(parityString("settings.default")).font(.caption).padding(5).background(.quaternary, in: Capsule()) }
                    Button { editor = ProviderEditorState(row: row) } label: { Image(systemName: "pencil") }
                    if !row.isDefault { Button(parityString("settings.makeDefault")) { Task { await model.setDefault(row) } } }
                    Button(parityString("settings.testProvider")) { Task { await model.test(row) } }
                    if model.nativeSecretStoreAvailable,
                       let nativeStore = model.nativeSecretStore,
                       row.hasAPIKey,
                       row.secretStore == "sqlcipher_inline" {
                        Button(parityString("settings.moveKeyNative")) { Task { await model.moveAPIKey(row, to: nativeStore) } }
                    } else if row.hasAPIKey && !row.secretStore.isEmpty && row.secretStore != "sqlcipher_inline" {
                        Button(parityString("settings.moveKeyDatabase")) { Task { await model.moveAPIKey(row, to: "sqlcipher_inline") } }
                    }
                }
            }
        }
        parityCard(parityString("settings.chatHistory")) {
            Picker(parityString("settings.chatHistoryPolicy"), selection: Binding(
                get: { history.mode },
                set: { mode in Task { await history.configure(mode) } }
            )) {
                Text(parityString("settings.chatHistoryAuto")).tag("auto")
                Text(parityString("settings.chatHistoryOn")).tag("on")
                Text(parityString("settings.chatHistoryOff")).tag("off")
            }
            LabeledContent(parityString("settings.chatHistoryEffective"), value: history.effectiveEnabled ? parityString("common.yes") : parityString("common.no"))
            LabeledContent(parityString("settings.storedSessions"), value: String(history.sessionCount))
            Button(parityString("settings.clearChatHistory"), role: .destructive) { Task { await history.clear() } }
                .disabled(history.sessionCount == 0 || history.isWorking)
            if let error = history.errorMessage { Text(parityError(error)).foregroundStyle(.red) }
        }
        parityStatus(model.errorMessage, nil)
    }
}

private struct ProviderEditorState: Identifiable {
    let id = UUID(); var original: AIProviderRow?; var draft: AIProviderDraft
    init(row: AIProviderRow? = nil) { original = row; draft = row.map(AIProviderDraft.init) ?? AIProviderDraft() }
}

private struct ProviderEditorSheet: View {
    @Bindable var model: AIProviderSettingsViewModel
    let original: AIProviderRow?
    @State private var draft: AIProviderDraft
    let didFinish: () -> Void
    @Environment(\.dismiss) private var dismiss
    init(model: AIProviderSettingsViewModel, state: ProviderEditorState, didFinish: @escaping () -> Void) { self.model = model; original = state.original; _draft = State(initialValue: state.draft); self.didFinish = didFinish }
    var body: some View {
        NavigationStack {
            Form {
                TextField(parityString("field.name"), text: $draft.name).disabled(original != nil)
                TextField(parityString("settings.displayName"), text: $draft.displayName)
                TextField(parityString("settings.baseURL"), text: $draft.baseURL)
                Picker(parityString("field.type"), selection: $draft.kind) {
                    Text(parityString("settings.localProvider")).tag("local")
                    Text(parityString("settings.remoteProvider")).tag("remote")
                    Text(AppLocalization.code("tee")).tag("tee")
                    Text(AppLocalization.code("cli")).tag("cli")
                }
                TextField(parityString("settings.defaultModel"), text: $draft.defaultModel)
                SecureField(parityString("settings.apiKeyOptional"), text: $draft.apiKey)
                if draft.kind != "local" { Toggle(parityString("settings.remoteAcknowledgement"), isOn: $draft.acknowledged) }
                if let error = model.errorMessage { Text(parityError(error)).foregroundStyle(.red) }
            }.formStyle(.grouped).navigationTitle(original == nil ? parityString("settings.addProvider") : parityString("settings.editProvider"))
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button(parityString("action.cancel")) { dismiss() } }
                    if let original, !original.isDefault { ToolbarItem { Button(parityString("action.delete"), role: .destructive) { Task { await model.delete(original); if model.errorMessage == nil { didFinish() } } } } }
                    ToolbarItem(placement: .confirmationAction) { Button(parityString("action.save")) { Task { await model.save(draft, editing: original?.name); if model.errorMessage == nil { didFinish() } } }.disabled(draft.name.isEmpty || draft.baseURL.isEmpty || model.isWorking) }
                }
        }.frame(width: 600, height: 520)
    }
}

private struct MaintenanceParitySettings: View {
    @Bindable var model: MaintenanceSettingsViewModel
    var body: some View {
        parityCard(parityString("settings.reportFreshness")) {
            Toggle(parityString("settings.autoSyncReports"), isOn: $model.settings.autoSyncBeforeReportReads)
            Toggle(parityString("settings.backgroundRefresh"), isOn: $model.settings.backgroundEnabled)
            Toggle(parityString("settings.reportReadSync"), isOn: $model.settings.reportReadSync)
        }
        parityCard(parityString("settings.refreshSources")) {
            Toggle(parityString("settings.sourceOnchain"), isOn: $model.settings.onchainWallet)
            Toggle(parityString("settings.sourceBTCPayWallet"), isOn: $model.settings.btcpayWallet)
            Toggle(parityString("settings.sourceBTCPayProvenance"), isOn: $model.settings.btcpayProvenance)
            Toggle(parityString("settings.sourceRates"), isOn: $model.settings.marketRates)
            Toggle(parityString("settings.sourceJournals"), isOn: $model.settings.journals)
            Toggle(parityString("settings.carryingValue"), isOn: $model.settings.bitcoinRailCarryingValue)
            Picker(parityString("settings.rateProvider"), selection: $model.settings.marketRateProvider) { ForEach(model.settings.marketRateProviders, id: \.self, content: Text.init) }
            HStack { Button(parityString("action.save")) { Task { await model.save() } }; Button(parityString("settings.runLocal")) { Task { await model.run(syncMode: "never") } }; Button(parityString("settings.runRefresh")) { Task { await model.run(syncMode: "if_enabled") } } }
        }
        ForEach(model.blockers, id: \.self) { Label($0, systemImage: "exclamationmark.triangle").foregroundStyle(.orange) }
        parityStatus(model.errorMessage, model.resultMessage)
    }
}

private struct MarketParitySettings: View {
    @Bindable var model: RatesSettingsViewModel
    @State private var path = ""
    @State private var pair = "BTC-EUR"
    @State private var source = "coinbase-exchange"
    @State private var days = 30
    @State private var reprice = true
    var body: some View {
        parityCard(parityString("settings.rateSummary")) { Text(model.summary).font(.title2.monospacedDigit()) }
        parityCard(parityString("settings.krakenArchive")) {
            HStack {
                TextField(parityString("connections.sourceFile"), text: $path)
                Button(parityString("action.browse")) { if let url = NativeAffordances.chooseFile(types: [.commaSeparatedText, .zip, .data]) { path = url.path } }
            }
            TextField(parityString("settings.pair"), text: $pair)
            Button(parityString("settings.importRates")) { Task { await model.importKrakenCSV(path: path, pair: pair.nilIfEmpty, operation: "incremental") } }.disabled(path.isEmpty)
        }
        parityCard(parityString("settings.rebuildRates")) {
            Picker(parityString("settings.rateProvider"), selection: $source) { ForEach(["coinbase-exchange", "coingecko", "mempool", "kraken-csv"], id: \.self, content: Text.init) }
            Stepper("\(days) \(parityString("settings.days"))", value: $days, in: 1...3650)
            Toggle(parityString("settings.repriceTransactions"), isOn: $reprice)
            Button(parityString("settings.rebuildRates")) { Task { await model.rebuild(source: source, pair: pair.nilIfEmpty, days: days, path: path.nilIfEmpty, reprice: reprice) } }
        }
        parityStatus(model.errorMessage, model.resultMessage)
    }
}

private struct ReplicationParitySettings: View {
    @Bindable var model: ReplicationSettingsViewModel
    @State private var ownerName = "Owner"
    @State private var transportLabel = "Shared mailbox"
    @State private var transportPath = ""
    @State private var joinText = ""
    @State private var memberID = ""
    @State private var memberRole = "auditor"
    var body: some View {
        parityCard(parityString("settings.replicationStatus")) {
            Toggle(parityString("settings.replicationEnabled"), isOn: Binding(
                get: { model.enabled },
                set: { enabled in Task { await model.setEnabled(enabled, displayName: ownerName) } }
            ))
            LabeledContent(parityString("settings.replicationConfigured"), value: model.configured ? parityString("common.yes") : parityString("common.no"))
            HStack {
                Button(parityString("settings.syncPush")) { Task { await model.push(transport: model.transports.first?.id) } }.disabled(!model.enabled)
                Button(parityString("settings.syncPull")) { Task { await model.pull(transport: model.transports.first?.id) } }.disabled(!model.enabled)
            }
        }
        parityCard(parityString("settings.transports")) {
            ForEach(model.transports) { row in
                HStack { Label(row.label, systemImage: "folder"); Text(AppLocalization.code(row.kind)).foregroundStyle(.secondary); Spacer(); Button(role: .destructive) { Task { await model.deleteTransport(row) } } label: { Image(systemName: "trash") } }
            }
            TextField(parityString("settings.transportLabel"), text: $transportLabel)
            HStack {
                TextField(parityString("settings.transportPath"), text: $transportPath)
                Button(parityString("action.chooseFolder")) { if let url = NativeAffordances.chooseDirectory() { transportPath = url.path } }
            }
            Button(parityString("settings.addFolderTransport")) {
                Task { await model.configureTransport(id: nil, kind: "folder", label: transportLabel, config: ["path": .string(transportPath)]) }
            }.disabled(transportPath.isEmpty)
        }
        parityCard(parityString("settings.membersDevices")) {
            ForEach(model.members) { row in
                HStack { Text(row.name); Text(AppLocalization.code(row.role)).foregroundStyle(.secondary); Spacer(); if !row.revoked { Button(parityString("settings.revoke"), role: .destructive) { Task { await model.revokeMember(row) } } } }
            }
            Divider()
            ForEach(model.devices) { row in
                HStack {
                    Text(row.label)
                    Text(row.memberName).foregroundStyle(.secondary)
                    Spacer()
                    if !row.isLocal {
                        Button(parityString("settings.revoke"), role: .destructive) {
                            Task { await model.revokeDevice(row) }
                        }
                    }
                }
            }
        }
        parityCard(parityString("settings.joinInvitation")) {
            TextField(parityString("settings.memberID"), text: $memberID)
            Picker(parityString("settings.memberRole"), selection: $memberRole) {
                Text(parityString("settings.roleEditor")).tag("editor")
                Text(parityString("settings.roleAuditor")).tag("auditor")
            }
            HStack {
                Button(parityString("settings.createInvite")) { Task { await model.createInvite(memberID: memberID, role: memberRole) } }
                    .disabled(memberID.isEmpty)
                Button(parityString("settings.createJoinRequest")) { Task { await model.joinRequest(displayName: ownerName, deviceLabel: Host.current().localizedName ?? "Mac") } }
            }
            if let result = model.resultMessage, !result.isEmpty {
                HStack {
                    Text(result).font(.system(.caption, design: .monospaced)).textSelection(.enabled).lineLimit(3)
                    Button(parityString("settings.copyCode")) { NativeAffordances.copy(result) }
                }
            }
            TextEditor(text: $joinText).font(.system(.caption, design: .monospaced)).frame(minHeight: 80)
            Button(parityString("settings.join")) { Task { await model.join(invitation: joinText) } }.disabled(joinText.isEmpty)
        }
        if !model.conflicts.isEmpty {
            parityCard(parityString("settings.conflicts")) {
                ForEach(model.conflicts) { row in
                    HStack { Text("\(row.table).\(row.field)"); Spacer(); Button(parityString("settings.keepLocal")) { Task { await model.resolveConflict(row, choice: "local") } }; Button(parityString("settings.keepRemote")) { Task { await model.resolveConflict(row, choice: "remote") } } }
                }
            }
        }
        parityStatus(model.errorMessage, model.resultMessage)
    }
}

private struct DataParitySettings: View {
    @Bindable var model: DestructiveSettingsViewModel
    @State private var bookName = ""
    @State private var workspaceName = ""
    @State private var passphrase = ""
    @State private var clearRates = false
    @State private var resetConfirm = false
    @State private var deleteConfirm = false
    var body: some View {
        parityCard(parityString("settings.resetBook")) {
            Text(parityString("settings.resetBookHint")).foregroundStyle(.secondary)
            TextField(parityString("book.name"), text: $bookName)
            SecureField(parityString("security.passphraseOptional"), text: $passphrase)
            Toggle(parityString("settings.clearSharedRates"), isOn: $clearRates)
            Button(parityString("settings.resetBook"), role: .destructive) { resetConfirm = true }.disabled(bookName.isEmpty)
        }
        parityCard(parityString("settings.deleteWorkspace")) {
            Text(parityString("settings.deleteWorkspaceHint")).foregroundStyle(.secondary)
            TextField(parityString("book.setName"), text: $workspaceName)
            SecureField(parityString("security.passphraseOptional"), text: $passphrase)
            Button(parityString("settings.deleteWorkspace"), role: .destructive) { deleteConfirm = true }.disabled(workspaceName.isEmpty)
        }
        parityStatus(model.errorMessage, model.resultMessage)
        .confirmationDialog(parityString("settings.resetConfirm"), isPresented: $resetConfirm) {
            Button(parityString("settings.resetBook"), role: .destructive) { Task { await model.resetBook(name: bookName, clearSharedRates: clearRates, passphrase: passphrase.nilIfEmpty) } }
        }
        .confirmationDialog(parityString("settings.deleteConfirm"), isPresented: $deleteConfirm) {
            Button(parityString("settings.deleteWorkspace"), role: .destructive) { Task { await model.deleteWorkspace(name: workspaceName, passphrase: passphrase.nilIfEmpty) } }
        }
    }
}

/// Reusable first-run view. The host can present it before AppShellView when
/// status has no active profile, while Settings can reopen it for setup repair.
struct FullOnboardingScreen: View {
    @State private var model: OnboardingParityViewModel
    let didFinish: () -> Void
    init(daemon: any DaemonClient, didFinish: @escaping () -> Void) {
        _model = State(initialValue: OnboardingParityViewModel(
            daemon: daemon,
            touchIDManager: MacOSNativeServices.touchID,
            touchIDAccount: canonicalTouchIDAccount(for: nil)
        )); self.didFinish = didFinish
    }
    var body: some View {
        Group {
            if model.flowMode == .start { startChoices }
            else { setupFlow }
        }
        .frame(width: 760, height: 700)
        .task { await model.loadCatalog() }
    }

    private var startChoices: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 5) {
                    Text(parityString("onboarding.start.title")).font(.largeTitle.bold())
                    Text(parityString("onboarding.start.subtitle")).foregroundStyle(.secondary)
                }
                Spacer()
                Button(parityString("action.cancel")) { didFinish() }
            }
            if model.awaitingImportedProjectUnlock, let project = model.importedProject {
                GroupBox(parityString("onboarding.import.unlockTitle")) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(URL(fileURLWithPath: project.stateRoot).lastPathComponent).font(.headline)
                        Text(project.dataRoot).font(.caption.monospaced()).foregroundStyle(.secondary)
                            .textSelection(.enabled)
                        SecureField(parityString("onboarding.import.passphrase"), text: $model.importedProjectPassphrase)
                        Button(parityString("onboarding.import.unlock")) {
                            Task { await model.unlockImportedProject(); if model.didComplete { didFinish() } }
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(model.importedProjectPassphrase.isEmpty || model.isWorking)
                    }.padding(6).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            onboardingChoice(
                parityString("onboarding.start.create.title"),
                parityString("onboarding.start.create.body"), "book.closed", prominent: true
            ) { model.beginSetup() }
            onboardingChoice(
                parityString("onboarding.start.import.title"),
                parityString("onboarding.start.import.body"), "folder"
            ) { importProject() }
            onboardingChoice(
                parityString("onboarding.start.quick.title"),
                parityString("onboarding.start.quick.body"), "forward.end"
            ) { model.beginQuickStart() }
            if model.regtestAvailable {
                onboardingChoice(
                    parityString("onboarding.start.demo.title"),
                    parityString("onboarding.start.demo.body"), "server.rack"
                ) {
                    Task { await model.openRegtestDemo(); if model.didComplete { didFinish() } }
                }
            }
            Spacer()
            Label(parityString("onboarding.start.privacy"), systemImage: "lock.shield")
                .font(.caption).foregroundStyle(.secondary)
            if let error = model.errorMessage { Label(localOnboardingError(error), systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
        }.padding(28)
    }

    private var setupFlow: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                ForEach(OnboardingStep.allCases, id: \.rawValue) { item in
                    VStack(alignment: .leading, spacing: 5) {
                        Text(parityString("onboarding.step.\(stepKey(item))"))
                            .font(.caption).foregroundStyle(item == model.step ? .primary : .secondary)
                        Capsule()
                            .fill(item.rawValue <= model.step.rawValue ? Color.accentColor : Color.secondary.opacity(0.2))
                            .frame(height: 4)
                    }
                }
            }.padding([.horizontal, .top])
            Form {
                switch model.step {
                case .essentials: essentialsStep
                case .sync: syncStep
                case .ai: aiStep
                case .security: securityStep
                case .review: reviewStep
                }
                if let warning = model.touchIDWarning {
                    Label(warning, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                if let error = model.errorMessage {
                    Label(localOnboardingError(error), systemImage: "exclamationmark.triangle").foregroundStyle(.red)
                }
            }.formStyle(.grouped)
            Divider()
            HStack {
                Button(parityString("action.cancel")) { didFinish() }
                Spacer()
                Button(parityString("action.back")) { model.goBack() }
                if model.step == .review {
                    Button(parityString("onboarding.finish")) {
                        Task { await model.finish(); if model.didComplete { didFinish() } }
                    }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(!model.canContinue)
                } else {
                    Button(parityString("action.next")) { model.goNext() }
                        .keyboardShortcut(.defaultAction)
                        .buttonStyle(.borderedProminent)
                        .disabled(!model.canContinue)
                }
            }.padding()
        }
    }

    @ViewBuilder private var essentialsStep: some View {
        Section(parityString("onboarding.essentials.title")) {
            TextField(parityString("book.setName"), text: $model.workspace)
            Text(parityString("onboarding.essentials.setHint")).font(.caption).foregroundStyle(.secondary)
            TextField(parityString("book.name"), text: $model.profile)
            Picker(parityString("reports.taxCountry"), selection: $model.taxCountry) {
                Text(parityString("reports.austria")).tag("at")
                Text(parityString("reports.generic")).tag("generic")
            }
            Picker(parityString("reports.currency"), selection: $model.fiatCurrency) {
                ForEach(["EUR", "USD", "CHF", "GBP"], id: \.self) { Text($0).tag($0) }
            }
            Picker(parityString("reports.algorithm"), selection: $model.gainsAlgorithm) {
                ForEach(gainsAlgorithms, id: \.self) { Text(parityString("onboarding.algorithm.\($0)" )).tag($0) }
            }
            if model.taxCountry != "at" {
                Stepper("\(parityString("onboarding.longTermDays")): \(model.longTermDays)", value: $model.longTermDays, in: 1...3_650)
            }
        }
        .onChange(of: model.taxCountry) { _, country in
            if country == "at" { model.fiatCurrency = "EUR"; model.gainsAlgorithm = "MOVING_AVERAGE_AT" }
            else if model.gainsAlgorithm == "MOVING_AVERAGE_AT" { model.gainsAlgorithm = "FIFO" }
        }
    }

    @ViewBuilder private var syncStep: some View {
        Section(parityString("onboarding.sync.title")) {
            Picker(parityString("onboarding.sync.mode"), selection: $model.backendMode) {
                Text(parityString("onboarding.sync.recommended")).tag(OnboardingBackendMode.recommended)
                Text(parityString("onboarding.sync.custom")).tag(OnboardingBackendMode.custom)
                Text(parityString("onboarding.sync.skip")).tag(OnboardingBackendMode.skip)
            }.pickerStyle(.segmented)
            switch model.backendMode {
            case .recommended:
                Text(parityString("onboarding.sync.recommendedHint")).font(.caption).foregroundStyle(.secondary)
                ForEach(model.publicDefaultBackends) { row in
                    LabeledContent(row.name, value: [AppLocalization.code(row.kind), row.url].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.caption)
                }
            case .custom:
                TextField(parityString("field.name"), text: $model.backend.name)
                Picker(parityString("field.type"), selection: $model.backend.kind) {
                    Text(AppLocalization.code("esplora")).tag("esplora")
                    Text(AppLocalization.code("electrum")).tag("electrum")
                    Text(AppLocalization.code("bitcoinrpc")).tag("bitcoinrpc")
                    Text(parityString("connections.liquidEsplora")).tag("liquid-esplora")
                }
                TextField(AppLocalization.code("url"), text: $model.backend.url)
                Text(parityString("onboarding.sync.customHint")).font(.caption).foregroundStyle(.secondary)
            case .skip:
                Toggle(parityString("onboarding.sync.skipAck"), isOn: $model.skipBackendsAcknowledged)
                Text(parityString("onboarding.sync.skipHint")).font(.caption).foregroundStyle(.orange)
            }
        }
    }

    @ViewBuilder private var aiStep: some View {
        Section(parityString("onboarding.ai.title")) {
            Picker(parityString("onboarding.ai.mode"), selection: $model.aiMode) {
                Text(parityString("onboarding.ai.local")).tag(OnboardingAIMode.local)
                Text(parityString("onboarding.ai.remote")).tag(OnboardingAIMode.remote)
                Text(parityString("onboarding.ai.disabled")).tag(OnboardingAIMode.disabled)
            }.pickerStyle(.segmented)
            if model.aiMode != .disabled {
                TextField(parityString("field.name"), text: $model.provider.name)
                TextField(parityString("settings.baseURL"), text: $model.provider.baseURL)
                SecureField(parityString("settings.apiKeyOptional"), text: $model.provider.apiKey)
                if model.aiMode == .remote {
                    Toggle(parityString("settings.remoteAcknowledgement"), isOn: $model.provider.acknowledged)
                }
            }
            Text(parityString("onboarding.ai.hint.\(model.aiMode.rawValue)"))
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private var securityStep: some View {
        Section(parityString("onboarding.security.title")) {
            Picker(parityString("onboarding.security.storage"), selection: $model.databaseMode) {
                Text(parityString("onboarding.security.encrypted")).tag(OnboardingDatabaseMode.encrypted)
                Text(parityString("onboarding.security.plaintext")).tag(OnboardingDatabaseMode.plaintext)
            }.pickerStyle(.segmented)
            if model.databaseMode == .encrypted {
                SecureField(parityString("security.newPassphrase"), text: $model.databasePassphrase)
                SecureField(parityString("security.confirmPassphrase"), text: $model.databasePassphraseConfirmation)
                Toggle(parityString("onboarding.security.recoveryAck"), isOn: $model.recoveryAcknowledged)
                Toggle(parityString("security.migrateCredentials"), isOn: $model.migrateCredentials)
                if model.touchIDStatus.available {
                    Toggle(parityString("security.rememberTouchID"), isOn: $model.enableTouchID)
                }
                Text(parityString("onboarding.passphraseWarning")).font(.caption).foregroundStyle(.orange)
            } else {
                Toggle(parityString("onboarding.security.plaintextAck"), isOn: $model.plaintextAcknowledged)
                Text(parityString("onboarding.security.plaintextHint")).font(.caption).foregroundStyle(.orange)
            }
        }
    }

    @ViewBuilder private var reviewStep: some View {
        Section(parityString("onboarding.review.title")) {
            reviewRow("onboarding.review.books", value: "\(model.workspace) · \(model.profile)", step: .essentials)
            reviewRow("onboarding.review.tax", value: "\(AppLocalization.code(model.taxCountry)) · \(model.fiatCurrency) · \(parityString("onboarding.algorithm.\(model.gainsAlgorithm)"))", step: .essentials)
            reviewRow("onboarding.review.sync", value: parityString("onboarding.sync.\(model.backendMode.rawValue)"), step: .sync)
            reviewRow("onboarding.review.ai", value: parityString("onboarding.ai.\(model.aiMode.rawValue)"), step: .ai)
            reviewRow("onboarding.review.storage", value: parityString("onboarding.security.\(model.databaseMode.rawValue)"), step: .security)
            Text(parityString("onboarding.review.localOnly")).font(.caption).foregroundStyle(.secondary)
        }
    }

    private var gainsAlgorithms: [String] {
        model.taxCountry == "at"
            ? ["MOVING_AVERAGE_AT", "FIFO", "LIFO", "HIFO", "LOFO"]
            : ["FIFO", "LIFO", "HIFO", "LOFO", "MOVING_AVERAGE"]
    }

    private func stepKey(_ step: OnboardingStep) -> String {
        switch step {
        case .essentials: "essentials"
        case .sync: "sync"
        case .ai: "ai"
        case .security: "security"
        case .review: "review"
        }
    }

    private func importProject() {
        guard let folder = NativeAffordances.chooseDirectory(title: parityString("onboarding.start.import.title")) else { return }
        Task { await model.importProject(folder); if model.didComplete { didFinish() } }
    }

    private func localOnboardingError(_ error: String) -> String {
        AppLocalization.error(error)
    }

    private func onboardingChoice(
        _ title: String, _ body: String, _ icon: String, prominent: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 14) {
                Image(systemName: icon).font(.title2).frame(width: 34)
                VStack(alignment: .leading, spacing: 4) {
                    Text(title).font(.headline)
                    Text(body).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.leading)
                }
                Spacer()
                Image(systemName: "chevron.right").foregroundStyle(.secondary)
            }.padding(10).contentShape(Rectangle())
        }
        .buttonStyle(.bordered)
        .tint(prominent ? Color.accentColor : Color.secondary)
        .controlSize(.large)
    }

    private func reviewRow(_ key: String, value: String, step: OnboardingStep) -> some View {
        HStack(alignment: .firstTextBaseline) {
            LabeledContent(parityString(key), value: value)
            Button(parityString("onboarding.review.change")) { model.jump(to: step) }.buttonStyle(.link)
        }
    }
}

@ViewBuilder private func parityCard<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
    GroupBox(title) { VStack(alignment: .leading, spacing: 10) { content() }.padding(8).frame(maxWidth: .infinity, alignment: .leading) }
}

@ViewBuilder private func parityStatus(_ error: String?, _ result: String?) -> some View {
    if let error { Label(parityError(error), systemImage: "exclamationmark.triangle").foregroundStyle(.red) }
    if let result { Label(AppLocalization.code(result), systemImage: "checkmark.circle").foregroundStyle(.green) }
}

private func parityError(_ value: String) -> String {
    AppLocalization.error(value)
}
