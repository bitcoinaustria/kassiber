import { describe, expect, it } from "vitest";
import { formatMinor } from "./model";
import { isTaskPreview } from "./taskModel";

const preview = {
  id: "task", period_id: "2025", state: "active", source_count: 1,
  expected_digest: "a".repeat(64), expected_revision: 3, ready: true,
  blockers: [], step: "prepare", proposals: [{
    source_id: "row", source_kind: "bank", payload: {
      description: "Synthetic", entry_date: "2025-01-01", lines: [
        { account_code: "bank", debit_minor: "100", credit_minor: "0" },
        { account_code: "sales", debit_minor: "0", credit_minor: "100" },
      ],
    },
  }],
};

describe("minimal local accounting review", () => {
  it("rejects a ready preparation without exact proposals or valid authority metadata", () => {
    expect(isTaskPreview(preview)).toBe(true);
    for (const change of [
      { proposals: undefined }, { proposals: [] }, { expected_digest: "bad" },
      { expected_revision: -1 }, { source_count: -1 }, { blockers: undefined },
      { blockers: [{ kind: "not_ready" }] },
    ]) expect(isTaskPreview({ ...preview, ...change })).toBe(false);
  });

  it("requires exact tax forms and export hashes", () => {
    expect(isTaskPreview({ ...preview, step: "tax_finalize", detail: { forms: [] } })).toBe(false);
    expect(isTaskPreview({ ...preview, step: "export_close", detail: { id: "close", snapshot_digest: "bad" } })).toBe(false);
    expect(isTaskPreview({ ...preview, step: "export_close", detail: { id: "close", snapshot_digest: "b".repeat(64) } })).toBe(true);
  });

  it("renders large and negative minor units without floating point loss", () => {
    const eur = { currency: "EUR", minor_unit_exponent: 2 };
    expect(formatMinor("9007199254740997", eur)).toContain("90.071.992.547.409,97");
    expect(formatMinor("-101", eur)).toMatch(/^−.*1,01/);
    expect(formatMinor("100", { currency: "JPY", minor_unit_exponent: 0 })).toContain("100");
  });
});
