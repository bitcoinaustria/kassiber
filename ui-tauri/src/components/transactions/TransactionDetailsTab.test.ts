import { describe, expect, it } from "vitest";

import type { TransactionSwapRoute } from "./TransactionGraphTab";
import { preloadableSwapLegGraphReference } from "./TransactionDetailsTab";

describe("preloadableSwapLegGraphReference", () => {
  it("skips the current transaction leg and returns the paired leg reference", () => {
    const route: TransactionSwapRoute = {
      id: "pair-1",
      currentLeg: "out",
      out: {
        id: "out-row",
        txid: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
      },
      in: {
        id: "in-row",
        txid: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
      },
    };

    expect(preloadableSwapLegGraphReference(route, "out", ["OUT-ROW"])).toBeNull();
    expect(preloadableSwapLegGraphReference(route, "in", ["OUT-ROW"])).toBe("in-row");
  });

  it("falls back to txid/external id and compares current refs case-insensitively", () => {
    const route: TransactionSwapRoute = {
      id: "pair-2",
      currentLeg: "in",
      out: {
        txid: "A152E23BFB6646B3",
      },
      in: {
        externalId: "afec51d0bc49779e",
      },
    };

    expect(preloadableSwapLegGraphReference(route, "out", ["afec51d0bc49779e"])).toBe(
      "A152E23BFB6646B3",
    );
    expect(preloadableSwapLegGraphReference(route, "in", ["AFEC51D0BC49779E"])).toBeNull();
  });
});
