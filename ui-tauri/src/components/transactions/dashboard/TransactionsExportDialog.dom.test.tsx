// @vitest-environment happy-dom
//
// Mounted: the dialog only exists once the Export button opens it, and its
// preview follows the chosen format, period and wallets.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  mutate: {} as Record<string, ReturnType<typeof vi.fn>>,
  navigate: vi.fn(),
  previewArgs: [] as Record<string, unknown>[],
  preview: { data: undefined as unknown, error: null as unknown },
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

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { DaemonRequestError } from "@/daemon/client";

import { TransactionsExportDialog } from "./TransactionsExportDialog";

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
  range: null,
};

describe("TransactionsExportDialog", () => {
  const exportButton = () =>
    screen
      .getAllByRole("button")
      .find((button) => button.textContent?.includes("dashboard.export.submit")) as HTMLButtonElement;
  const lastPreviewArgs = () => daemon.previewArgs[daemon.previewArgs.length - 1];
  const settle = () => act(() => vi.advanceTimersByTime(400));
  const mount = () => {
    render(<TransactionsExportDialog bookTimezone="Europe/Vienna" />);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.label" }));
  };
  const pickFormat = (name: RegExp) => {
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.format$/ }));
    fireEvent.click(screen.getByRole("radio", { name }));
  };
  const openPeriod = () =>
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.period.label" }));

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
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("previews every transaction as Excel without tabs to choose from", () => {
    mount();
    expect(screen.queryByRole("tab")).toBeNull();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "xlsx" });
    expect(screen.getByRole("cell", { name: "1.00000000" })).toBeTruthy();
    expect(screen.getByText("dashboard.export.period.everything")).toBeTruthy();
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.transactions.export_xlsx"]).toHaveBeenCalledWith({}, expect.any(Object));
  });

  it("narrows the file to a calendar period the core resolves", () => {
    daemon.preview = {
      data: {
        ...TRANSACTIONS_PREVIEW,
        file_name: "kassiber-transactions-2025-12.xlsx",
        range: { timezone: "Europe/Vienna", first_day: "2025-12-01", last_day: "2025-12-31" },
      },
      error: null,
    };
    mount();
    openPeriod();
    fireEvent.click(screen.getByRole("radio", { name: "dashboard.export.period.presets.last_month" }));
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "xlsx", range: "last_month" });
    // The core names the exact days back, in the book's calendar.
    expect(screen.getByText(/dashboard.export.period.span · Europe\/Vienna/)).toBeTruthy();
    expect(screen.getByText("kassiber-transactions-2025-12.xlsx")).toBeTruthy();
    pickFormat(/dashboard.export.formats.blockpit/);
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "transactions", format: "blockpit", range: "last_month" });
    fireEvent.click(exportButton());
    // The file is written for the days the preview showed, not "last month"
    // resolved again: the dialog may have stayed open past midnight.
    expect(daemon.mutate["ui.transactions.export_platform_csv"]).toHaveBeenCalledWith(
      { platform: "blockpit", from: "2025-12-01", to: "2025-12-31", timezone: "Europe/Vienna" },
      expect.any(Object),
    );
  });

  it("waits for the book's calendar before offering a period", () => {
    render(<TransactionsExportDialog bookTimezone={undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.label" }));
    expect(
      (screen.getByRole("button", { name: "dashboard.export.period.label" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("takes a custom range of days", () => {
    mount();
    openPeriod();
    fireEvent.click(screen.getByRole("radio", { name: "dashboard.export.period.presets.custom" }));
    fireEvent.change(screen.getByLabelText("dashboard.export.period.from"), { target: { value: "2025-03-01" } });
    fireEvent.change(screen.getByLabelText("dashboard.export.period.to"), { target: { value: "2025-03-15" } });
    fireEvent.click(screen.getByRole("button", { name: "dashboard.export.period.apply" }));
    settle();
    expect(lastPreviewArgs()).toEqual({
      export: "transactions",
      format: "xlsx",
      from: "2025-03-01",
      to: "2025-03-15",
    });
  });

  it("exports BIP 329 labels as one more format, without a period", () => {
    mount();
    pickFormat(/dashboard.export.formats.labels/);
    settle();
    expect(lastPreviewArgs()).toEqual({ export: "labels" });
    expect(screen.queryByRole("button", { name: "dashboard.export.period.label" })).toBeNull();
    expect(screen.getByText("dashboard.export.period.labelsAllTime")).toBeTruthy();
    fireEvent.click(exportButton());
    expect(daemon.mutate["ui.metadata.bip329.export"]).toHaveBeenCalledWith({ mode: "all" }, expect.any(Object));
  });

  it("exports only the wallets left checked", () => {
    mount();
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
    expect(exportButton().disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: /dashboard.export.blocked.review/ }));
    expect(daemon.navigate).toHaveBeenCalledWith({ to: "/quarantine" });
  });
});
