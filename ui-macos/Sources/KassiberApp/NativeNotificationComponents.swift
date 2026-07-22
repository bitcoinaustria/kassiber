import SwiftUI
import KassiberViewModels

private func notificationLocalized(_ key: String) -> String { AppLocalization.string(key) }

struct NativeNotificationBell: View {
    @Bindable var store: NativeNotificationStore
    var onNavigate: (AppScreen) -> Void
    var onProcessJournals: () -> Void
    var onRestoreBookRefresh: () -> Void
    @State private var isPresented = false

    var body: some View {
        Button { isPresented.toggle() } label: {
            ZStack(alignment: .topTrailing) {
                Image(systemName: store.count > 0 ? "bell.fill" : "bell")
                if store.count > 0 {
                    Text(min(store.count, 99), format: .number)
                        .font(.system(size: 8, weight: .bold))
                        .foregroundStyle(.white)
                        .padding(3)
                        .background(toneColor, in: Circle())
                        .offset(x: 7, y: -7)
                }
            }
        }
        .help(store.count > 0
              ? String(format: notificationLocalized("notifications.items %lld"), store.count)
              : notificationLocalized("notifications.title"))
        .popover(isPresented: $isPresented, arrowEdge: .bottom) {
            NativeNotificationCenterPanel(
                store: store,
                onNavigate: { screen in
                    isPresented = false
                    onNavigate(screen)
                },
                onProcessJournals: {
                    isPresented = false
                    onProcessJournals()
                },
                onRestoreBookRefresh: {
                    isPresented = false
                    onRestoreBookRefresh()
                }
            )
        }
    }

    private var toneColor: Color {
        let tones = store.allNotifications.map(\.tone)
        if tones.contains(.error) { return .red }
        if tones.contains(.warning) { return .orange }
        return .accentColor
    }
}

struct NativeNotificationCenterPanel: View {
    @Bindable var store: NativeNotificationStore
    var onNavigate: (AppScreen) -> Void
    var onProcessJournals: () -> Void
    var onRestoreBookRefresh: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(notificationLocalized("notifications.title")).font(.headline)
                Spacer()
                if !store.notifications.isEmpty {
                    Button(notificationLocalized("notifications.clearAll")) { store.clearAll() }
                        .buttonStyle(.link)
                        .font(.caption)
                }
            }
            .padding(14)
            Divider()
            if store.allNotifications.isEmpty {
                ContentUnavailableView(
                    notificationLocalized("notifications.empty"),
                    systemImage: "bell.slash",
                    description: Text(notificationLocalized("notifications.empty.body"))
                )
                .frame(height: 180)
            } else {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(store.allNotifications) { item in
                            notificationRow(item)
                            Divider().padding(.leading, 44)
                        }
                    }
                }
                .frame(maxHeight: 440)
            }
        }
        .frame(width: 390)
    }

    private func notificationRow(_ item: NativeAppNotification) -> some View {
        Button { perform(item) } label: {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: symbol(item.tone))
                    .foregroundStyle(color(item.tone))
                    .frame(width: 20)
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(notificationLocalized(item.titleKey)).font(.subheadline.weight(.semibold))
                        Spacer()
                        if item.category != .review {
                            Text(item.createdAt, style: .relative)
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                    Text(body(item)).font(.caption).foregroundStyle(.secondary)
                    if let detail = item.ephemeralDetail, !detail.isEmpty {
                        Text(detail).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(2)
                    }
                    if let progress = item.progress {
                        ProgressView(value: progress.indeterminate ? nil : progress.value)
                            .controlSize(.small)
                        if !progress.label.isEmpty {
                            Text(progress.label).font(.caption2).foregroundStyle(.tertiary).lineLimit(1)
                        }
                    }
                }
                if item.action != nil || item.target != nil {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .padding(.top, 4)
                }
            }
            .padding(12)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }

    private func perform(_ item: NativeAppNotification) {
        switch item.action {
        case .processJournals: onProcessJournals()
        case .restoreBookRefresh: onRestoreBookRefresh()
        case .open, .none:
            if let screen = item.target?.screen { onNavigate(screen) }
        }
    }

    private func body(_ item: NativeAppNotification) -> String {
        if let count = item.count {
            return String(format: notificationLocalized(item.bodyKey), count)
        }
        return notificationLocalized(item.bodyKey)
    }

    private func symbol(_ tone: NativeNotificationTone) -> String {
        switch tone {
        case .info: "info.circle.fill"
        case .success: "checkmark.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .error: "xmark.octagon.fill"
        }
    }

    private func color(_ tone: NativeNotificationTone) -> Color {
        switch tone {
        case .info: .accentColor
        case .success: .green
        case .warning: .orange
        case .error: .red
        }
    }
}

/// Compact transient mutation outcomes for the shell overlay. Durable review
/// state remains in the bell and is never represented as a toast.
struct NativeNotificationToastRail: View {
    @Bindable var store: NativeNotificationStore

    var body: some View {
        VStack(alignment: .trailing, spacing: 8) {
            ForEach(store.notifications.filter(\.transient).prefix(3)) { item in
                HStack(spacing: 8) {
                    Image(systemName: item.tone == .error ? "xmark.octagon.fill" : "checkmark.circle.fill")
                        .foregroundStyle(item.tone == .error ? .red : .green)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(notificationLocalized(item.titleKey)).font(.caption.weight(.semibold))
                        if let detail = item.ephemeralDetail {
                            Text(detail).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                    Button { store.remove(item.id) } label: { Image(systemName: "xmark") }
                        .buttonStyle(.plain)
                }
                .padding(10)
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
                .shadow(radius: 5, y: 2)
            }
        }
    }
}
