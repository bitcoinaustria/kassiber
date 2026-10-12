// @vitest-environment happy-dom
//
// Mounted: the dialog only exists once the Export button opens it, and its
// preview follows the selected export.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  mutate: {} as Record<string, ReturnType<typeof vi.fn>>,
  navigate: vi.fn(),
  previewArgs: [] as Record<string, unknown>[],
  preview: { data: undefined as unknown, error: null as unknown },
  crossCheckArgs: [] as Record<string, unknown>[],
  crossCheck: undefined as unknown,
  picked: null as { path: string; contentsBase64: string } | null,
  wallets: [] as { id: string; label: string; kind?: string; transaction_count?: number }[],
}));

vi.hoisted(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/client", () => {
  class DaemonRequestError extends Error {
    envelope: unknown;
    constructor(kind: string, envelope: { error?: { message?: string } }) {
      super(envelope.error?.message ?? kind);
      this.envelope = envelope;
    }
  }
  return {
    DaemonRequestError,
    useDaemon: (kind: string, args: Record<string, unknown>) => {
      if (kind === "ui.wallets.list") {
        return { data: { data: { wallets: daemon.wallets } }, error: null, isLoading: false };
      }
      if (kind === "ui.reports.cross_check") {
        if (args) daemon.crossCheckArgs.push(args);
        return {
          data: args && daemon.crossCheck ? { data: daemon.crossCheck } : undefined,
          error: null,
          isLoading: false,
          isFetching: false,
        };
      }
      daemon.previewArgs.push(args);
      return {
        data: daemon.preview.data ? { data: daemon.preview.data } : undefined,
        error: daemon.preview.error,
        isLoading: false,
      };
    },
    useDaemonMutation: (kind: string) => {
      daemon.mutate[kind] ??= vi.fn();
      return { mutate: daemon.mutate[kind], isPending: false, error: null };
    },
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => daemon.navigate,
}));

vi.mock("@/lib/exportFile", () => ({
  saveDaemonExport: vi.fn(async ({ exportPath }: { exportPath: string }) => ({
    savedPath: exportPath,
    copied: false,
  })),
  exportBasename: (path: string) => path.split("/").pop() ?? path,
}));

vi.mock("@/lib/filePicker", () => ({
  isFilePickerAvailable: true,
  pickFileWithContentsBase64: vi.fn(async () => daemon.picked),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { DaemonRequestError } from "@/daemon/client";

import { TransactionsExportDialog } from "./TransactionsExportDialog";
import { defaultExportYear } from "./exportYears";

const TRANSACTIONS_PREVIEW = {
  file_name: "kassiber-transactions.xlsx",
  row_count: 142,
  sheets: [
    {
      name: "Transactions",
      columns: ["Occurred At", "Wallet", "Amount"],
      rows: [["2025-01-10T10:00:00Z", "Hot", "1.00000000"]],
      total: 142,
    },
  ],
  warnings: [],
  reconciled: null,
};

const tx = (over: Record<string, unknown>) => ({
  occurred_at: "2025-06-01T10:00:00Z",
  direction: "inbound",
  asset: "BTC",
  amount: "1.00000000",
  fee: "",
  value: "",
  ...over,
});

const CROSS_CHECK = {
  tool: "blockpit",
  file_name: "blockpit.csv",
  book_currency: "EUR",
  year: 2025,
  counts: { different: 1, only_tool: 1, only_kassiber: 0, excluded_in_kassiber: 0, matched: 1, not_comparable: 0 },
  pairs: [
    {
      key: "wallet:Hot",
      wallet: "Hot",
      tool_names: ["Ledger"],
      state: "paired",
      rows: 2,
      issues: 1,
      counts: { different: 1, only_tool: 0, only_kassiber: 0, excluded_in_kassiber: 0, matched: 1, not_comparable: 0 },
    },
    {
      key: "tool:Kraken",
      wallet: null,
      tool_names: ["Kraken"],
      state: "tool_only",
      rows: 1,
      issues: 0,
      counts: { different: 0, only_tool: 1, only_kassiber: 0, excluded_in_kassiber: 0, matched: 0, not_comparable: 0 },
    },
  ],
  insights: [
    { code: "clock_offset", params: { hours: 2 }, message: "offset" },
    { code: "tool_other_assets", params: { count: 3 }, message: "other" },
  ],
  rows: [
    {
      id: "r1",
      status: "different",
      match: "txid",
      pair: "wallet:Hot",
      tool: tx({ row: 1, source: "Ledger", type: "Withdrawal", direction: "outbound", amount: "0.10000000", txid: "pay-1" }),
      kassiber: tx({ ref: "tx-pay", wallet: "Hot", classification: "disposal", direction: "outbound", amount: "0.10000000", txid: "pay-1", excluded: false }),
      differences: [{ field: "type", severity: "tax", tool: "Withdrawal", kassiber: "disposal", message: "type" }],
    },
    {
      id: "r2",
      status: "matched",
      match: "txid",
      pair: "wallet:Hot",
      tool: tx({ row: 2, source: "Ledger", type: "Deposit", txid: "in-1" }),
      kassiber: tx({ ref: "tx-in", wallet: "Hot", classification: "acquisition", txid: "in-1", excluded: false }),
      differences: [],
    },
    {
      id: "r3",
      status: "only_tool",
      match: null,
      pair: "tool:Kraken",
      tool: tx({ row: 3, source: "Kraken", type: "Deposit", amount: "0.20000000", txid: "extra-1" }),
      kassiber: null,
      differences: [],
    },
  ],
};

const HANDOFF_PREVIEW = {
  file_name: "kassiber-year-handoff-2025.xlsx",
  row_count: 9,
  sheets: [
    { name: "Overview", columns: ["Group", "Field", "Value"], rows: [["Scope", "Year", "2025"]], total: 20 },
    { name: "Transactions", columns: ["Wallet"], rows: [["Hot"]], total: 9 },
    { name: "Balances", columns: ["Wallet", "Closing Quantity"], rows: [["Cold", "0.70000000"]], total: 3 },
  ],
  warnings: ["1 wallet/asset rows do not reconcile"],
  reconciled: false,
};

describe("export year options", () => {
  it("defaults to the last completed year when the book covers it", () => {
    const now = new Date(2026, 9, 12);
    expect(defaultExportYear([2026, 2025, 2024], now)).toBe(2025);
    expect(defaultExportYear([2024, 2023], now)).toBe(2024);
    expect(defaultExportYear([], now)).toBeNull();
  });
});

describe("TransactionsExportDialog", () => {
  // The core's accounting years for the book, newest first.
  const YEARS = [2026, 2025, 2024, 2023];
  const exportButton = () =>
    screen
      .getAllByRole("button")
      .find((button) => button.textContent?.includes("dashboard.export.submit")) as HTMLButtonElement;
  const switchTab = (name: string) => {
    const tab = screen.getByRole("tab", { name: new RegExp(name) });
    fireEvent.mouseDown(tab);
    fireEvent.click(tab);
  };
  const lastPreviewArgs = () => daemon.previewArgs[daemon.previewArgs.length - 1];

  const mount = () => {
    render(<TransactionsExportDialog years={YEARS} />);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.label" }));
  };

  const settle = () => act(() => vi.advanceTimersByTime(400));

  beforeEach(() => {
    vi.useFakeTimers();
    daemon.wallets = [
      { id: "w-hot", label: "Hot", kind: "descriptor", transaction_count: 120 },
      { id: "w-cold", label: "Cold", kind: "descriptor", transaction_count: 22 },
    ];
    daemon.mutate = {};
    daemon.navigate.mockReset();
    daemon.previewArgs = [];
    daemon.preview = { data: TRANSACTIONS_PREVIEW, error: null };
    daemon.crossCheckArgs = [];
    daemon.crossCheck = CROSS_CHECK;
    daemon.picked = {
      path: "/home/me/blockpit.csv",
      contentsBase64: btoa("Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount\n"),
    };
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("previews the transaction file and exports every wallet as Excel", () => {
    mount();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "xlsx" });
    expect(screen.getByRole("columnheader", { name: "Wallet" })).toBeTruthy();
    expect(screen.getByRole("cell", { name: "1.00000000" })).toBeTruthy();
    expect(screen.getByText("kassiber-transactions.xlsx")).toBeTruthy();
    expect(exportButton().textContent).toContain("dashboard.export.submitRows");
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.transactions.export_xlsx"]).toHaveBeenCalledWith({}, expect.any(Object));
  });

  it("previews and exports a tool's import format", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.format$/ }));
    fireEvent.click(screen.getByRole("radio", { name: /dashboard.export.formats.blockpit/ }));
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "blockpit" });
    expect(screen.getByText("dashboard.export.importHint.blockpit")).toBeTruthy();
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.transactions.export_platform_csv"]).toHaveBeenCalledWith(
      { platform: "blockpit" },
      expect.any(Object),
    );
  });

  it("shows each handoff sheet and exports the chosen year", () => {
    daemon.preview = { data: HANDOFF_PREVIEW, error: null };
    mount();
    switchTab("dashboard.export.tabs.yearHandoff");
    settle();
    const year = defaultExportYear(YEARS);
    expect(lastPreviewArgs()).toEqual({ export: "year_handoff", year });
    expect(screen.getByText("dashboard.export.status.differences")).toBeTruthy();
    expect(screen.getByText("1 wallet/asset rows do not reconcile")).toBeTruthy();
    // Transactions is the sheet a reader looks at first.
    expect(screen.getByRole("cell", { name: "Hot" })).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: /Balances/ }));
    expect(screen.getByRole("cell", { name: "0.70000000" })).toBeTruthy();
    // The year steps back one at a time; the grid jumps anywhere.
    expect(year).toBe(2025);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.yearPicker.older" }));
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "year_handoff", year: 2024 });
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.reports.export_year_handoff"]).toHaveBeenCalledWith(
      { year: 2024 },
      expect.any(Object),
    );
  });

  it("exports BIP 329 labels with stored and synthesized labels", () => {
    mount();
    switchTab("dashboard.export.tabs.labels");
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "labels" });
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.metadata.bip329.export"]).toHaveBeenCalledWith(
      { mode: "all" },
      expect.any(Object),
    );
  });

  it("exports only the wallets left checked", () => {
    mount();
    expect(screen.getByText(/dashboard.export.walletsCount/)).toBeTruthy();
    fireEvent.click(screen.getByRole("checkbox", { name: "Cold" }));
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "xlsx", wallets: ["w-hot"] });
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.transactions.export_xlsx"]).toHaveBeenCalledWith(
      { wallets: ["w-hot"] },
      expect.any(Object),
    );
  });

  it("asks for a wallet when none is checked", () => {
    mount();
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.selectNone" }));
    settle();
    expect(screen.getByText("dashboard.export.selectWallet")).toBeTruthy();
    expect(exportButton().disabled).toBe(true);
  });

  it("filters a long wallet list and applies None to the matches only", () => {
    daemon.wallets = Array.from({ length: 12 }, (_, index) => ({
      id: `w-${index}`,
      label: index < 3 ? `Ledger ${index}` : `Wallet ${index}`,
      kind: "descriptor",
      transaction_count: 10,
    }));
    mount();
    fireEvent.change(screen.getByRole("textbox", { name: "dashboard.export.filterWallets" }), {
      target: { value: "ledger" },
    });
    expect(screen.getAllByRole("checkbox")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.selectNone" }));
    settle();
    expect(lastPreviewArgs()).toEqual({
      export: "transactions",
      format: "xlsx",
      wallets: Array.from({ length: 9 }, (_, index) => `w-${index + 3}`),
    });
  });

  const openCompare = async () => {
    mount();
    switchTab("dashboard.export.tabs.compare");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /dashboard.export.compare.chooseFile/ }));
    });
  };
  const listedRows = () =>
    screen.queryByRole("list", { name: "dashboard.export.compare.rows" })?.querySelectorAll(":scope > li")
      .length ?? 0;

  it("compares a recognized export wallet by wallet, tool and Kassiber side by side", async () => {
    await openCompare();
    // The header names Blockpit, so nobody is asked which tool it was.
    expect(screen.queryByText("dashboard.export.compare.whichTool")).toBeNull();
    const year = defaultExportYear(YEARS);
    expect(daemon.crossCheckArgs.at(-1)).toEqual({
      tool: "blockpit",
      filename: "blockpit.csv",
      source_bytes_base64: daemon.picked?.contentsBase64,
      year,
    });
    expect(screen.getByRole("status").textContent).toContain("dashboard.export.compare.headline.attention");
    expect(screen.getByText("dashboard.export.compare.notes.clock_offset")).toBeTruthy();
    // Rows that could not be compared are named, never silently dropped.
    expect(screen.getByText("dashboard.export.compare.notes.tool_other_assets")).toBeTruthy();
    // "Needs a look" lists the paired wallet's difference; Kraken, which
    // Kassiber does not have, is one wallet finding instead of a row.
    expect(listedRows()).toBe(1);
    expect(screen.getByText("dashboard.export.compare.diff.type")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: /dashboard.export.compare.views.matched/ }));
    expect(listedRows()).toBe(1);
    expect(screen.getByText("dashboard.export.compare.issue.matched")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^Kraken/ }));
    expect(screen.getByRole("status").textContent).toContain(
      "dashboard.export.compare.headline.walletNotInKassiber",
    );
    fireEvent.click(screen.getByRole("radio", { name: /dashboard.export.compare.views.all/ }));
    expect(listedRows()).toBe(1);
    expect(screen.getByText("dashboard.export.compare.issue.onlyTool")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^Hot/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "dashboard.export.compare.search" }), {
      target: { value: "pay-1" },
    });
    expect(listedRows()).toBe(1);
    fireEvent.click(screen.getAllByRole("button", { name: "dashboard.export.compare.openTransaction" })[0]);
    expect(daemon.navigate).toHaveBeenCalledWith({ to: "/transactions", search: { tx: "tx-pay" } });
  });

  it("does not call an empty comparison agreement", async () => {
    daemon.crossCheck = { ...CROSS_CHECK, rows: [], pairs: [], counts: { ...CROSS_CHECK.counts, different: 0, only_tool: 0, matched: 0 } };
    await openCompare();
    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toContain("dashboard.export.compare.headline.nothingCompared");
    expect(status).not.toContain("headline.agrees");
  });

  it("asks which tool wrote a file it does not recognize, then exports the comparison", async () => {
    daemon.picked = { path: "/home/me/export.csv", contentsBase64: btoa("date,amount\n") };
    await openCompare();
    expect(screen.getByText("dashboard.export.compare.whichTool")).toBeTruthy();
    expect(daemon.crossCheckArgs).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.formats.cointracking/ }));
    expect(daemon.crossCheckArgs.at(-1)).toMatchObject({ tool: "cointracking", filename: "export.csv" });
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.compare.export/ }));
    expect(daemon.mutate["ui.reports.export_cross_check"]).toHaveBeenCalledWith(
      daemon.crossCheckArgs.at(-1),
      expect.any(Object),
    );
  });

  it("shows a quarantine block from the preview before any export", () => {
    daemon.preview = {
      data: undefined,
      error: new DaemonRequestError("ui.exports.preview", {
        kind: "error",
        error: {
          code: "transaction_export_quarantined",
          message: "blocked",
          details: {
            count: 7,
            transactions: [
              {
                transaction_ref: "ref-1",
                transaction_id: "abcdef0123456789abcdef0123456789",
                wallet: "Hot",
                occurred_at: "2025-03-01T10:00:00Z",
                reason: "missing_spot_price",
              },
            ],
          },
        },
      } as never),
    };
    mount();
    expect(screen.getByRole("alert").textContent).toContain("dashboard.export.blocked.title");
    expect(screen.getByText("missing_spot_price")).toBeTruthy();
    expect(screen.getByText("dashboard.export.blocked.more")).toBeTruthy();
    expect(exportButton().disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.blocked.review/ }));
    expect(daemon.navigate).toHaveBeenCalledWith({ to: "/quarantine" });
  });
});
