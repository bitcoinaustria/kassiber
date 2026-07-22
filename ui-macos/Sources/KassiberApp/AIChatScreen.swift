import SwiftUI
import AppKit
import UniformTypeIdentifiers
import Textual
import KassiberDaemonKit
import KassiberViewModels

private func chatLocalized(_ key: String) -> String { AppLocalization.string(key) }

private func chatActivityLabel(_ value: String) -> String {
    let isStableCode = !value.isEmpty && value.allSatisfy {
        $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" || $0 == "."
    }
    return isStableCode ? AppLocalization.code(value) : value
}

struct AIChatScreen: View {
    let daemon: any DaemonClient
    let compact: Bool
    @State private var model: AIChatViewModel
    @State private var showingHistory = false
    @State private var editingMessage: ChatMessage?
    @State private var editedText = ""
    @Environment(\.locale) private var locale

    init(daemon: any DaemonClient, sharedModel: AIChatViewModel? = nil, compact: Bool = false) {
        self.daemon = daemon
        self.compact = compact
        _model = State(initialValue: sharedModel ?? AIChatViewModel(daemon: daemon))
    }

    var body: some View {
        VStack(spacing: 0) {
            if compact {
                compactToolbar
            } else {
                HStack {
                Picker(chatLocalized("chat.provider"), selection: $model.provider) {
                    if model.providers.isEmpty { Text(chatLocalized("state.unavailable")).tag("") }
                    ForEach(model.providers, id: \.self) { Text($0).tag($0) }
                }
                .frame(width: 210)
                ComboBox(label: chatLocalized("chat.model"), text: $model.model, choices: model.models)
                    .frame(width: 240)
                Toggle(chatLocalized("chat.allowReadTools"), isOn: $model.readToolsEnabled)
                    .help(chatLocalized("chat.allowReadToolsHelp"))
                thinkingEffortPicker
                if model.sessionID == nil {
                    Toggle(chatLocalized("chat.incognito"), isOn: $model.incognito)
                        .help(chatLocalized("chat.incognitoHelp"))
                }
                Spacer()
                if !model.queuedPrompts.isEmpty {
                    Label(
                        String(format: chatLocalized("chat.queuedCount %lld"), Int64(model.queuedPrompts.count)),
                        systemImage: "text.badge.plus"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                Button { showingHistory = true; Task { await model.loadSessions() } } label: { Label(chatLocalized("chat.history"), systemImage: "clock.arrow.circlepath") }
                if !model.messages.isEmpty {
                    Button { exportChat() } label: { Label(chatLocalized("chat.export"), systemImage: "square.and.arrow.up") }
                }
                if !model.messages.isEmpty {
                    Button(chatLocalized("chat.clear")) { model.reset() }
                        .disabled(model.isStreaming)
                }
                }
                .padding(10)
            }
            Divider()
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: compact ? 12 : 18) {
                        if model.messages.isEmpty {
                            ContentUnavailableView {
                                Label(chatLocalized("chat.heading"), systemImage: "sparkles")
                            } description: {
                                Text(chatLocalized("chat.emptyDetail"))
                            }
                            .frame(maxWidth: .infinity, minHeight: compact ? 180 : 380)
                        }
                        ForEach(model.messages) { message in
                            ChatMessageView(message: message, branch: { model.branch(from: message.id) }, edit: {
                                editedText = message.content
                                editingMessage = message
                            })
                                .id(message.id)
                        }
                    }
                    .frame(maxWidth: compact ? .infinity : 780)
                    .padding(compact ? 12 : 24)
                    .frame(maxWidth: .infinity)
                }
                .onChange(of: model.messages) { _, messages in
                    if let id = messages.last?.id { withAnimation { proxy.scrollTo(id, anchor: .bottom) } }
                }
            }
            if let error = model.errorMessage {
                Text(AppLocalization.error(error)).foregroundStyle(.red).font(.caption).padding(.horizontal)
            }
            Divider()
            HStack(alignment: .bottom) {
                TextEditor(text: $model.draft)
                    .font(.body)
                    .frame(minHeight: 42, maxHeight: compact ? 80 : 110)
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(.separator))
                if model.isStreaming {
                    Button { Task { await model.stop() } } label: {
                        Label(chatLocalized("chat.stop"), systemImage: "stop.fill")
                    }
                    Button { Task { await model.send() } } label: {
                        Label(chatLocalized("chat.queue"), systemImage: "text.badge.plus")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .keyboardShortcut(.return, modifiers: .command)
                } else {
                    Button { Task { await model.send() } } label: {
                        Label(chatLocalized("chat.send"), systemImage: "arrow.up.circle.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || model.model.isEmpty)
                    .keyboardShortcut(.return, modifiers: .command)
                }
            }
            .padding(compact ? 10 : 14)
        }
        .navigationTitle(chatLocalized("nav.assistant"))
        .task {
            await model.loadProviders()
            await model.loadModels()
            if ProcessInfo.processInfo.environment["KASSIBER_PREVIEW_AI"] == "1" {
                model.loadPreviewConversation()
            }
        }
        .onChange(of: model.provider) { _, _ in
            Task { await model.loadModels() }
        }
        .sheet(item: Binding(
            get: { model.pendingConsent },
            set: { _ in }
        )) { request in
            ToolConsentSheet(request: request) { decision in
                Task { await model.decideConsent(decision) }
            }
        }
        .sheet(isPresented: $showingHistory) {
            NavigationStack {
                List(model.sessions) { session in
                    HStack {
                        Button { Task { await model.resume(session); showingHistory = false } } label: {
                            VStack(alignment: .leading) {
                                Text(session.title)
                                Text(sessionSummary(session))
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }.buttonStyle(.plain)
                        Spacer()
                        Button(role: .destructive) { Task { await model.delete(session) } } label: { Image(systemName: "trash") }
                    }.padding(.vertical, 4)
                }
                .navigationTitle(chatLocalized("chat.savedChats"))
                .toolbar { ToolbarItem(placement: .cancellationAction) { Button(chatLocalized("action.cancel")) { showingHistory = false } }; ToolbarItem(placement: .destructiveAction) { Button(chatLocalized("chat.clearHistory"), role: .destructive) { Task { await model.clearSessions() } } } }
            }.frame(width: 520, height: 520)
        }
        .sheet(item: $editingMessage) { message in
            VStack(alignment: .leading, spacing: 14) {
                Text(chatLocalized("chat.editMessage")).font(.title2)
                TextEditor(text: $editedText).frame(minHeight: 160)
                HStack {
                    Spacer()
                    Button(chatLocalized("action.cancel")) { editingMessage = nil }
                    Button(chatLocalized("chat.editAndRetry")) {
                        let replacement = editedText
                        editingMessage = nil
                        Task { await model.editAndRetry(message.id, content: replacement) }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(editedText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }.padding(24).frame(width: 560)
        }
    }

    private var compactToolbar: some View {
        VStack(spacing: 8) {
            HStack {
                Picker(chatLocalized("chat.provider"), selection: $model.provider) {
                    if model.providers.isEmpty { Text(chatLocalized("state.unavailable")).tag("") }
                    ForEach(model.providers, id: \.self) { Text($0).tag($0) }
                }
                .frame(maxWidth: 180)
                ComboBox(label: chatLocalized("chat.model"), text: $model.model, choices: model.models)
                    .frame(maxWidth: 210)
            }
            HStack {
                Toggle(chatLocalized("chat.allowReadTools"), isOn: $model.readToolsEnabled)
                    .help(chatLocalized("chat.allowReadToolsHelp"))
                thinkingEffortPicker
                if model.sessionID == nil {
                    Toggle(chatLocalized("chat.incognito"), isOn: $model.incognito)
                        .help(chatLocalized("chat.incognitoHelp"))
                }
                Spacer()
                if !model.queuedPrompts.isEmpty {
                    Text(String(format: chatLocalized("chat.queuedCount %lld"), Int64(model.queuedPrompts.count)))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Button { showingHistory = true; Task { await model.loadSessions() } } label: {
                    Image(systemName: "clock.arrow.circlepath")
                }.help(chatLocalized("chat.history"))
                if !model.messages.isEmpty {
                    Button { exportChat() } label: { Image(systemName: "square.and.arrow.up") }
                        .help(chatLocalized("chat.export"))
                    Button { model.reset() } label: { Image(systemName: "trash") }
                        .help(chatLocalized("chat.clear")).disabled(model.isStreaming)
                }
            }
        }.padding(10)
    }

    private var thinkingEffortPicker: some View {
        Picker(chatLocalized("chat.thinkingEffort"), selection: $model.thinkingEffort) {
            ForEach(ChatThinkingEffort.allCases) { effort in
                Text(chatLocalized("chat.thinking.\(effort.rawValue)")).tag(effort)
            }
        }
        .pickerStyle(.menu)
        .fixedSize()
    }

    private func exportChat() {
        let panel = NSSavePanel(); panel.nameFieldStringValue = "kassiber-chat.md"; panel.allowedContentTypes = [.plainText]
        if panel.runModal() == .OK, let url = panel.url { try? model.markdownExport.write(to: url, atomically: true, encoding: .utf8) }
    }

    private func sessionSummary(_ session: ChatSessionRow) -> String {
        guard let updatedAt = session.updatedAt else {
            return String(format: chatLocalized("chat.messagesCount %lld"), Int64(session.messageCount))
        }
        let date = updatedAt.formatted(
            .dateTime.year().month(.abbreviated).day().hour().minute().locale(locale)
        )
        return String(
            format: chatLocalized("chat.sessionSummary %lld %@"),
            Int64(session.messageCount),
            date
        )
    }
}

private struct ComboBox: NSViewRepresentable {
    let label: String
    @Binding var text: String
    let choices: [String]

    func makeCoordinator() -> Coordinator { Coordinator(text: $text) }
    func makeNSView(context: Context) -> NSComboBox {
        let box = NSComboBox()
        box.placeholderString = label
        box.isEditable = true
        box.delegate = context.coordinator
        return box
    }
    func updateNSView(_ box: NSComboBox, context: Context) {
        box.removeAllItems()
        box.addItems(withObjectValues: choices)
        if box.stringValue != text { box.stringValue = text }
    }
    final class Coordinator: NSObject, NSComboBoxDelegate {
        @Binding var text: String
        init(text: Binding<String>) { _text = text }
        func controlTextDidChange(_ notification: Notification) {
            guard let box = notification.object as? NSComboBox else { return }
            text = box.stringValue
        }
    }
}

private struct ChatMessageView: View {
    let message: ChatMessage
    let branch: () -> Void
    let edit: () -> Void

    var body: some View {
        HStack(alignment: .top) {
            if message.role == .assistant {
                Image(systemName: "sparkles").foregroundStyle(.tint).frame(width: 24)
            } else {
                Spacer(minLength: 80)
            }
            VStack(alignment: .leading, spacing: 8) {
                if message.role == .assistant {
                    StructuredText(markdown: message.content.isEmpty ? "…" : message.content)
                        .textual.structuredTextStyle(.gitHub)
                        .textual.textSelection(.enabled)
                } else {
                    Text(message.content).textSelection(.enabled)
                }
                ForEach(message.thinkingSegments.indices, id: \.self) { index in
                    let segment = message.thinkingSegments[index]
                    if !segment.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        DisclosureGroup {
                            StructuredText(markdown: segment.content)
                                .textual.structuredTextStyle(.gitHub)
                                .textual.textSelection(.enabled)
                                .padding(.top, 4)
                        } label: {
                            Label(
                                String(format: chatLocalized("chat.thinkingRound %lld"), Int64(index + 1)),
                                systemImage: "brain"
                            )
                            .font(.caption)
                        }
                    }
                }
                ForEach(message.tools) { tool in
                    DisclosureGroup {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(jsonText(tool.arguments))
                                .font(.system(.caption, design: .monospaced))
                                .textSelection(.enabled)
                            if let result = tool.result {
                                Divider()
                                Text(chatLocalized("field.details"))
                                    .font(.caption.bold())
                                Text(jsonText(result))
                                    .font(.system(.caption, design: .monospaced))
                                    .textSelection(.enabled)
                            }
                        }
                    } label: {
                        Label("\(tool.name) · \(AppLocalization.code(tool.status))", systemImage: tool.status == "done" ? "checkmark.circle" : "wrench.and.screwdriver")
                            .font(.caption)
                    }
                }
                if !message.activity.isEmpty {
                    Text(chatActivityLabel(message.activity)).font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(12)
            .background(message.role == .user ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
            if message.role == .assistant { Spacer(minLength: 80) }
        }
        .contextMenu {
            Button(chatLocalized("chat.copyMessage")) { NativeAffordances.copy(message.content) }
            Button(chatLocalized("chat.branchHere"), action: branch)
            if message.role == .user { Button(chatLocalized("chat.editMessage"), action: edit) }
        }
    }
}

private struct ToolConsentSheet: View {
    let request: ToolConsentRequest
    let decide: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(chatLocalized("chat.consentTitle"), systemImage: "checkmark.shield")
                .font(.title2.bold())
            Text(request.summary)
            GroupBox(request.name) {
                Text(jsonText(request.arguments))
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(6)
            }
            Text(chatLocalized("chat.consentDetail")).font(.caption).foregroundStyle(.secondary)
            HStack {
                Button(chatLocalized("chat.deny"), role: .destructive) { decide("deny") }
                Spacer()
                Button(chatLocalized("chat.allowSession")) { decide("allow_session") }
                Button(chatLocalized("chat.allowOnce")) { decide("allow_once") }
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(24)
        .frame(width: 520)
    }
}

private func jsonText(_ value: JSONValue) -> String {
    guard let data = try? JSONEncoder().encode(value),
          let text = String(data: data, encoding: .utf8) else { return "{}" }
    return text
}
