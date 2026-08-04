import { describe, expect, it } from "vitest";

import {
  connectionAssetIconKind,
  connectionAssetLabel,
  connectionCategoryLabel,
  connectionCategorySortRank,
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
    expect(connectionTypeLabel({ kind: "descriptor" })).toBe("Wallet export");
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

  it("sorts displayed source categories in product order", () => {
    const categories = [
      { kind: "lnd" as const },
      { kind: "address" as const, chain: "bitcoin" },
      { kind: "descriptor" as const, chain: "liquid" },
      { kind: "btcpay" as const },
      { kind: "backend" as const, role: "backend" as const },
    ]
      .sort(
        (a, b) => connectionCategorySortRank(a) - connectionCategorySortRank(b),
      )
      .map((connection) => connectionCategoryLabel(connection));

    expect(categories).toEqual([
      "On-chain",
      "Liquid",
      "Lightning",
      "BTCPay",
      "Infrastructure",
    ]);
  });
});

describe("connectionAssetLabel observed assets", () => {
  it("prefers what was actually imported over configured intent", () => {
    // A file import declares no chain, so a `custom` wallet holding L-BTC rows
    // would otherwise read as BTC and show the wrong icon.
    expect(
      connectionAssetLabel({
        kind: "custom",
        sourceFormat: "generic_ledger",
        observedAssets: ["LBTC"],
      }),
    ).toBe("LBTC");
  });

  it("takes the most common asset in a mixed import", () => {
    // The daemon orders observedAssets by row count, so the badge follows the
    // bulk of the rows rather than whichever asset sorts first.
    expect(
      connectionAssetLabel({
        kind: "custom",
        observedAssets: ["BTC", "LBTC"],
      }),
    ).toBe("BTC");
  });

  it("treats a SATS-denominated import as BTC", () => {
    expect(
      connectionAssetLabel({ kind: "custom", observedAssets: ["SATS"] }),
    ).toBe("BTC");
  });

  it("does not relabel a Lightning connection as plain BTC", () => {
    // Lightning transactions are stored with asset "BTC", so observed assets
    // must never outrank a Lightning kind.
    for (const kind of ["phoenix", "lnd", "core-ln", "nwc"] as const) {
      expect(
        connectionAssetLabel({ kind, observedAssets: ["BTC"] }),
      ).toBe("LN-BTC");
    }
  });

  it("does not relabel a Liquid-configured wallet from its BTC rows", () => {
    expect(
      connectionAssetLabel({
        kind: "descriptor",
        chain: "liquid",
        observedAssets: ["BTC"],
      }),
    ).toBe("LBTC");
  });

  it("falls back to config signals when nothing was imported yet", () => {
    expect(
      connectionAssetLabel({
        kind: "custom",
        chain: "liquid",
        observedAssets: [],
      }),
    ).toBe("LBTC");
  });

  it("ignores assets it does not recognize rather than guessing", () => {
    expect(
      connectionAssetLabel({
        kind: "descriptor",
        chain: "liquid",
        observedAssets: ["SOMETHINGELSE"],
      }),
    ).toBe("LBTC");
  });
});
