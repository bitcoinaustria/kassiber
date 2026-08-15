import { describe, expect, it } from "vitest";

import {
  editConfigKindForConnection,
  walletSyncNeedsBackend,
} from "./connectionEditKind";

describe("editConfigKindForConnection", () => {
  it("does not require a backend for file-import refreshes", () => {
    expect(walletSyncNeedsBackend("file_import", undefined)).toBe(false);
    expect(walletSyncNeedsBackend("backend_descriptor", undefined)).toBe(true);
  });

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
