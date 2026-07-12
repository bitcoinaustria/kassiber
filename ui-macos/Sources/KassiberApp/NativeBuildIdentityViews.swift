import SwiftUI
import AppKit
import KassiberViewModels

struct NativeBuildIdentityFooter: View {
    let identity: NativeBuildIdentity
    @State private var showingAbout = false

    init(identity: NativeBuildIdentity = NativeBuildIdentity(
        infoDictionary: Bundle.main.infoDictionary ?? [:]
    )) {
        self.identity = identity
    }

    var body: some View {
        Button {
            showingAbout.toggle()
        } label: {
            Label(identity.compactLabel, systemImage: "info.circle")
                .font(.caption2.monospaced())
                .foregroundStyle(.secondary)
        }
        .buttonStyle(.plain)
        .help(AppLocalization.string("buildIdentity.showDetails"))
        .accessibilityLabel(
            String(
                format: AppLocalization.string("buildIdentity.accessibility %@"),
                identity.compactLabel
            )
        )
        .popover(isPresented: $showingAbout) {
            NativeAboutDetail(identity: identity)
        }
    }
}

struct NativeAboutDetail: View {
    let identity: NativeBuildIdentity

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(nsImage: NSApplication.shared.applicationIconImage)
                    .resizable()
                    .frame(width: 42, height: 42)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(identity.productName).font(.headline)
                    Text(AppLocalization.string("buildIdentity.summary"))
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 7) {
                identityRow(AppLocalization.string("buildIdentity.version"), identity.version)
                if let build = identity.build {
                    identityRow(AppLocalization.string("buildIdentity.build"), build)
                }
                if let commit = identity.commit {
                    identityRow(AppLocalization.string("buildIdentity.commit"), commit)
                }
                if let signing = identity.signingStrength {
                    identityRow(
                        AppLocalization.string("buildIdentity.signing"),
                        signingLabel(signing)
                    )
                }
            }
            Link(
                AppLocalization.string("buildIdentity.source"),
                destination: URL(string: "https://github.com/bitcoinaustria/kassiber")!
            )
        }
        .padding(18)
        .frame(width: 390)
    }

    private func identityRow(_ label: String, _ value: String) -> some View {
        GridRow {
            Text(label).foregroundStyle(.secondary)
            Text(value).font(.body.monospaced()).textSelection(.enabled)
        }
    }

    private func signingLabel(_ raw: String) -> String {
        let key = "buildIdentity.signing.\(raw.lowercased())"
        let translated = AppLocalization.string(key)
        return translated == key ? raw : translated
    }
}
