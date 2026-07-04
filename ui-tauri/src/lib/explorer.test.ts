import { describe, expect, it } from "vitest";

import { explorerTargetForTransaction } from "./explorer";

describe("explorerTargetForTransaction", () => {
  it("uses Kassiber's bundled mempool instance by default for Bitcoin", () => {
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "bitcoin",
      }),
    ).toEqual({
      label: "mempool.bitcoin-austria.at",
      url: "https://mempool.bitcoin-austria.at/tx/abc123",
      configured: false,
    });
  });

  it("uses the configured explorer base when provided", () => {
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "bitcoin",
        settings: {
          bitcoinBaseUrl: "https://example.test/api",
          liquidBaseUrl: "",
          publicFallbacks: true,
        },
      }),
    ).toEqual({
      label: "example.test",
      url: "https://example.test/tx/abc123",
      configured: true,
    });
  });

  it("returns no target when public fallbacks are disabled and no explorer is configured", () => {
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "liquid",
        settings: {
          bitcoinBaseUrl: "",
          liquidBaseUrl: "",
          publicFallbacks: false,
        },
      }),
    ).toBeNull();
  });
});
