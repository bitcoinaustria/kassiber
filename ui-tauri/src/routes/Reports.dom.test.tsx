// @vitest-environment happy-dom
//
// Which daemon kind each file button calls, and with what: an Austrian book's
// tax report is the year's E 1kv, a generic book's covers every year and the
// daemon rejects a year for it.
import type { ComponentProps } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { CapitalGainsReport } from "@/mocks/reports";

// The UI store persists to localStorage, which this environment lacks.
vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  report: null as CapitalGainsReport | null,
  wallets: [] as Array<{ id: string; label: string }>,
  exports: [] as Array<{ kind: string; args: unknown }>,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to: string }) => (
    <a href={to}>{children}</a>
  ),
  useNavigate: () => () => {},
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string) => {
    if (kind === "ui.reports.capital_gains") {
      return { data: { data: daemon.report }, isLoading: false };
    }
    if (kind === "ui.wallets.list") {
      return { data: { data: { wallets: daemon.wallets } } };
    }
    return {};
  },
  useDaemonMutation: (kind: string) => ({
    mutateAsync: async (args: unknown) => {
      daemon.exports.push({ kind, args });
      return { data: { file: "/exports/reports/file", format: "pdf", pages: 1 } };
    },
  }),
}));

vi.mock("@/daemon/transport", () => ({
  canOpenExportedFiles: () => false,
  canSaveExportedFiles: () => false,
  openExportedFile: vi.fn(),
  saveExportedFileAs: vi.fn(),
}));

vi.mock("@/components/lightning/LightningProfitabilityPanel", () => ({
  LightningProfitabilityPanel: () => null,
}));

import { Reports } from "./Reports";

const baseReport: CapitalGainsReport = {
  jurisdictionCode: "GENERIC",
  fiatCurrency: "EUR",
  year: 2025,
  method: "fifo",
};

async function exportWith(label: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: label }));
  });
  return daemon.exports.at(-1);
}

describe("Reports exports", () => {
  beforeEach(() => {
    daemon.exports = [];
    daemon.wallets = [];
    window.history.replaceState(null, "", "/reports");
  });
  afterEach(cleanup);

  it("writes a generic book's complete report without a year", async () => {
    daemon.report = baseReport;
    render(<Reports />);

    expect(await exportWith("Export Tax report as PDF")).toEqual({
      kind: "ui.reports.export_pdf",
      args: {},
    });
    expect(await exportWith("Export Tax report as CSV")).toEqual({
      kind: "ui.reports.export_csv",
      args: {},
    });
  });

  it("writes an Austrian book's E 1kv for the selected year", async () => {
    daemon.report = { ...baseReport, jurisdictionCode: "AT", method: "moving_average_at" };
    render(<Reports />);

    expect(await exportWith("Export Tax report as PDF")).toEqual({
      kind: "ui.reports.export_austrian_e1kv_pdf",
      args: { year: 2025 },
    });
    expect(await exportWith("Export Tax report as XLSX")).toEqual({
      kind: "ui.reports.export_austrian_e1kv_xlsx",
      args: { year: 2025 },
    });
  });

  it("scopes the year's other files to the selected year", async () => {
    daemon.report = baseReport;
    daemon.wallets = [
      { id: "w1", label: "Vault" },
      { id: "w2", label: "Spending" },
    ];
    render(<Reports />);

    expect(await exportWith("Export Bookkeeping handoff as XLSX")).toEqual({
      kind: "ui.reports.export_year_handoff",
      args: { year: 2025 },
    });
    expect(await exportWith("Export Summary as PDF")).toEqual({
      kind: "ui.reports.export_summary_pdf",
      args: {
        start: "2025-01-01T00:00:00Z",
        end: "2025-12-31T23:59:59Z",
        include_snapshot: true,
        wallets: ["w1", "w2"],
      },
    });
    expect(await exportWith("Export Audit package")).toEqual({
      kind: "ui.reports.export_audit_package",
      args: {
        include_copied_attachments: true,
        include_url_references: true,
        include_journal_state: true,
        include_review_state: true,
        include_edit_history: false,
      },
    });
    // The saved file stays beside the files once the notification is gone.
    expect(screen.getByText("file saved.")).toBeTruthy();
  });
});
