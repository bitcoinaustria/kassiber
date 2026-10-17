// @vitest-environment happy-dom
//
// The tool cross-check on Reconcile: a picked CSV is compared wallet by
// wallet, with the tool and Kassiber side by side.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const daemon = vi.hoisted(() => ({
  mutate: {} as Record<string, ReturnType<typeof vi.fn>>,
  crossCheckArgs: [] as Record<string, unknown>[],
  crossCheck: undefined as unknown,
  picked: null as { path: string; contentsBase64: string } | null,
}));

vi.hoisted(() => {
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/client", () => ({
  useDaemon: (_kind: string, args: Record<string, unknown> | undefined) => {
    if (args) daemon.crossCheckArgs.push(args);
    return {
      data: args && daemon.crossCheck ? { data: daemon.crossCheck } : undefined,
      error: null,
      isLoading: false,
      isFetching: false,
    };
  },
  useDaemonMutation: (kind: string) => {
    daemon.mutate[kind] ??= vi.fn();
    return { mutate: daemon.mutate[kind], isPending: false, error: null };
  },
}));

vi.mock("@/lib/exportFile", () => ({
  saveDaemonExport: vi.fn(async ({ exportPath }: { exportPath: string }) => ({ savedPath: exportPath, copied: false })),
  exportBasename: (path: string) => path.split("/").pop() ?? path,
}));

vi.mock("@/lib/filePicker", () => ({
  isFilePickerAvailable: true,
  pickFileWithContentsBase64: vi.fn(async () => daemon.picked),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { forgetCrossCheck } from "./crossCheckMemory";
import { ToolCrossCheck } from "./ToolCrossCheck";

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
  year: null,
  years: [2026, 2025, 2024],
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


describe("ToolCrossCheck", () => {
  const navigate = vi.fn();

  beforeEach(() => {
    daemon.mutate = {};
    daemon.crossCheckArgs = [];
    daemon.crossCheck = CROSS_CHECK;
    daemon.picked = {
      path: "/home/me/blockpit.csv",
      contentsBase64: btoa("Date (UTC),Integration Name,Label,Outgoing Asset,Outgoing Amount,Incoming Asset,Incoming Amount\n"),
    };
    navigate.mockReset();
    forgetCrossCheck();
  });

  afterEach(cleanup);

  const openCompare = async () => {
    render(<ToolCrossCheck onOpenTransaction={(ref) => navigate({ to: "/transactions", search: { tx: ref } })} />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /reconcile.tools.chooseFile/ }));
    });
  };
  const listedRows = () =>
    screen.queryByRole("list", { name: "reconcile.tools.rows" })?.querySelectorAll(":scope > li").length ?? 0;
  const daemonNavigate = navigate;

  it("compares a recognized export wallet by wallet, tool and Kassiber side by side", async () => {
    await openCompare();
    // The header names Blockpit, so nobody is asked which tool it was.
    expect(screen.queryByText("reconcile.tools.whichTool")).toBeNull();
    // Every year until the user picks one; the book's years arrive with the result.
    expect(daemon.crossCheckArgs.at(-1)).toEqual({
      tool: "blockpit",
      filename: "blockpit.csv",
      source_bytes_base64: daemon.picked?.contentsBase64,
    });
    expect(screen.getByRole("status").textContent).toContain("reconcile.tools.headline.attention");
    expect(screen.getByText("reconcile.tools.notes.clock_offset")).toBeTruthy();
    // Rows that could not be compared are named, never silently dropped.
    expect(screen.getByText("reconcile.tools.notes.tool_other_assets")).toBeTruthy();
    // "Needs a look" lists the paired wallet's difference; Kraken, which
    // Kassiber does not have, is one wallet finding instead of a row.
    expect(listedRows()).toBe(1);
    expect(screen.getByText("reconcile.tools.diff.type")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: /reconcile.tools.views.matched/ }));
    expect(listedRows()).toBe(1);
    expect(screen.getByText("reconcile.tools.issue.matched")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^Kraken/ }));
    expect(screen.getByRole("status").textContent).toContain(
      "reconcile.tools.headline.walletNotInKassiber",
    );
    fireEvent.click(screen.getByRole("radio", { name: /reconcile.tools.views.all/ }));
    expect(listedRows()).toBe(1);
    expect(screen.getByText("reconcile.tools.issue.onlyTool")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^Hot/ }));
    fireEvent.change(screen.getByRole("textbox", { name: "reconcile.tools.search" }), {
      target: { value: "pay-1" },
    });
    expect(listedRows()).toBe(1);
    fireEvent.click(screen.getAllByRole("button", { name: "reconcile.tools.openTransaction" })[0]);
    expect(daemonNavigate).toHaveBeenCalledWith({ to: "/transactions", search: { tx: "tx-pay" } });
  });

  it("keeps the comparison when the user opens a transaction and comes back", async () => {
    await openCompare();
    fireEvent.click(screen.getByRole("radio", { name: /reconcile.tools.views.matched/ }));
    // Opening a transaction leaves Reconcile, which unmounts the panel.
    cleanup();
    render(<ToolCrossCheck onOpenTransaction={navigate} />);
    expect(screen.getByRole("status").textContent).toContain("reconcile.tools.headline.attention");
    expect(screen.getByText("reconcile.tools.issue.matched")).toBeTruthy();
    expect(daemon.crossCheckArgs.at(-1)).toMatchObject({ tool: "blockpit", filename: "blockpit.csv" });
  });

  it("does not call an empty comparison agreement", async () => {
    daemon.crossCheck = { ...CROSS_CHECK, rows: [], pairs: [], counts: { ...CROSS_CHECK.counts, different: 0, only_tool: 0, matched: 0 } };
    await openCompare();
    const status = screen.getByRole("status").textContent ?? "";
    expect(status).toContain("reconcile.tools.headline.nothingCompared");
    expect(status).not.toContain("headline.agrees");
  });

  it("asks which tool wrote a file it does not recognize, then exports the comparison", async () => {
    daemon.picked = { path: "/home/me/export.csv", contentsBase64: btoa("date,amount\n") };
    await openCompare();
    expect(screen.getByText("reconcile.tools.whichTool")).toBeTruthy();
    expect(daemon.crossCheckArgs).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: /CoinTracking/ }));
    expect(daemon.crossCheckArgs.at(-1)).toMatchObject({ tool: "cointracking", filename: "export.csv" });
    fireEvent.click(screen.getByRole("button", { name: /reconcile.tools.export$/ }));
    expect(daemon.mutate["ui.reports.export_cross_check"]).toHaveBeenCalledWith(
      daemon.crossCheckArgs.at(-1),
      expect.any(Object),
    );
  });

});
