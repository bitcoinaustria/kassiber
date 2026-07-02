import { describe, expect, it } from "vitest";

import {
  connectionAssetIconKind,
  connectionAssetLabel,
  connectionCategoryLabel,
  connectionTypeLabel,
} from "./connectionDisplay";

describe("connection display", () => {
  it("shows the balance asset/layer independently of wallet source kind", () => {
    expect(connectionAssetLabel({ kind: "xpub", chain: "bitcoin" })).toBe("BTC");
    expect(connectionAssetLabel({ kind: "samourai", chain: "bitcoin" })).toBe(
      "BTC",
    );
    expect(connectionAssetLabel({ kind: "descriptor", chain: "liquid" })).toBe(
      "LBTC",
    );
    expect(connectionAssetLabel({ kind: "bullbitcoin", network: "liquidv1" })).toBe(
      "LBTC",
    );
    expect(connectionAssetLabel({ kind: "core-ln" })).toBe("LN-BTC");
    expect(connectionAssetLabel({ kind: "custom", paymentMethodId: "BTC-LN" })).toBe(
      "LN-BTC",
    );
  });

  it("uses the Bitcoin mark for BTC layers and the Liquid mark for LBTC", () => {
    expect(connectionAssetIconKind("BTC")).toBe("bitcoin");
    expect(connectionAssetIconKind("LN-BTC")).toBe("bitcoin");
    expect(connectionAssetIconKind("LBTC")).toBe("liquid");
  });

  it("shows concrete connection types instead of broad categories", () => {
    expect(connectionTypeLabel({ kind: "descriptor" })).toBe("Wallet descriptor");
    expect(connectionTypeLabel({ kind: "address" })).toBe("Address list");
    expect(connectionTypeLabel({ kind: "core-ln" })).toBe("Core Lightning API");
    expect(connectionTypeLabel({ kind: "custom", syncMode: "btcpay" })).toBe(
      "BTCPay API",
    );
    expect(
      connectionTypeLabel({
        kind: "custom",
        syncMode: "btcpay",
        paymentMethodId: "BTC-CHAIN",
      }),
    ).toBe("BTCPay API · BTC-CHAIN");
    expect(
      connectionTypeLabel({
        kind: "bullbitcoin",
        sourceFormat: "bullbitcoin_wallet_csv",
      }),
    ).toBe("Bull Bitcoin Wallet CSV");
    expect(
      connectionTypeLabel({
        kind: "backend",
        role: "backend",
        syncSource: "Electrum / Fulcrum",
      }),
    ).toBe("Electrum / Fulcrum");
    expect(
      connectionCategoryLabel({
        kind: "backend",
        role: "backend",
        chain: "liquid",
      }),
    ).toBe("Infrastructure");
  });
});
