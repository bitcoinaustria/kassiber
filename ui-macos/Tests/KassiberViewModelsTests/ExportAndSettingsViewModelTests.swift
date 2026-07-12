import Foundation
import Testing
import KassiberDaemonKit
@testable import KassiberViewModels

@Suite("Exports and layered settings")
@MainActor
struct ExportAndSettingsViewModelTests {
    @Test("export coordinator preserves the daemon-managed source artifact")
    func exportArtifact() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiReportsExportPdf: [DaemonRecord(kind: "ui.reports.export_pdf", data: [
                "file": "/tmp/report.pdf", "filename": "report.pdf", "format": "pdf",
            ])],
        ])
        let model = ReportExportViewModel(daemon: client)

        await model.export(.uiReportsExportPdf)

        #expect(model.artifact?.sourceURL == URL(fileURLWithPath: "/tmp/report.pdf"))
        #expect(model.artifact?.filename == "report.pdf")
    }

    @Test("connection inventory separates base, Lightning, and Liquid layers")
    func connectionLayers() async throws {
        let client = ScriptedDaemonClient(scripts: [
            .uiBackendsList: [DaemonRecord(kind: "ui.backends.list", data: [
                "summary": ["workspace": "Books", "profile": "Private"],
                "backends": [
                    ["name": "base", "kind": "electrum", "chain": "bitcoin", "network": "main", "has_url": true],
                    ["name": "node", "kind": "lnd", "chain": "bitcoin", "network": "main", "has_url": true, "has_token": true],
                    ["name": "liquid", "kind": "liquid-esplora", "chain": "liquid", "network": "liquidv1", "has_url": true],
                ],
            ])],
        ])
        let model = ConnectionSettingsViewModel(daemon: client)
        await model.load()

        #expect(model.visibleEndpoints.map(\.name) == ["base"])
        model.layer = .lightning
        #expect(model.visibleEndpoints.map(\.name) == ["node"])
        #expect(model.visibleEndpoints.first?.usesCredentials == true)
        model.layer = .liquid
        #expect(model.visibleEndpoints.map(\.name) == ["liquid"])
    }
}
