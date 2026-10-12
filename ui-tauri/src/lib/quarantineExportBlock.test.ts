import { describe, expect, it } from "vitest";

import { DaemonRequestError } from "@/daemon/client";

import { quarantineExportBlock } from "./quarantineExportBlock";

function daemonError(code: string, details?: Record<string, unknown>) {
  return new DaemonRequestError("ui.reports.export_year_handoff", {
    kind: "error",
    error: { code, message: "blocked", details },
  } as never);
}

describe("quarantineExportBlock", () => {
  it("reads the blocking transactions of a quarantined year handoff", () => {
    const row = {
      transaction_ref: "ref-1",
      transaction_id: "abcdef",
      wallet: "Hot",
      occurred_at: "2025-03-01T10:00:00Z",
      reason: "missing_spot_price",
    };
    expect(
      quarantineExportBlock(
        daemonError("year_handoff_quarantined", { year: 2025, count: 3, transactions: [row] }),
      ),
    ).toEqual({ count: 3, transactions: [row] });
  });

  it("covers platform exports and ignores every other failure", () => {
    expect(quarantineExportBlock(daemonError("transaction_export_quarantined"))).toEqual({
      count: 0,
      transactions: [],
    });
    expect(quarantineExportBlock(daemonError("report_stale"))).toBeNull();
    expect(quarantineExportBlock(new Error("boom"))).toBeNull();
  });
});
