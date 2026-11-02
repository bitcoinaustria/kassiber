import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import "@/i18n";

import { ReportFilesPanel } from "./ReportFilesPanel";
import type { AuditPackageOptions } from "./reportModel";

const audit: AuditPackageOptions = {
  scope: "active_profile",
  caseId: "",
  include: {
    copiedAttachments: true,
    urlReferences: true,
    journalState: true,
    reviewState: true,
    editHistory: false,
  },
};

function render(props: Partial<ComponentProps<typeof ReportFilesPanel>> = {}) {
  return renderToStaticMarkup(
    <ReportFilesPanel
      year={2025}
      region="AT"
      activeExport={null}
      successfulExport={null}
      exportStatus={null}
      openableExportPath={null}
      openingExportPath={null}
      onExport={() => {}}
      onOpenExport={() => {}}
      onReviewQuarantine={() => {}}
      wallets={[
        { id: "w1", label: "Vault", chain: "bitcoin" },
        { id: "w2", label: "Spending", chain: "bitcoin" },
      ]}
      selectedWalletIds={["w1", "w2"]}
      onToggleWallet={() => {}}
      onSelectAllWallets={() => {}}
      includeCover
      onIncludeCoverChange={() => {}}
      audit={audit}
      onAuditChange={() => {}}
      sourceFundsCases={[]}
      {...props}
    />,
  );
}

describe("ReportFilesPanel", () => {
  it("labels every format button with its file and format", () => {
    const html = render();

    for (const label of [
      "Export Tax report as PDF",
      "Export Tax report as XLSX",
      "Export Tax report as CSV",
      "Export Bookkeeping handoff as XLSX",
      "Export Summary as PDF",
      "Export Audit package",
    ]) {
      expect(html).toContain(`aria-label="${label}"`);
    }
    expect(html).toContain("No keys or credentials");
    expect(html).toContain('title="No file includes wallet descriptors, xpubs,');
    // The ledger's own exports live on Transactions.
    expect(html).not.toContain("Export Transactions");
  });

  it("says a generic book's tax report covers every year", () => {
    expect(render()).toContain("E 1kv 2025 with every row");
    expect(render({ region: "GENERIC" })).toContain("Every year in the book");
  });

  it("summarises the folded options", () => {
    expect(render()).toContain("All wallets · balance cover page");
    expect(render({ selectedWalletIds: ["w1"], includeCover: false })).toContain(
      "1 of 2 wallets · no cover page",
    );
    expect(render()).toContain("Whole book · 4 of 5 parts");
  });

  it("disables every format while one export runs and marks the finished one", () => {
    const running = render({ activeExport: "pdf" });
    expect(running.match(/<button[^>]*disabled=""[^>]*aria-label="Export/g)).toHaveLength(6);

    const done = render({ successfulExport: "csv" });
    expect(done).not.toContain('disabled=""');
    const buttons = done.match(/<button[^>]*aria-label="Export[^"]*"[^>]*>.*?<\/button>/g) ?? [];
    const checked = buttons.filter((button) => button.includes("lucide-check "));
    expect(checked).toHaveLength(1);
    expect(checked[0]).toContain('aria-label="Export Tax report as CSV"');
  });

  it("keeps the last export's outcome beside the files", () => {
    const html = render({
      exportStatus: {
        year: 2025,
        tone: "warning",
        message: "handoff.xlsx saved with notes.",
        notes: ["Year-end rate fell back to 30 Dec."],
        path: "/tmp/handoff.xlsx",
      },
    });

    expect(html).toContain("handoff.xlsx saved with notes.");
    expect(html).toContain("Year-end rate fell back to 30 Dec.");
    expect(html).toContain("/tmp/handoff.xlsx");
  });
});
