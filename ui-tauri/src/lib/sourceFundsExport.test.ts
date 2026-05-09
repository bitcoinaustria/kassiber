import { describe, expect, it } from "vitest";

import { sourceFundsExportArgs } from "@/lib/sourceFundsExport";

describe("source funds export args", () => {
  it("exports only a saved case snapshot", () => {
    const args = sourceFundsExportArgs({ case: { id: "case-123" } });

    expect(args).toEqual({ case: "case-123" });
    expect(args).not.toHaveProperty("target_transaction");
    expect(args).not.toHaveProperty("target_amount");
  });

  it("blocks export args until the preview has a saved case", () => {
    expect(sourceFundsExportArgs(null)).toBeNull();
    expect(sourceFundsExportArgs({})).toBeNull();
    expect(sourceFundsExportArgs({ case: { id: "" } })).toBeNull();
  });
});
