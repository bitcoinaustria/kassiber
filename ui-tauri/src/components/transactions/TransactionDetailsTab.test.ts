import { describe, expect, it } from "vitest";

import type { TransactionSwapRoute } from "./TransactionGraphTab";
import {
  preloadableSwapLegGraphReference,
  transactionGraphLookupArgs,
  transactionGraphLookupReferenceArgs,
} from "./TransactionDetailsTab";

describe("transactionGraphLookupArgs", () => {
  it("opts on-chain rows with explorer txids into configured public lookup", () => {
    expect(
      transactionGraphLookupArgs({
        id: "row-1",
        explorerId: "a".repeat(64),
        paymentMethod: "On-chain",
      } as Parameters<typeof transactionGraphLookupArgs>[0]),
    ).toEqual({
      transaction: "row-1",
      allowPublicLookup: true,
    });
  });

  it("does not public-lookup source ids without a verified explorer txid", () => {
    expect(
      transactionGraphLookupArgs({
        id: "row-2",
        txnId: "b".repeat(64),
        paymentMethod: "Exchange",
      } as Parameters<typeof transactionGraphLookupArgs>[0]),
    ).toEqual({
      transaction: "row-2",
      allowPublicLookup: false,
    });
  });

  it("keeps disabled detail queries shaped consistently", () => {
    expect(transactionGraphLookupArgs(null)).toEqual({
      transaction: "",
      allowPublicLookup: false,
    });
  });

  it("keeps preloaded swap leg references local-only", () => {
    expect(transactionGraphLookupReferenceArgs("row-3")).toEqual({
      transaction: "row-3",
      allowPublicLookup: false,
    });
  });
});

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
