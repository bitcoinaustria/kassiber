import { describe, expect, it } from "vitest";

import {
  explorerTargetForAddress,
  explorerTargetForTransaction,
} from "./explorer";

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

  it("does not send a non-mainnet reference to a public mainnet explorer", () => {
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "bitcoin",
        networkName: "signet",
      }),
    ).toBeNull();
    // A configured explorer is trusted for any network.
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "bitcoin",
        networkName: "signet",
        settings: {
          bitcoinBaseUrl: "https://signet.example.test",
          liquidBaseUrl: "",
          publicFallbacks: true,
        },
      })?.url,
    ).toBe("https://signet.example.test/tx/abc123");
  });

  it("still uses the public fallback for the mainnet aliases the daemon emits", () => {
    for (const networkName of ["main", "mainnet", undefined]) {
      expect(
        explorerTargetForTransaction({
          txid: "abc123",
          network: "bitcoin",
          networkName,
        })?.configured,
      ).toBe(false);
    }
    expect(
      explorerTargetForTransaction({
        txid: "abc123",
        network: "liquid",
        networkName: "liquidv1",
      })?.url,
    ).toBe("https://liquid.network/tx/abc123");
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

describe("explorerTargetForAddress", () => {
  it("opens default Bitcoin explorers on the address route", () => {
    expect(
      explorerTargetForAddress({
        address: "bc1qabc",
        network: "bitcoin",
      }),
    ).toEqual({
      label: "mempool.bitcoin-austria.at",
      url: "https://mempool.bitcoin-austria.at/address/bc1qabc",
      configured: false,
    });
  });

  it("supports configured address URL templates", () => {
    expect(
      explorerTargetForAddress({
        address: "ert1qdemo",
        network: "liquid",
        settings: {
          bitcoinBaseUrl: "",
          liquidBaseUrl: "https://liquid.example.test/a/{address}",
          publicFallbacks: true,
        },
      }),
    ).toEqual({
      label: "liquid.example.test",
      url: "https://liquid.example.test/a/ert1qdemo",
      configured: true,
    });
  });
});
