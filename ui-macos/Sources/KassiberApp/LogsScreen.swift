import SwiftUI
import AppKit
import UniformTypeIdentifiers
import KassiberDaemonKit
import KassiberViewModels

private func logsLocalized(_ key: String) -> String { AppLocalization.string(key) }
private func logLevelLabel(_ level: String) -> String { AppLocalization.code(level) }

struct LogsScreen: View {
    @State private var model: LogsViewModel
    @State private var expanded: Set<Int64> = []
    @State private var showRawConfirmation = false
    @State private var pendingExport: LogExportFormat?
    @State private var showRawExportConfirmation = false
    @State private var showSupportBundle = false
    @State private var supportDescription = ""
    @State private var supportMode: LogRedactionMode = .highSignal
    @State private var exportError: String?
    @FocusState private var searchFocused: Bool

    init(daemon: any DaemonClient) {
        _model = State(initialValue: LogsViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            controls
            if model.isRawVisible { rawWarning }
            if model.gap { gapWarning }
            Divider()
            logContent
            Divider()
            statusBar
        }
        .navigationTitle(logsLocalized("nav.logs"))
        .toolbar { toolbarContent }
        .task { await model.pollContinuously() }
        .task(id: model.rawUntil) {
            guard let deadline = model.rawUntil else { return }
            while !Task.isCancelled, Date() < deadline {
                try? await Task.sleep(for: .seconds(1))
            }
            model.expireRawIfNeeded()
        }
        .onExitCommand { model.clearFilters() }
        .sheet(isPresented: $showSupportBundle) {
            SupportBundleSheet(
                model: model,
                issueDescription: $supportDescription,
                mode: $supportMode,
                onSave: saveSupportBundle
            )
        }
        .alert(logsLocalized("logs.rawRevealTitle"), isPresented: $showRawConfirmation) {
            Button(logsLocalized("action.cancel"), role: .cancel) {}
            Button(logsLocalized("logs.revealRaw"), role: .destructive) { model.revealRaw() }
        } message: {
            Text(logsLocalized("logs.rawRevealBody"))
        }
        .alert(logsLocalized("logs.exportRawTitle"), isPresented: $showRawExportConfirmation) {
            Button(logsLocalized("action.cancel"), role: .cancel) { pendingExport = nil }
            Button(logsLocalized("action.export"), role: .destructive) {
                if let pendingExport { saveLogs(pendingExport) }
                pendingExport = nil
            }
        } message: {
            Text(logsLocalized("logs.exportRawConfirm"))
        }
        .alert(logsLocalized("logs.exportFailedTitle"), isPresented: Binding(
            get: { exportError != nil },
            set: { if !$0 { exportError = nil } }
        )) {
            Button(logsLocalized("action.ok")) { exportError = nil }
        } message: {
            Text(exportError ?? "")
        }
        .background(shortcutButtons)
    }

    private var header: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 7) {
                    Text(logsLocalized("logs.developerTools"))
                        .font(.headline)
                    Label(logsLocalized("logs.live"), systemImage: "circle.fill")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.green)
                        .labelStyle(.titleAndIcon)
                }
                Text(logsLocalized("logs.ramNote"))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button(action: copyLast200) {
                Label(logsLocalized("logs.copy200"), systemImage: "doc.on.doc")
            }
            Menu {
                Button(logsLocalized("logs.supportBundle")) { showSupportBundle = true }
                Divider()
                Button(logsLocalized("logs.markdown")) { requestExport(.markdown) }
                Button("JSONL") { requestExport(.jsonl) }
                Button(logsLocalized("logs.logFormat")) { requestExport(.plain) }
            } label: {
                Label(logsLocalized("action.export"), systemImage: "square.and.arrow.up")
            }
            Button(role: .destructive) {
                expanded.removeAll()
                model.clearView()
            } label: {
                Label(logsLocalized("logs.clearLogs"), systemImage: "trash")
            }
            .help(logsLocalized("logs.clearLogs"))
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    private var controls: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField(logsLocalized("logs.searchPlaceholder"), text: $model.query)
                    .textFieldStyle(.roundedBorder)
                    .focused($searchFocused)
                    .frame(minWidth: 220, maxWidth: 420)
                if !model.query.isEmpty {
                    Button { model.query = "" } label: { Image(systemName: "xmark.circle.fill") }
                        .buttonStyle(.plain)
                        .foregroundStyle(.secondary)
                }
                levelPicker
                modulePicker
                Spacer()
                settingsMenu
                Toggle(isOn: Binding(
                    get: { model.autoscroll },
                    set: { model.autoscroll = $0 }
                )) {
                    Label(logsLocalized("logs.follow"), systemImage: "arrow.down.to.line")
                }
                .toggleStyle(.button)
            }
            HStack(spacing: 8) {
                if model.level != "all" { filterChip("\(logsLocalized("logs.level")): \(logLevelLabel(model.level))") { model.level = "all" } }
                if model.module != "all" { filterChip("\(logsLocalized("logs.module")): \(model.module)") { model.module = "all" } }
                if !model.query.isEmpty {
                    filterChip("\(model.useRegex ? logsLocalized("logs.regexPrefix") : "")\(model.query)") { model.query = "" }
                }
                if let regexError = model.regexError {
                    Label(regexError, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .lineLimit(1)
                }
                Spacer()
                if model.level != "all" || model.module != "all" || !model.query.isEmpty {
                    Button(logsLocalized("logs.clearFilters")) { model.clearFilters() }
                        .buttonStyle(.link)
                        .font(.caption)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
    }

    private var levelPicker: some View {
        Menu {
            Button("\(logsLocalized("logs.allLevels")) (\(model.records.count))") { model.level = "all" }
            Divider()
            ForEach(LogsViewModel.levels, id: \.self) { level in
                Button("\(logLevelLabel(level)) (\(model.levelCounts[level, default: 0]))") { model.level = level }
            }
        } label: {
            Label(model.level == "all" ? logsLocalized("logs.allLevels") : logLevelLabel(model.level), systemImage: "line.3.horizontal.decrease")
                .frame(minWidth: 105, alignment: .leading)
        }
    }

    private var modulePicker: some View {
        Menu {
            Button(logsLocalized("logs.allModules")) { model.module = "all" }
            Divider()
            ForEach(model.modules, id: \.self) { module in
                Button("\(module) (\(model.moduleCounts[module, default: 0]))") { model.module = module }
            }
        } label: {
            Label(model.module == "all" ? logsLocalized("logs.allModules") : model.module, systemImage: "shippingbox")
                .lineLimit(1)
                .frame(minWidth: 110, maxWidth: 220, alignment: .leading)
        }
    }

    private var settingsMenu: some View {
        Menu {
            Toggle(logsLocalized("logs.regexSearch"), isOn: $model.useRegex)
            Toggle(logsLocalized("logs.redactedView"), isOn: Binding(
                get: { model.redacted },
                set: { value in
                    if value { model.hideRaw() } else { showRawConfirmation = true }
                }
            ))
            Toggle(logsLocalized("logs.maskAmounts"), isOn: $model.maskAmounts)
            Picker(logsLocalized("logs.bundleMode"), selection: $model.redactionMode) {
                Text(logsLocalized("logs.highSignalTitle")).tag(LogRedactionMode.highSignal)
                Text(logsLocalized("logs.publicSafeTitle")).tag(LogRedactionMode.publicSafe)
            }
        } label: {
            Label(logsLocalized("logs.settings"), systemImage: "gearshape")
        }
    }

    private var rawWarning: some View {
        HStack {
            Image(systemName: "eye.trianglebadge.exclamationmark.fill")
            Text(rawWarningText)
            Spacer()
            Button(logsLocalized("logs.hideRaw")) { model.hideRaw() }
        }
        .font(.caption)
        .foregroundStyle(.orange)
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(Color.orange.opacity(0.10))
    }

    private var rawWarningText: String {
        let time = model.rawUntil?.formatted(date: .omitted, time: .shortened) ?? logsLocalized("logs.rawWarningSoon")
        return String(format: logsLocalized("logs.rawWarning %@"), time)
    }

    private var gapWarning: some View {
        Label(logsLocalized("logs.gapWarning"), systemImage: "exclamationmark.triangle.fill")
            .font(.caption)
            .foregroundStyle(.orange)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
    }

    private var logContent: some View {
        ScrollViewReader { proxy in
            ScrollView([.vertical, .horizontal]) {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if model.filteredRecords.count > model.visibleRecords.count {
                        Button {
                            model.loadOlder()
                        } label: {
                            Label(logsLocalized("logs.loadOlder"), systemImage: "arrow.up")
                                .frame(maxWidth: .infinity)
                                .padding(8)
                        }
                        .buttonStyle(.plain)
                        Divider()
                    }
                    if model.visibleRecords.isEmpty {
                        ContentUnavailableView(
                            model.records.isEmpty ? logsLocalized("logs.waiting") : logsLocalized("logs.noMatching"),
                            systemImage: model.records.isEmpty ? "waveform" : "line.3.horizontal.decrease.circle"
                        )
                        .frame(minWidth: 650, minHeight: 280)
                    } else {
                        ForEach(model.visibleRecords) { record in
                            logRow(record)
                            Divider()
                        }
                        Color.clear.frame(width: 1, height: 1).id("logs-bottom")
                    }
                }
            }
            .onChange(of: model.records.count) { _, _ in
                if model.autoscroll {
                    withAnimation(.easeOut(duration: 0.16)) { proxy.scrollTo("logs-bottom", anchor: .bottom) }
                }
            }
            .overlay(alignment: .bottom) {
                if !model.autoscroll, model.newWhilePaused > 0 {
                    Button {
                        model.jumpToLatest()
                        withAnimation(.easeOut(duration: 0.16)) { proxy.scrollTo("logs-bottom", anchor: .bottom) }
                    } label: {
                        Label(
                            String(format: logsLocalized("logs.jumpLatest %lld"), model.newWhilePaused),
                            systemImage: "arrow.down"
                        )
                    }
                    .buttonStyle(.borderedProminent)
                    .padding(.bottom, 14)
                }
            }
        }
    }

    private func logRow(_ record: LogRecordRow) -> some View {
        let rendered = model.renderedRecord(record)
        let isExpanded = expanded.contains(record.id)
        return VStack(alignment: .leading, spacing: 0) {
            Button {
                if isExpanded { expanded.remove(record.id) } else { expanded.insert(record.id) }
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .frame(width: 10)
                    Text(rendered.timestamp)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .frame(width: 188, alignment: .leading)
                    Text(logLevelLabel(rendered.level).uppercased())
                        .font(.caption2.monospaced().weight(.semibold))
                        .foregroundStyle(levelColor(rendered.level))
                        .frame(width: 76, alignment: .leading)
                    Text(rendered.location)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .frame(width: 240, alignment: .leading)
                    Text(rendered.message)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                    let fields = model.fieldSummary(record)
                    if !fields.isEmpty {
                        Text(fields)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if isExpanded {
                ScrollView(.horizontal) {
                    Text(model.expandedJSON(record))
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .padding(.leading, 42)
                        .padding(.trailing, 12)
                        .padding(.bottom, 9)
                }
            }
        }
        .id(record.id)
    }

    private var statusBar: some View {
        HStack(spacing: 10) {
            Text(String(
                format: logsLocalized("logs.recordsLine %lld %@ %lld"),
                model.records.count,
                ByteCountFormatter.string(fromByteCount: model.bufferBytes, countStyle: .memory),
                model.bufferPercent
            ))
            if model.filteredRecords.count > model.visibleRecords.count {
                Text("·")
                Text(String(format: logsLocalized("logs.renderingNewest %lld"), model.visibleRecords.count))
            }
            Spacer()
            ProgressView(value: Double(model.bufferBytes), total: Double(max(1, model.maxBytes)))
                .frame(width: 110)
            if model.isLoading { ProgressView().controlSize(.small) }
            if let error = model.errorMessage {
                Label(AppLocalization.error(error), systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .lineLimit(1)
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem {
            Button { Task { await model.poll() } } label: {
                Label(logsLocalized("action.refresh"), systemImage: "arrow.clockwise")
            }
            .disabled(model.isLoading)
        }
    }

    private var shortcutButtons: some View {
        HStack {
            Button("") { searchFocused = true }.keyboardShortcut("/", modifiers: [])
            Button("") { copyLast200() }.keyboardShortcut("c", modifiers: [.command, .shift])
            Button("") { requestExport(.markdown) }.keyboardShortcut("e", modifiers: [.command])
        }
        .frame(width: 0, height: 0)
        .opacity(0)
        .accessibilityHidden(true)
    }

    private func filterChip(_ label: String, clear: @escaping () -> Void) -> some View {
        HStack(spacing: 5) {
            Text(label).lineLimit(1)
            Button(action: clear) { Image(systemName: "xmark") }.buttonStyle(.plain)
        }
        .font(.caption)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(.quaternary, in: Capsule())
    }

    private func levelColor(_ level: String) -> Color {
        switch level {
        case "error": .red
        case "warning": .orange
        case "debug", "trace": .secondary
        default: .primary
        }
    }

    private func copyLast200() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(model.copyLast200Text(), forType: .string)
    }

    private func requestExport(_ format: LogExportFormat) {
        if model.isRawVisible {
            pendingExport = format
            showRawExportConfirmation = true
        } else {
            saveLogs(format)
        }
    }

    private func saveLogs(_ format: LogExportFormat) {
        let panel = NSSavePanel()
        panel.title = logsLocalized("logs.exportTitle")
        panel.nameFieldStringValue = model.exportFilename(format: format)
        panel.allowedContentTypes = [contentType(for: format)]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try model.exportText(format: format, header: exportHeader)
                .write(to: url, atomically: true, encoding: .utf8)
        } catch {
            exportError = String(describing: error)
        }
    }

    private func saveSupportBundle() {
        guard let contents = model.supportBundle(
            issueDescription: supportDescription,
            mode: supportMode,
            header: exportHeader
        ) else { return }
        let panel = NSSavePanel()
        panel.title = logsLocalized("logs.exportSupportTitle")
        panel.nameFieldStringValue = model.supportBundleFilename()
        panel.allowedContentTypes = [UTType(filenameExtension: "jsonl") ?? .json]
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try contents.write(to: url, atomically: true, encoding: .utf8)
            showSupportBundle = false
        } catch {
            exportError = String(describing: error)
        }
    }

    private var exportHeader: LogExportHeader {
        LogExportHeader(
            appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev",
            operatingSystem: ProcessInfo.processInfo.operatingSystemVersionString
        )
    }

    private func contentType(for format: LogExportFormat) -> UTType {
        switch format {
        case .markdown: UTType(filenameExtension: "md") ?? .plainText
        case .plain: .plainText
        case .jsonl: UTType(filenameExtension: "jsonl") ?? .json
        }
    }
}

private struct SupportBundleSheet: View {
    let model: LogsViewModel
    @Binding var issueDescription: String
    @Binding var mode: LogRedactionMode
    let onSave: () -> Void
    @Environment(\.dismiss) private var dismiss

    private var header: LogExportHeader {
        LogExportHeader(
            appVersion: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev",
            operatingSystem: ProcessInfo.processInfo.operatingSystemVersionString
        )
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text(logsLocalized("logs.supportTitle")).font(.title2.weight(.semibold))
                    Text(logsLocalized("logs.supportDescription")).foregroundStyle(.secondary)
                }
                Spacer()
                Button { dismiss() } label: { Image(systemName: "xmark") }
                    .buttonStyle(.plain)
            }
            Text(logsLocalized("logs.issueDescription")).font(.headline)
            TextEditor(text: $issueDescription)
                .font(.body)
                .frame(height: 82)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.separator))
            Picker(logsLocalized("logs.bundleMode"), selection: $mode) {
                Text(logsLocalized("logs.highSignalTitle")).tag(LogRedactionMode.highSignal)
                Text(logsLocalized("logs.publicSafeTitle")).tag(LogRedactionMode.publicSafe)
            }
            .pickerStyle(.segmented)
            Text(mode == .highSignal ? logsLocalized("logs.highSignalDescription") : logsLocalized("logs.publicSafeDescription"))
                .font(.caption)
                .foregroundStyle(.secondary)
            HStack {
                Text(logsLocalized("logs.preview")).font(.headline)
                Spacer()
                Text(String(format: logsLocalized("logs.firstLines %lld"), 30)).font(.caption).foregroundStyle(.secondary)
            }
            ScrollView([.vertical, .horizontal]) {
                Text(model.supportBundlePreview(
                    issueDescription: issueDescription,
                    mode: mode,
                    header: header,
                    lineLimit: 30
                ))
                .font(.caption.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
            }
            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 6))
            .frame(minHeight: 220)
            HStack {
                Spacer()
                Button(logsLocalized("action.cancel")) { dismiss() }
                Button(logsLocalized("logs.saveBundle"), action: onSave)
                    .buttonStyle(.borderedProminent)
                    .disabled(issueDescription.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(20)
        .frame(minWidth: 760, minHeight: 620)
    }
}
