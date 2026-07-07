import { describe, expect, it } from "vitest";

import type { TransactionSwapRoute } from "./TransactionGraphModel";
import {
  preloadableSwapLegGraphLookupArgs,
  preloadableSwapLegGraphReference,
  transactionGraphLookupArgs,
  transactionGraphLookupReferenceArgs,
} from "./TransactionGraphLookup";

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

  it("opts chain-backed rows into configured public lookup even when the payment label is generic", () => {
    expect(
      transactionGraphLookupArgs({
        id: "row-mining",
        explorerId: "c".repeat(64),
        paymentMethod: "Exchange",
        chain: "bitcoin",
      } as Parameters<typeof transactionGraphLookupArgs>[0]),
    ).toEqual({
      transaction: "row-mining",
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

  it("keeps ad hoc reference lookups local-only by default", () => {
    expect(transactionGraphLookupReferenceArgs("row-3")).toEqual({
      transaction: "row-3",
      allowPublicLookup: false,
    });
  });

  it("can opt local transaction reference lookups into configured public lookup", () => {
    expect(transactionGraphLookupReferenceArgs("row-3", true)).toEqual({
      transaction: "row-3",
      allowPublicLookup: true,
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

  it("allows configured graph lookup only for paired legs with a local row id", () => {
    const route: TransactionSwapRoute = {
      id: "pair-3",
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

    expect(preloadableSwapLegGraphLookupArgs(route, "in", ["out-row"])).toEqual({
      transaction: "in-row",
      allowPublicLookup: true,
    });
  });

  it("keeps txid-only paired leg lookups local-only", () => {
    const route: TransactionSwapRoute = {
      id: "pair-4",
      currentLeg: "in",
      out: {
        txid: "A152E23BFB6646B3",
      },
      in: {
        externalId: "afec51d0bc49779e",
      },
    };

    expect(preloadableSwapLegGraphLookupArgs(route, "out", ["afec51d0bc49779e"])).toEqual({
      transaction: "A152E23BFB6646B3",
      allowPublicLookup: false,
    });
  });
});
