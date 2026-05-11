import { describe, expect, it } from "vitest";

import { describeWalletSyncResult, summarizeSyncResults } from "./syncResults";

describe("syncResults", () => {
  it("keeps the wallet-specific error message in all-sync summaries", () => {
    expect(
      summarizeSyncResults([
        { wallet: "Cold", status: "synced" },
        {
          wallet: "Descriptor",
          status: "error",
          message: "Descriptor-backed refresh requires embit.",
          hint: "Use a desktop build that bundles embit.",
        },
      ]),
    ).toBe(
      "1 refreshed, 1 failed: Descriptor: Descriptor-backed refresh requires embit. Use a desktop build that bundles embit.",
    );
  });

  it("keeps the wallet-specific error message on detail sync", () => {
    expect(
      describeWalletSyncResult(
        {
          wallet: "Descriptor",
          status: "error",
          message: "Failed to reach backend local-esplora: timed out",
        },
        "Descriptor",
      ),
    ).toBe("Descriptor refresh failed: Failed to reach backend local-esplora: timed out");
  });
});
