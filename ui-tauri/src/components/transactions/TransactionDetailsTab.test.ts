import { describe, expect, it } from "vitest";

import type { TransactionSwapRoute } from "./TransactionGraphModel";
import {
  isPublicGraphLookupApproved,
  preloadableSwapLegGraphLookupArgs,
  preloadableSwapLegGraphReference,
  transactionGraphLookupArgs,
  transactionGraphLookupReferenceArgs,
} from "./TransactionGraphLookup";

it("does not carry public lookup approval to another transaction", () => {
  const approved = { id: "row-a" } as Parameters<
    typeof isPublicGraphLookupApproved
  >[1];
  const next = { id: "row-b" } as Parameters<
    typeof isPublicGraphLookupApproved
  >[1];

  expect(isPublicGraphLookupApproved("row-a", approved)).toBe(true);
  expect(isPublicGraphLookupApproved("row-a", next)).toBe(false);
});

describe("transactionGraphLookupArgs", () => {
  it("keeps on-chain rows local until the user asks for a lookup", () => {
    // Having an explorer txid means a lookup is possible, not requested.
    // Opening a detail sheet must not send that txid to a backend.
    const row = {
      id: "row-1",
      explorerId: "a".repeat(64),
      paymentMethod: "On-chain",
    } as Parameters<typeof transactionGraphLookupArgs>[0];

    expect(transactionGraphLookupArgs(row)).toEqual({
      transaction: "row-1",
      allowPublicLookup: false,
    });
    expect(transactionGraphLookupArgs(row, true)).toEqual({
      transaction: "row-1",
      allowPublicLookup: true,
    });
  });

  it("allows a requested lookup for chain-backed rows with a generic payment label", () => {
    expect(
      transactionGraphLookupArgs(
        {
          id: "row-mining",
          explorerId: "c".repeat(64),
          paymentMethod: "Exchange",
          chain: "bitcoin",
        } as Parameters<typeof transactionGraphLookupArgs>[0],
        true,
      ),
    ).toEqual({
      transaction: "row-mining",
      allowPublicLookup: true,
    });
  });

  it("does not public-lookup source ids without a verified explorer txid", () => {
    expect(
      transactionGraphLookupArgs(
        {
          id: "row-2",
          txnId: "b".repeat(64),
          paymentMethod: "Exchange",
        } as Parameters<typeof transactionGraphLookupArgs>[0],
        true,
      ),
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

    // Preloading both legs of a swap would turn one unrequested lookup into
    // three, so the legs follow the sheet's own opt-in.
    expect(preloadableSwapLegGraphLookupArgs(route, "in", ["out-row"])).toEqual({
      transaction: "in-row",
      allowPublicLookup: false,
    });
    expect(
      preloadableSwapLegGraphLookupArgs(route, "in", ["out-row"], true),
    ).toEqual({
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

    expect(
      preloadableSwapLegGraphLookupArgs(route, "out", ["afec51d0bc49779e"], true),
    ).toEqual({
      transaction: "A152E23BFB6646B3",
      allowPublicLookup: false,
    });
  });
});
