import { describe, expect, it } from "vitest";

import { CONNECTION_CATEGORIES, CONNECTION_SOURCES } from "./connectionCatalog";

describe("connection catalog", () => {
  it("has stable unique ids and visible artwork for every source", () => {
    const ids = CONNECTION_SOURCES.map((source) => source.id);
    expect(new Set(ids).size).toBe(ids.length);

    for (const source of CONNECTION_SOURCES) {
      expect(source.title.trim()).not.toBe("");
      expect(source.description.trim()).not.toBe("");
      expect(source.image || source.icon).toBeTruthy();
      expect(
        CONNECTION_CATEGORIES.some((category) => category.id === source.category),
      ).toBe(true);
    }
  });

  it("keeps ready integrations on implemented setup paths", () => {
    const implementedSetupKinds = new Set([
      "descriptor",
      "file-wallet",
      "btcpay",
      "bip329",
      "backend-settings",
    ]);

    for (const source of CONNECTION_SOURCES.filter(
      (candidate) => candidate.status === "ready",
    )) {
      expect(implementedSetupKinds.has(source.setupKind ?? "")).toBe(true);
    }
  });

  it("includes the Bitcoin-native connection families Kassiber can already use", () => {
    expect(CONNECTION_SOURCES.map((source) => source.id)).toEqual(
      expect.arrayContaining([
        "bitcoin-core",
        "electrum",
        "esplora",
        "btcpay",
        "phoenix",
        "river",
      ]),
    );
  });

  it("marks transparent dark logos with a light frame for dark mode", () => {
    for (const id of [
      "bitbox",
      "trezor",
      "coldcard",
      "ledger",
      "foundation-passport",
      "coinfinity",
    ]) {
      expect(
        CONNECTION_SOURCES.find((source) => source.id === id)
          ?.imageFrameClassName,
      ).toContain("bg-white");
    }
  });
});
