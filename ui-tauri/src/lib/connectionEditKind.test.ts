import { describe, expect, it } from "vitest";

import { editConfigKindForConnection } from "./connectionEditKind";

describe("editConfigKindForConnection", () => {
  it("keeps generic custom file imports on the file editor path", () => {
    expect(
      editConfigKindForConnection({
        kind: "custom",
        syncMode: "file_import",
        syncSource: "csv",
        sourceFormat: "csv",
      }),
    ).toBe("file-wallet");
  });

  it("routes BTCPay-backed custom wallets to the BTCPay editor path", () => {
    expect(
      editConfigKindForConnection({
        kind: "custom",
        syncMode: "btcpay",
        syncSource: "btcpay",
      }),
    ).toBe("btcpay");
  });
});
