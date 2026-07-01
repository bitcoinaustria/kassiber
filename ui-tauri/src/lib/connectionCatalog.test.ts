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
      "address-list",
      "silent-payment",
      "file-wallet",
      "file-enrichment",
      "btcpay",
      "bullbitcoin-wallet",
      "bip329",
      "backend-settings",
      "samourai",
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
        "address-list",
        "silent-payment",
        "bitcoin-core",
        "electrum",
        "esplora",
        "btcpay",
        "samourai",
        "phoenix",
        "bullbitcoin-wallet",
        "river",
        "bullbitcoin",
        "coinfinity",
        "strike",
      ]),
    );
  });

  it("keeps the Samourai Whirlpool setup watch-only in the catalog", () => {
    const samouraiSources = CONNECTION_SOURCES.filter(
      (source) => source.setupKind === "samourai",
    );

    expect(samouraiSources.map((source) => source.id)).toEqual(["samourai"]);
    expect(samouraiSources[0]?.description).toMatch(/Whirlpool/i);
    expect(JSON.stringify(samouraiSources)).not.toMatch(
      /backup|mnemonic|seed|passphrase|recovery/i,
    );
  });

  it("keeps Silent Payments setup watch-only in the catalog", () => {
    const source = CONNECTION_SOURCES.find(
      (candidate) => candidate.id === "silent-payment",
    );

    expect(source?.setupKind).toBe("silent-payment");
    expect(source?.walletKind).toBe("silent-payment");
    expect(`${source?.description} ${source?.details.join(" ")}`).toMatch(
      /watch-only/i,
    );
    expect(JSON.stringify(source)).not.toMatch(/spspend|private key|seed/i);
  });

  it("uses bundled official artwork for privacy-wallet imports", () => {
    for (const id of ["samourai", "wasabi"]) {
      const source = CONNECTION_SOURCES.find((candidate) => candidate.id === id);

      expect(source?.image).toBeTruthy();
      expect(source?.image).not.toContain("data:image/svg+xml");
    }
  });

  it("uses bundled brand artwork for Core Lightning", () => {
    const coreLightning = CONNECTION_SOURCES.find(
      (source) => source.id === "core-ln",
    );

    expect(coreLightning?.image).toContain("data:image/svg+xml");
    expect(coreLightning?.image).not.toContain("font-family");
    expect(coreLightning?.image).not.toContain("Core%20Lightning");
  });

  it("uses the Bull Bitcoin logo for both wallet and exchange entries", () => {
    const bullWallet = CONNECTION_SOURCES.find(
      (source) => source.id === "bullbitcoin-wallet",
    );
    const bullExchange = CONNECTION_SOURCES.find(
      (source) => source.id === "bullbitcoin",
    );

    expect(bullWallet?.image).toBeTruthy();
    expect(bullWallet?.image).toBe(bullExchange?.image);
    expect(bullWallet?.image).toContain("bullbitcoin-mark");
    expect(bullWallet?.imageClassName).toContain("size-9");
    expect(bullWallet?.imageClassName).not.toContain("rounded");
  });

  it("uses bundled Blockstream Green artwork instead of a generated placeholder", () => {
    const blockstreamGreen = CONNECTION_SOURCES.find(
      (source) => source.id === "blockstream-green",
    );

    expect(blockstreamGreen?.image).toContain("%2300B45A");
    expect(blockstreamGreen?.image).not.toContain("font-family");
    expect(blockstreamGreen?.image).not.toContain("GR");
    expect(blockstreamGreen?.imageClassName).toContain("size-9");
  });

  it("marks transparent dark logos with a theme-aware frame", () => {
    const hardwareWalletIds = [
      "bitbox",
      "trezor",
      "coldcard",
      "ledger",
      "foundation-passport",
    ];

    for (const id of [...hardwareWalletIds, "coinfinity"]) {
      const source = CONNECTION_SOURCES.find((candidate) => candidate.id === id);

      expect(source?.imageFrameClassName).toContain("bg-muted");
    }

    for (const id of hardwareWalletIds) {
      const source = CONNECTION_SOURCES.find((candidate) => candidate.id === id);

      expect(source?.imageClassName).toContain("brightness-0");
      expect(source?.imageClassName).toContain("dark:invert");
    }
  });
});
