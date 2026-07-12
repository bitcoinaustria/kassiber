import SwiftUI
import KassiberViewModels

private func flowLocalized(_ key: String) -> String {
    AppLocalization.string(key)
}

struct TransactionFlowPresentation: View {
    let snapshot: TransactionGraphSnapshot
    let privacy: TransactionPrivacyContext
    let privacyIsLoading: Bool
    var graphIsLoading = false
    var expanded = false
    var onSelectRouteLeg: ((String) -> Void)?

    @State private var selectedRouteLeg: String?
    @State private var showsAllInputs = false
    @State private var showsAllOutputs = false
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let route = snapshot.swapRoute {
                swapRoute(route)
            }
            if !snapshot.warnings.isEmpty {
                warningStrip
            }
            if snapshot.hasFlowEvidence {
                flowDiagram
                annotations
                inputOutputPanels
                technicalSummary
            } else {
                ContentUnavailableView {
                    Label(flowLocalized("transactionFlow.unavailable"), systemImage: "point.3.connected.trianglepath.dotted")
                } description: {
                    Text(unavailableReason)
                }
                .frame(minHeight: 150)
            }
            privacyPanel
        }
        .onAppear {
            if selectedRouteLeg == nil { selectedRouteLeg = snapshot.swapRoute?.currentLeg }
        }
        .onChange(of: snapshot.swapRoute?.id) { _, _ in
            selectedRouteLeg = snapshot.swapRoute?.currentLeg
        }
        .overlay {
            if graphIsLoading {
                ProgressView(flowLocalized("transactionFlow.loadingGraph"))
                    .padding(12)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 9))
            }
        }
    }

    private func swapRoute(_ route: TransactionSwapRoute) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(routeTitle(route.routeKind), systemImage: "arrow.left.arrow.right.circle.fill")
                    .font(.headline)
                Spacer()
                if let policy = route.policy {
                    Text(AppLocalization.code(policy))
                        .font(.caption)
                        .padding(.horizontal, 7)
                        .padding(.vertical, 3)
                        .background(.quaternary, in: Capsule())
                }
            }
            HStack(spacing: 10) {
                routeLeg(route.out, title: flowLocalized(route.out.role == "consolidation"
                    ? "transactionFlow.consolidation" : "transactionFlow.outgoing"), key: "out")
                VStack(spacing: 5) {
                    Image(systemName: "arrow.right")
                    Text(route.out.counterparty ?? route.incoming.counterparty ?? flowLocalized("transactionFlow.pairedRoute"))
                        .font(.caption)
                        .lineLimit(2)
                        .kassiberSensitive()
                    if let fee = route.feeSats {
                        Text(String(format: flowLocalized("transactionFlow.routeFee %@"), KassiberFormatting.sats(fee, locale: locale)))
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                            .kassiberSensitive()
                    }
                }
                .frame(minWidth: 130)
                routeLeg(route.incoming, title: flowLocalized(route.incoming.role == "consolidation"
                    ? "transactionFlow.consolidation" : "transactionFlow.incoming"), key: "in")
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
    }

    private func routeLeg(_ leg: TransactionSwapRouteLeg, title: String, key: String) -> some View {
        Button {
            selectedRouteLeg = key
            onSelectRouteLeg?(key)
        } label: {
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(title).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    if selectedRouteLeg == key {
                        Text(flowLocalized("transactionFlow.selected"))
                            .font(.caption2)
                            .foregroundStyle(.tint)
                    }
                }
                Label(leg.network.isEmpty ? leg.asset : leg.network, systemImage: assetIcon(leg.asset, leg.network))
                    .font(.callout.weight(.medium))
                if let amount = leg.amountSats {
                    Text(assetAmount(amount, asset: leg.asset))
                        .font(.callout.monospacedDigit())
                        .kassiberSensitive()
                }
                Text(leg.wallet ?? flowLocalized("transactionFlow.unknownWallet"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .kassiberSensitive()
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selectedRouteLeg == key ? Color.accentColor.opacity(0.12) : Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(selectedRouteLeg == key ? Color.accentColor.opacity(0.65) : Color.secondary.opacity(0.2)))
        }
        .buttonStyle(.plain)
        .disabled(onSelectRouteLeg == nil || (leg.id == nil && leg.transactionID == nil))
    }

    private var warningStrip: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(snapshot.warnings) { warning in
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: warning.level == "error" ? "xmark.octagon.fill" : "exclamationmark.triangle.fill")
                        .foregroundStyle(warning.level == "error" ? .red : .orange)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(warningLabel(warning)).font(.callout)
                        Text(AppLocalization.code(warning.code)).font(.caption2).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 8))
    }

    private var flowDiagram: some View {
        ZStack {
            Canvas { context, size in
                let center = CGPoint(x: size.width / 2, y: size.height / 2)
                let inputs = Array(snapshot.inputs.prefix(expanded ? 24 : 10))
                let outputs = Array(snapshot.outputs.prefix(expanded ? 24 : 10))
                for (index, _) in inputs.enumerated() {
                    let y = size.height * CGFloat(index + 1) / CGFloat(inputs.count + 1)
                    var path = Path()
                    path.move(to: CGPoint(x: 14, y: y))
                    path.addCurve(to: center, control1: CGPoint(x: size.width * 0.28, y: y), control2: CGPoint(x: size.width * 0.38, y: center.y))
                    context.stroke(path, with: .color(.red.opacity(0.55)), lineWidth: 2.2)
                }
                for (index, _) in outputs.enumerated() {
                    let y = size.height * CGFloat(index + 1) / CGFloat(outputs.count + 1)
                    var path = Path()
                    path.move(to: center)
                    path.addCurve(to: CGPoint(x: size.width - 14, y: y), control1: CGPoint(x: size.width * 0.62, y: center.y), control2: CGPoint(x: size.width * 0.72, y: y))
                    context.stroke(path, with: .color(.green.opacity(0.58)), lineWidth: 2.2)
                }
                if snapshot.fee != nil {
                    var feePath = Path()
                    feePath.move(to: center)
                    feePath.addLine(to: CGPoint(x: size.width * 0.67, y: size.height - 12))
                    context.stroke(feePath, with: .color(.orange.opacity(0.7)), style: StrokeStyle(lineWidth: 2, dash: [5, 4]))
                }
            }
            HStack {
                Text(String(format: flowLocalized("transactionFlow.inputs %lld"), Int64(snapshot.inputs.count)))
                    .font(.caption2).foregroundStyle(.secondary)
                Spacer()
                VStack(spacing: 3) {
                    Image(systemName: "point.3.connected.trianglepath.dotted")
                    Text(shortReference(snapshot.transaction?.transactionID ?? snapshot.transaction?.id ?? ""))
                        .font(.caption2.monospaced())
                        .kassiberSensitive()
                }
                .padding(9)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                Spacer()
                Text(String(format: flowLocalized("transactionFlow.outputs %lld"), Int64(snapshot.outputs.count)))
                    .font(.caption2).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 16)
        }
        .frame(height: expanded ? 280 : 190)
        .background(Color(nsColor: .textBackgroundColor).opacity(0.45), in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(.separator.opacity(0.7)))
        .accessibilityLabel(flowLocalized("transactionFlow.diagramAccessibility"))
    }

    @ViewBuilder private var annotations: some View {
        let items = snapshot.annotations + snapshot.linkedPairs
        if !items.isEmpty || snapshot.quarantineReason != nil {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    if let quarantine = snapshot.quarantineReason {
                        Label(AppLocalization.code(quarantine), systemImage: "exclamationmark.octagon.fill")
                            .foregroundStyle(.red)
                            .flowBadge()
                    }
                    ForEach(items.prefix(12)) { annotation in
                        Label(annotation.label, systemImage: annotation.severity == "warning" ? "exclamationmark.triangle" : "tag")
                            .foregroundStyle(annotation.severity == "warning" ? .orange : .primary)
                            .flowBadge()
                    }
                }
            }
        }
    }

    private var inputOutputPanels: some View {
        HStack(alignment: .top, spacing: 14) {
            nodeColumn(
                title: flowLocalized("transactionFlow.inputsTitle"), nodes: snapshot.inputs,
                side: "input", expanded: $showsAllInputs
            )
            nodeColumn(
                title: flowLocalized("transactionFlow.outputsTitle"), nodes: snapshot.outputs,
                side: "output", expanded: $showsAllOutputs
            )
        }
    }

    private func nodeColumn(
        title: String, nodes: [TransactionGraphNode], side: String, expanded: Binding<Bool>
    ) -> some View {
        let displayed = expanded.wrappedValue ? nodes : Array(nodes.prefix(8))
        return VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Spacer()
                Text(nodes.count.formatted(.number.locale(locale))).font(.caption2).foregroundStyle(.secondary)
            }
            .padding(.bottom, 5)
            ForEach(displayed) { node in
                nodeRow(node, side: side)
                if node.id != displayed.last?.id { Divider() }
            }
            if nodes.count > 8 {
                Button(expanded.wrappedValue ? flowLocalized("transactionFlow.showFewer") : String(format: flowLocalized("transactionFlow.showAll %lld"), Int64(nodes.count - 8))) {
                    expanded.wrappedValue.toggle()
                }
                .buttonStyle(.link)
                .font(.caption)
                .padding(.top, 6)
            }
            Divider().padding(.top, 6)
            HStack {
                Text(nodes.allSatisfy { $0.valueSats != nil } ? flowLocalized("transactionFlow.total") : flowLocalized("transactionFlow.knownTotal"))
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Text(nodeTotal(nodes)).font(.caption.monospacedDigit()).kassiberSensitive()
            }
            .padding(.top, 6)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .background(.quaternary.opacity(0.25), in: RoundedRectangle(cornerRadius: 8))
    }

    private func nodeRow(_ node: TransactionGraphNode, side: String) -> some View {
        HStack(alignment: .top, spacing: 7) {
            Image(systemName: side == "input" ? "arrow.right.circle.fill" : "arrow.right.circle")
                .foregroundStyle(side == "input" ? .red : .green)
            VStack(alignment: .leading, spacing: 2) {
                Text(node.overflow
                    ? String(format: flowLocalized("transactionFlow.moreLegs %lld"), Int64(node.overflowCount))
                    : shortReference(node.reference))
                    .font(.caption.monospaced())
                    .lineLimit(1)
                    .textSelection(.enabled)
                    .kassiberSensitive()
                Text([roleLabel(node.role), ownershipLabel(node.ownership), node.scriptType]
                    .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.caption2).foregroundStyle(.secondary).lineLimit(2)
                if !node.annotations.isEmpty {
                    Text(node.annotations.prefix(2).map(\.label).joined(separator: " · "))
                        .font(.caption2).foregroundStyle(.orange).lineLimit(1)
                }
            }
            Spacer(minLength: 5)
            Text(nodeAmount(node)).font(.caption.monospacedDigit()).kassiberSensitive()
        }
        .padding(.vertical, 6)
    }

    @ViewBuilder private var technicalSummary: some View {
        if let metadata = snapshot.transaction {
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 5) {
                GridRow { technical(flowLocalized("transactionFlow.support"), AppLocalization.code(snapshot.supportLevel)); technical(flowLocalized("transactionFlow.network"), metadata.network.isEmpty ? metadata.asset : metadata.network) }
                GridRow { technical(flowLocalized("transactionFlow.feeRate"), metadata.feeRateSatVByte.map { String(format: "%.2f sat/vB", $0) } ?? "—"); technical(flowLocalized("transactionFlow.version"), metadata.version.map(String.init) ?? "—") }
                GridRow { technical(flowLocalized("transactionFlow.size"), metadata.size.map { "\($0) B" } ?? "—"); technical(flowLocalized("transactionFlow.virtualSize"), metadata.virtualSize.map { "\($0) vB" } ?? "—") }
                GridRow { technical(flowLocalized("transactionFlow.weight"), metadata.weight.map { "\($0) WU" } ?? "—"); technical(flowLocalized("transactionFlow.locktime"), metadata.locktime.map(String.init) ?? "—") }
            }
            .font(.caption)
            .padding(10)
            .background(.quaternary.opacity(0.2), in: RoundedRectangle(cornerRadius: 8))
        }
    }

    private func technical(_ label: String, _ value: String) -> some View {
        LabeledContent(label, value: value).frame(maxWidth: .infinity)
    }

    private var privacyPanel: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack {
                Label(flowLocalized("transactionFlow.privacyTitle"), systemImage: "shield.lefthalf.filled")
                    .font(.headline)
                Spacer()
                Text(AppLocalization.code(privacy.evidenceLevel))
                    .font(.caption)
                    .flowBadge()
            }
            if privacyIsLoading && privacy.matchedTransactionID == nil {
                ProgressView(flowLocalized("transactionFlow.privacyLoading"))
                    .controlSize(.small)
            } else {
                HStack(spacing: 12) {
                    privacyMetric(flowLocalized("transactionFlow.privacyTells"), privacy.degraded ? "—" : String(privacy.tellCount))
                    privacyMetric(flowLocalized("transactionFlow.walletPenalties"), privacy.degraded ? "—" : String(privacy.walletPenaltyCount))
                    privacyMetric(
                        flowLocalized("transactionFlow.tellKinds"),
                        privacy.tellKinds.isEmpty
                            ? flowLocalized("transactionFlow.none")
                            : privacy.tellKinds.map(tellKindLabel).joined(separator: ", ")
                    )
                }
                Text(privacyMessage)
                    .font(.caption)
                    .foregroundStyle(privacy.degraded ? .orange : .secondary)
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 9))
        .overlay(RoundedRectangle(cornerRadius: 9).stroke(.separator.opacity(0.55)))
    }

    private func privacyMetric(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(label).font(.caption2).foregroundStyle(.secondary)
            Text(value).font(.callout.monospacedDigit()).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var privacyMessage: String {
        if let message = privacy.message {
            return String(format: flowLocalized("transactionFlow.privacyError %@"), AppLocalization.error(message))
        }
        if let id = privacy.matchedTransactionID {
            return String(format: flowLocalized("transactionFlow.privacyMatched %@"), shortReference(id))
        }
        return flowLocalized("transactionFlow.privacyDegraded")
    }

    private var unavailableReason: String {
        if let reason = snapshot.unsupportedReason { return AppLocalization.code(reason) }
        return flowLocalized("transactionFlow.unavailableHelp")
    }

    private func routeTitle(_ kind: String) -> String {
        switch kind {
        case "swap": flowLocalized("transactionFlow.swapRoute")
        case "coinjoin": flowLocalized("transactionFlow.coinjoinRoute")
        case "transfer": flowLocalized("transactionFlow.transferRoute")
        default: flowLocalized("transactionFlow.pairRoute")
        }
    }

    private func roleLabel(_ value: String) -> String {
        let key = "transactionFlow.role.\(value)"
        let translated = flowLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private func ownershipLabel(_ value: String) -> String {
        let key = "transactionFlow.ownership.\(value)"
        let translated = flowLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private func tellKindLabel(_ value: String) -> String {
        let canonical = value.lowercased().replacingOccurrences(of: "-", with: "_")
        let key = "transactionFlow.tell.\(canonical)"
        let translated = flowLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private func warningLabel(_ warning: TransactionGraphWarning) -> String {
        let canonical = warning.code.lowercased().replacingOccurrences(of: "-", with: "_")
        let key = "transactionFlow.warning.\(canonical)"
        let translated = flowLocalized(key)
        return translated == key ? AppLocalization.error(warning.message) : translated
    }

    private func nodeAmount(_ node: TransactionGraphNode) -> String {
        if node.valueState == "confidential" { return flowLocalized("transactionFlow.confidential") }
        guard let value = node.valueSats else { return flowLocalized("transactionFlow.unknownAmount") }
        return KassiberFormatting.sats(value, locale: locale)
    }

    private func nodeTotal(_ nodes: [TransactionGraphNode]) -> String {
        let known = nodes.compactMap(\.valueSats)
        guard !known.isEmpty else {
            return nodes.contains(where: { $0.valueState == "confidential" })
                ? flowLocalized("transactionFlow.confidential") : flowLocalized("transactionFlow.unknownAmount")
        }
        return KassiberFormatting.sats(known.reduce(0, +), locale: locale)
    }

    private func assetAmount(_ sats: Int64, asset: String) -> String {
        KassiberFormatting.btc(fromSats: abs(sats), locale: locale)
            .replacingOccurrences(of: "BTC", with: asset)
    }

    private func assetIcon(_ asset: String, _ network: String) -> String {
        let value = "\(asset) \(network)".lowercased()
        if value.contains("lightning") { return "bolt.fill" }
        if value.contains("liquid") || value.contains("lbtc") { return "drop.fill" }
        return "bitcoinsign.circle.fill"
    }

    private func shortReference(_ value: String) -> String {
        guard value.count > 20 else { return value.isEmpty ? "—" : value }
        return "\(value.prefix(9))…\(value.suffix(7))"
    }
}

private extension View {
    func flowBadge() -> some View {
        padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(.quaternary, in: Capsule())
    }
}
