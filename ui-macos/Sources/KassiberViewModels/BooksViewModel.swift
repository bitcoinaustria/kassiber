import Foundation
import Observation
import KassiberDaemonKit

public struct BookRow: Identifiable, Equatable, Sendable {
    public let id: String
    public let workspaceID: String
    public var name: String
    public var fiatCurrency: String
    public var taxCountry: String
    public var gainsAlgorithm: String
    public var wallets: Int
    public var accounts: Int
    public var active: Bool
}

public struct WorkspaceRow: Identifiable, Equatable, Sendable {
    public let id: String
    public var name: String
    public var currency: String
    public var jurisdiction: String
    public var created: String
    public var books: [BookRow]
}

@MainActor
@Observable
public final class BooksViewModel {
    public private(set) var workspaces: [WorkspaceRow] = []
    public private(set) var activeProfileID = ""
    public private(set) var activeWorkspaceID = ""
    public private(set) var isWorking = false
    public private(set) var errorMessage: String?
    private let daemon: any DaemonClient
    public init(daemon: any DaemonClient) { self.daemon = daemon }

    public var allBooks: [BookRow] { workspaces.flatMap(\.books) }
    public var activeBook: BookRow? {
        allBooks.first(where: { $0.id == activeProfileID || $0.active })
    }

    public func load() async {
        await perform(refresh: false) { try await self.daemon.invoke(.uiProfilesSnapshot, args: nil) }
    }

    public func createWorkspace(label: String) async {
        await perform { try await self.daemon.invoke(.uiWorkspaceCreate, args: ["label": .string(label)]) }
    }

    public func renameWorkspace(_ id: String, label: String) async {
        await perform { try await self.daemon.invoke(.uiWorkspaceRename, args: ["workspace_id": .string(id), "label": .string(label)]) }
    }

    public func createBook(workspaceID: String, label: String, country: String, algorithm: String) async {
        await perform { try await self.daemon.invoke(.uiProfilesCreate, args: [
            "workspace_id": .string(workspaceID), "label": .string(label),
            "tax_country": .string(country), "gains_algorithm": .string(algorithm),
        ]) }
    }

    public func updateBook(_ id: String, label: String, country: String, algorithm: String) async {
        await perform(refresh: false) {
            let update = try await self.daemon.invoke(.uiProfilesUpdate, args: [
                "profile_id": .string(id), "tax_country": .string(country), "gains_algorithm": .string(algorithm),
            ])
            if let error = update.error { throw error }
            return try await self.daemon.invoke(.uiProfilesRename, args: ["profile_id": .string(id), "label": .string(label)])
        }
        if errorMessage == nil { await load() }
    }

    public func switchBook(_ id: String) async {
        await perform { try await self.daemon.invoke(.uiProfilesSwitch, args: ["profile_id": .string(id)]) }
    }

    public func handleHostEvent(_ event: DaemonRecord) async {
        guard !isWorking, Self.invalidatesCatalog(event) else { return }
        await load()
    }

    public static func invalidatesCatalog(_ event: DaemonRecord) -> Bool {
        guard event.kind == "native.request.activity",
              let data = event.data?.objectValue,
              data.string("state") == "finished",
              let kind = data.string("kind") else { return false }
        return [
            "ui.profiles.create", "ui.profiles.rename", "ui.profiles.update",
            "ui.profiles.switch", "ui.profiles.reset_data",
            "ui.workspace.create", "ui.workspace.rename", "ui.workspace.delete",
            "ui.projects.create", "ui.projects.select",
        ].contains(kind)
    }

    private func perform(refresh: Bool = true, _ operation: () async throws -> DaemonRecord) async {
        isWorking = true
        defer { isWorking = false }
        do {
            let result = try await operation()
            if let error = result.error { errorMessage = error.message; return }
            errorMessage = nil
            if refresh { await parseFreshSnapshot() } else { parse(result.data) }
        } catch { errorMessage = String(describing: error) }
    }

    private func parseFreshSnapshot() async {
        do {
            let snapshot = try await daemon.invoke(.uiProfilesSnapshot, args: nil)
            if let error = snapshot.error { errorMessage = error.message; return }
            parse(snapshot.data)
        } catch { errorMessage = String(describing: error) }
    }

    private func parse(_ data: JSONValue?) {
        guard let object = data?.objectValue, object["workspaces"] != nil else { return }
        activeProfileID = object.string("activeProfileId", "active_profile_id") ?? ""
        activeWorkspaceID = object.string("activeWorkspaceId", "active_workspace_id") ?? ""
        workspaces = object.objects("workspaces").compactMap { workspace -> WorkspaceRow? in
            guard let id = workspace.string("id") else { return nil }
            let books = workspace.objects("profiles").compactMap { profile -> BookRow? in
                guard let profileID = profile.string("id") else { return nil }
                return BookRow(id: profileID, workspaceID: id, name: profile.string("name", "label") ?? profileID,
                    fiatCurrency: profile.string("fiatCurrency", "fiat_currency") ?? "",
                    taxCountry: profile.string("taxCountry", "tax_country") ?? "generic",
                    gainsAlgorithm: profile.string("gainsAlgorithm", "gains_algorithm") ?? "fifo", wallets: Int(profile.int("wallets") ?? 0),
                    accounts: Int(profile.int("accounts") ?? 0), active: profile.bool("active") ?? (profileID == activeProfileID))
            }
            return WorkspaceRow(id: id, name: workspace.string("name", "label") ?? id, currency: workspace.string("currency") ?? "",
                jurisdiction: workspace.string("jurisdiction") ?? "", created: workspace.string("created") ?? "", books: books)
        }
    }
}
