import SwiftUI
import KassiberViewModels

private func draftLocalized(_ key: String) -> String {
    AppLocalization.string(key)
}

struct NewTransactionDraftSheet: View {
    @Binding var draft: NewTransactionDraft
    let wallets: [String]
    let saved: () -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.locale) private var locale

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(draftLocalized("transactionDraft.title")).font(.title2.bold())
                    Text(draftLocalized("transactionDraft.description")).foregroundStyle(.secondary)
                }
                Spacer()
                Button(draftLocalized("action.close")) { dismiss() }.buttonStyle(.borderless)
            }
            .padding(18)
            Divider()
            HSplitView {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        networkTiming
                        parties
                        amounts
                        movement
                        classification
                        evidence
                    }
                    .padding(18)
                }
                preview
                    .frame(minWidth: 280, idealWidth: 320, maxWidth: 380)
            }
            Divider()
            HStack {
                Text(draftLocalized("transactionDraft.demoNote"))
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button(draftLocalized("action.cancel")) { dismiss() }
                Button(draftLocalized("transactionDraft.save")) {
                    saved()
                    dismiss()
                }
                .buttonStyle(.borderedProminent)
            }
            .padding(12)
        }
        .frame(minWidth: 920, minHeight: 700)
    }

    private var networkTiming: some View {
        draftSection(draftLocalized("transactionDraft.networkTiming")) {
            VStack(alignment: .leading, spacing: 10) {
                Picker(draftLocalized("field.network"), selection: Binding(
                    get: { draft.network }, set: { draft.selectNetwork($0) }
                )) {
                    ForEach(NewTransactionDraft.networks, id: \.self) { network in
                        Text(draftLocalized("transactionDraft.network.\(network.lowercased())")).tag(network)
                    }
                }
                .pickerStyle(.segmented)
                Picker(draftLocalized("filter.flow"), selection: Binding(
                    get: { draft.flow }, set: { draft.selectFlow($0) }
                )) {
                    ForEach(NewTransactionDraft.flows, id: \.self) { flow in
                        Text(draftLocalized("transactionDraft.flow.\(flow)")).tag(flow)
                    }
                }
                .pickerStyle(.segmented)
                HStack {
                    DatePicker(draftLocalized("transactionDraft.occurredAt"), selection: $draft.occurredAt)
                    if draft.showsConfirmation {
                        Toggle(draftLocalized("transactionDraft.confirmed"), isOn: confirmedBinding)
                        if draft.confirmedAt != nil {
                            DatePicker("", selection: confirmedDateBinding).labelsHidden()
                        }
                    }
                }
            }
        }
    }

    private var parties: some View {
        draftSection(draftLocalized("transactionDraft.partiesRoute")) {
            if draft.isTwoLeg {
                HStack {
                    walletPicker(draftLocalized("transactionDraft.from"), selection: $draft.fromWallet)
                    walletPicker(draftLocalized("transactionDraft.to"), selection: $draft.toWallet)
                    TextField(draftLocalized("transactionDraft.swapService"), text: $draft.swapService)
                }
            } else if draft.flow == "incoming" {
                HStack {
                    TextField(draftLocalized("transactionDraft.fromExternal"), text: $draft.fromExternal)
                    walletPicker(draftLocalized("transactionDraft.to"), selection: $draft.toWallet)
                }
            } else if draft.flow == "outgoing" {
                HStack {
                    walletPicker(draftLocalized("transactionDraft.from"), selection: $draft.fromWallet)
                    TextField(draftLocalized("transactionDraft.toExternal"), text: $draft.toExternal)
                }
            } else {
                HStack {
                    walletPicker(draftLocalized("transactionDraft.from"), selection: $draft.fromWallet)
                    walletPicker(draftLocalized("transactionDraft.to"), selection: $draft.toWallet)
                }
            }
        }
    }

    private var amounts: some View {
        draftSection(draftLocalized("transactionDraft.amountPricing")) {
            VStack(spacing: 10) {
                if draft.isTwoLeg {
                    HStack {
                        VStack(alignment: .leading) {
                            Text(draftLocalized("transactionDraft.legOut")).font(.caption).foregroundStyle(.secondary)
                            HStack {
                                TextField(draftLocalized("transactionDraft.sats"), text: Binding(get: { draft.sendAmountSats }, set: { draft.updateAmount($0, field: "send") }))
                                TextField(draftLocalized("field.asset"), text: $draft.sendAsset).frame(width: 90)
                            }
                        }
                        VStack(alignment: .leading) {
                            Text(draftLocalized("transactionDraft.legIn")).font(.caption).foregroundStyle(.secondary)
                            HStack {
                                TextField(draftLocalized("transactionDraft.sats"), text: Binding(get: { draft.receiveAmountSats }, set: { draft.updateAmount($0, field: "receive") }))
                                TextField(draftLocalized("field.asset"), text: $draft.receiveAsset).frame(width: 90)
                            }
                        }
                    }
                } else {
                    HStack {
                        TextField(draftLocalized("transactionDraft.amountSats"), text: Binding(get: { draft.amountSats }, set: { draft.updateAmount($0) }))
                        if draft.network == "Exchange" || draft.network == "Other" {
                            TextField(draftLocalized("field.asset"), text: $draft.asset).frame(width: 100)
                        }
                        TextField(draftLocalized("transactionDraft.feeSats"), text: $draft.feeSats)
                    }
                }
                HStack {
                    TextField(String(format: draftLocalized("transactionDraft.price %@"), draft.fiatCurrency), text: Binding(get: { draft.pricePerBTC }, set: { draft.updatePrice($0) }))
                    TextField(String(format: draftLocalized("transactionDraft.value %@"), draft.fiatCurrency), text: Binding(get: { draft.totalValue }, set: { draft.updateTotal($0) }))
                    Picker(draftLocalized("transactionDraft.pricing"), selection: $draft.pricingSource) {
                        ForEach(NewTransactionDraft.pricingSources, id: \.self) { source in
                            Text(draftLocalized("transactionDraft.pricing.\(source)")).tag(source)
                        }
                    }
                    .frame(width: 210)
                }
            }
        }
    }

    private var movement: some View {
        draftSection(draftLocalized("transactionDraft.movement")) {
            HStack {
                TextField(draftLocalized("transactionDraft.movementSearch"), text: $draft.movementID)
                Button(draftLocalized("transactionDraft.newMovement")) { draft.movementID = "new" }
            }
        }
    }

    private var classification: some View {
        draftSection(draftLocalized("transactionDraft.classification")) {
            VStack(spacing: 10) {
                HStack {
                    Picker(draftLocalized("transaction.classification"), selection: $draft.classification) {
                        ForEach(TransactionDetailViewModel.classificationOptions, id: \.self) { value in
                            Text(draftLocalized("transaction.classification.\(value.lowercased().replacingOccurrences(of: " ", with: "_"))")).tag(value)
                        }
                    }
                    Picker(draftLocalized("transactionDraft.taxTreatment"), selection: $draft.taxTreatment) {
                        ForEach(TransactionDetailViewModel.taxOptions) { option in
                            Text(draftLocalized("transaction.taxTreatment.\(option.id.replacingOccurrences(of: ":", with: "_"))")).tag(option.id)
                        }
                    }
                }
                TextField(draftLocalized("transaction.tags"), text: $draft.tags)
                TextField(draftLocalized("transaction.note"), text: $draft.note, axis: .vertical)
                    .lineLimit(2...4)
            }
        }
    }

    private var evidence: some View {
        draftSection(draftLocalized("transactionDraft.evidence")) {
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 8) {
                GridRow {
                    TextField(draftLocalized("transactionDraft.txidEvidence"), text: $draft.evidence.transactionReference)
                    TextField(draftLocalized("transactionDraft.btcpayEvidence"), text: $draft.evidence.btcpayInvoiceID)
                }
                GridRow {
                    TextField(draftLocalized("transactionDraft.exchangeEvidence"), text: $draft.evidence.exchangeCSVRow)
                    TextField(draftLocalized("transactionDraft.swapEvidence"), text: $draft.evidence.swapID)
                }
                GridRow {
                    TextField(draftLocalized("transactionDraft.preimageEvidence"), text: $draft.evidence.preimage)
                    Color.clear
                }
            }
        }
    }

    private var preview: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(draftLocalized("transactionDraft.livePreview")).font(.caption).foregroundStyle(.secondary)
            Text(draftLocalized("transactionDraft.flow.\(draft.flow)"))
                .font(.title2.bold())
            Text("\(previewFrom) → \(previewTo)")
                .foregroundStyle(.secondary).lineLimit(2)
            GroupBox {
                VStack(alignment: .leading, spacing: 10) {
                    if draft.isTwoLeg {
                        previewRow(draftLocalized("transactionFlow.outgoing"), String(format: "%.8f %@", draft.sendBTC, draft.sendAsset))
                        previewRow(draftLocalized("transactionFlow.incoming"), String(format: "%.8f %@", draft.receiveBTC, draft.receiveAsset))
                    } else {
                        Text(String(format: "%+.8f BTC", draft.signedBTC))
                            .font(.title2.monospacedDigit())
                            .foregroundStyle(draft.signedBTC > 0 ? .green : draft.signedBTC < 0 ? .red : .primary)
                    }
                    Divider()
                    previewRow(draftLocalized("field.network"), networkLabel(draft.network))
                    previewRow(draftLocalized("transactionDraft.movement"), draft.movementID.isEmpty ? draftLocalized("transactionDraft.standalone") : draft.movementID)
                    previewRow(draftLocalized("transaction.classification"), classificationLabel(draft.classification))
                    previewRow(draftLocalized("transactionDraft.taxTreatment"), taxTreatmentLabel(draft.taxTreatment))
                    previewRow(draftLocalized("transactionDraft.valueLabel"), draft.totalValue.isEmpty ? "—" : "\(draft.totalValue) \(draft.fiatCurrency)")
                    previewRow(draftLocalized("transactionDraft.evidence"), draft.evidence.primary ?? draftLocalized("transactionFlow.none"))
                }
                .kassiberSensitive()
            }
            Spacer()
        }
        .padding(18)
        .background(.quaternary.opacity(0.2))
    }

    private func draftSection<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        GroupBox(title) { content().padding(.top, 4) }
    }

    private func walletPicker(_ title: String, selection: Binding<String>) -> some View {
        Picker(title, selection: selection) {
            ForEach(wallets, id: \.self) { Text($0).tag($0) }
        }
        .frame(maxWidth: .infinity)
    }

    private func previewRow(_ label: String, _ value: String) -> some View {
        LabeledContent(label, value: value)
    }

    private var previewFrom: String {
        if draft.flow == "incoming" {
            return draft.fromExternal.isEmpty ? draftLocalized("transactionDraft.externalParty") : draft.fromExternal
        }
        return draft.fromWallet.isEmpty ? draftLocalized("transactionDraft.unassigned") : draft.fromWallet
    }

    private var previewTo: String {
        if draft.flow == "outgoing" {
            return draft.toExternal.isEmpty ? draftLocalized("transactionDraft.externalParty") : draft.toExternal
        }
        return draft.toWallet.isEmpty ? draftLocalized("transactionDraft.unassigned") : draft.toWallet
    }

    private func networkLabel(_ value: String) -> String {
        let key = "transactionDraft.network.\(value.lowercased())"
        let translated = draftLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private func classificationLabel(_ value: String) -> String {
        let suffix = value.lowercased().replacingOccurrences(of: " ", with: "_")
        let key = "transaction.classification.\(suffix)"
        let translated = draftLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private func taxTreatmentLabel(_ value: String) -> String {
        let key = "transaction.taxTreatment.\(value.replacingOccurrences(of: ":", with: "_"))"
        let translated = draftLocalized(key)
        return translated == key ? AppLocalization.code(value) : translated
    }

    private var confirmedBinding: Binding<Bool> {
        Binding(
            get: { draft.confirmedAt != nil },
            set: { draft.confirmedAt = $0 ? draft.occurredAt : nil }
        )
    }

    private var confirmedDateBinding: Binding<Date> {
        Binding(
            get: { draft.confirmedAt ?? draft.occurredAt },
            set: { draft.confirmedAt = $0 }
        )
    }
}
