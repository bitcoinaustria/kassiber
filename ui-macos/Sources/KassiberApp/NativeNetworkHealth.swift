import SwiftUI
import Foundation
import Network
import KassiberDaemonKit
import KassiberViewModels

/// Process-local NWPathMonitor adapter. The view-model sees only the coarse
/// online/offline state; interface names, addresses, and routes never leave the
/// app target.
final class NativeNWPathSource: NativeNetworkPathSource, @unchecked Sendable {
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "at.bitcoinaustria.kassiber.native.network-path")
    private let lock = NSLock()
    private var state: NativeNetworkPathState = .unknown
    private var continuations: [UUID: AsyncStream<NativeNetworkPathState>.Continuation] = [:]
    private var started = false

    func currentState() -> NativeNetworkPathState {
        lock.lock()
        defer { lock.unlock() }
        return state
    }

    func updates() -> AsyncStream<NativeNetworkPathState> {
        let id = UUID()
        return AsyncStream { [weak self] continuation in
            guard let self else {
                continuation.finish()
                return
            }
            lock.lock()
            continuations[id] = continuation
            let current = state
            let shouldStart = !started
            if shouldStart { started = true }
            lock.unlock()

            continuation.yield(current)
            continuation.onTermination = { [weak self] _ in
                self?.removeContinuation(id)
            }
            if shouldStart {
                monitor.pathUpdateHandler = { [weak self] path in
                    self?.publish(path.status == .satisfied ? .online : .offline)
                }
                monitor.start(queue: queue)
            }
        }
    }

    deinit {
        monitor.cancel()
        lock.lock()
        let currentContinuations = Array(continuations.values)
        continuations.removeAll()
        lock.unlock()
        currentContinuations.forEach { $0.finish() }
    }

    private func publish(_ next: NativeNetworkPathState) {
        lock.lock()
        state = next
        let currentContinuations = Array(continuations.values)
        lock.unlock()
        currentContinuations.forEach { $0.yield(next) }
    }

    private func removeContinuation(_ id: UUID) {
        lock.lock()
        continuations.removeValue(forKey: id)
        lock.unlock()
    }
}

/// Actionable-only global connection health chrome. Once every checked source
/// is healthy the control disappears; there is no standing green indicator.
struct GlobalNetworkHealthIndicator: View {
    @State private var model: GlobalNetworkHealthViewModel
    let identityEpoch: String
    let openSettings: (String) -> Void

    init(
        daemon: any DaemonClient,
        identityEpoch: String,
        openSettings: @escaping (String) -> Void
    ) {
        _model = State(initialValue: GlobalNetworkHealthViewModel(
            pathSource: NativeNWPathSource(),
            checker: DaemonNativeEndpointHealthChecker(daemon: daemon)
        ))
        self.identityEpoch = identityEpoch
        self.openSettings = openSettings
    }

    var body: some View {
        Group {
            if model.isActionable {
                Menu {
                    Label(overallLabel, systemImage: overallSymbol)
                    Divider()
                    if model.actionableRows.isEmpty {
                        Text(AppLocalization.string("network.native.noEndpointDetails"))
                    } else {
                        ForEach(model.actionableRows) { row in
                            Button {
                                openSettings(row.endpoint.settingsSection)
                            } label: {
                                Label {
                                    VStack(alignment: .leading) {
                                        Text(row.endpoint.label)
                                        Text(rowLabel(row.state))
                                    }
                                } icon: {
                                    Image(systemName: rowSymbol(row.state))
                                }
                            }
                        }
                    }
                    Divider()
                    Button {
                        Task { await model.probeAll() }
                    } label: {
                        Label(
                            AppLocalization.string("network.native.checkNow"),
                            systemImage: "arrow.clockwise"
                        )
                    }
                    .disabled(!model.canProbe)
                    if model.loadFailed {
                        Button {
                            Task { await model.reloadAndProbe() }
                        } label: {
                            Label(
                                AppLocalization.string("action.retry"),
                                systemImage: "arrow.counterclockwise"
                            )
                        }
                    }
                } label: {
                    ZStack(alignment: .topTrailing) {
                        Image(systemName: overallSymbol)
                            .foregroundStyle(indicatorColor)
                        if model.issueCount > 0 {
                            Text(model.issueCount, format: .number)
                                .font(.system(size: 8, weight: .bold))
                                .foregroundStyle(.white)
                                .padding(3)
                                .background(.red, in: Circle())
                                .offset(x: 7, y: -7)
                        }
                    }
                }
                .help(overallLabel)
                .accessibilityLabel(overallLabel)
            }
        }
        .task(id: identityEpoch) {
            await model.reloadAndProbe()
            await model.startMonitoring()
        }
        .onDisappear { model.stopMonitoring() }
    }

    private var overallLabel: String {
        if model.pathState == .offline {
            return AppLocalization.string("network.native.offline")
        }
        if model.loadFailed {
            return AppLocalization.string("network.native.loadFailed")
        }
        if model.isLoading || model.isChecking {
            return AppLocalization.string("network.native.checking")
        }
        if model.issueCount > 0 {
            return String(
                format: AppLocalization.string("network.native.issues %lld"),
                Int64(model.issueCount)
            )
        }
        return AppLocalization.string("network.native.needsCheck")
    }

    private var overallSymbol: String {
        if model.pathState == .offline { return "wifi.slash" }
        if model.isLoading || model.isChecking { return "arrow.triangle.2.circlepath" }
        return "wifi.exclamationmark"
    }

    private var indicatorColor: Color {
        model.pathState == .offline || model.loadFailed || model.issueCount > 0
            ? .red : .orange
    }

    private func rowLabel(_ state: NativeEndpointHealthState) -> String {
        switch state {
        case .unchecked: AppLocalization.string("network.native.unchecked")
        case .checking: AppLocalization.string("network.native.checking")
        case .healthy: AppLocalization.string("network.native.healthy")
        case .unhealthy: AppLocalization.string("network.native.unhealthy")
        }
    }

    private func rowSymbol(_ state: NativeEndpointHealthState) -> String {
        switch state {
        case .unchecked: "questionmark.circle"
        case .checking: "arrow.clockwise"
        case .healthy: "checkmark.circle"
        case .unhealthy: "exclamationmark.triangle"
        }
    }
}
