import { describe, expect, it } from "vitest";

import { exchangeApiKind } from "./exchangeApiModel";

describe("exchangeApiKind", () => {
  it("treats only wallets with an attached backend as API connections", () => {
    expect(exchangeApiKind({ kind: "bitvavo", backend: { name: "bitvavo" } })).toBe("bitvavo");
    expect(exchangeApiKind({ kind: " Kraken ", backend: { name: "kraken-2" } })).toBe("kraken");
    // CSV-imported wallets of an exchange kind keep their file refresh.
    for (const kind of ["bitvavo", "binance", "kraken", "coinbase"]) {
      expect(exchangeApiKind({ kind, backend: { name: "" } })).toBeNull();
      expect(exchangeApiKind({ kind })).toBeNull();
    }
    // A backend beside a file source still makes it an API connection.
    expect(exchangeApiKind({ kind: "kraken", backend: { name: "kraken" } })).toBe("kraken");
    expect(exchangeApiKind({ kind: "descriptor", backend: { name: "esplora" } })).toBeNull();
    expect(exchangeApiKind({ kind: "custom", backend: { name: "kraken" } })).toBeNull();
    expect(exchangeApiKind(undefined)).toBeNull();
  });
});
