import { describe, expect, it } from "vitest";

import {
  buildSplitPayoutArgs,
  defaultPayoutAsset,
  parseBtc,
  splitPayoutValidation,
} from "./transactionSplitPayout";

describe("parseBtc", () => {
  it("parses positive decimals and rejects junk", () => {
    expect(parseBtc("0.02")).toBe(0.02);
    expect(parseBtc("  0.5 ")).toBe(0.5);
    expect(parseBtc("")).toBeNull();
    expect(parseBtc("abc")).toBeNull();
  });
});

describe("splitPayoutValidation", () => {
  it("requires positive amounts before it is ready", () => {
    const v = splitPayoutValidation({
      outAmount: "",
      payoutAmount: "",
      outboundBtc: 0.1,
    });
    expect(v.ready).toBe(false);
    expect(v.outError).toBeNull(); // empty is not an error yet, just not ready
  });

  it("rejects an out amount above the outbound total", () => {
    const v = splitPayoutValidation({
      outAmount: "0.2",
      payoutAmount: "0.19",
      outboundBtc: 0.1,
    });
    expect(v.outError).toContain("exceed the outbound total");
    expect(v.ready).toBe(false);
  });

  it("rejects non-positive amounts", () => {
    const v = splitPayoutValidation({
      outAmount: "0",
      payoutAmount: "-1",
      outboundBtc: 0.1,
    });
    expect(v.outError).toBe("Enter a positive BTC amount");
    expect(v.payoutError).toBe("Enter a positive BTC amount");
    expect(v.ready).toBe(false);
  });

  it("is ready when the portion is within the outbound and payout is positive", () => {
    const v = splitPayoutValidation({
      outAmount: "0.02",
      payoutAmount: "0.0198",
      outboundBtc: 0.05,
    });
    expect(v.outError).toBeNull();
    expect(v.payoutError).toBeNull();
    expect(v.ready).toBe(true);
  });

  it("allows the full outbound as the payout portion (no remainder)", () => {
    const v = splitPayoutValidation({
      outAmount: "0.05",
      payoutAmount: "0.0498",
      outboundBtc: 0.05,
    });
    expect(v.ready).toBe(true);
  });
});

describe("buildSplitPayoutArgs", () => {
  it("trims amounts and omits blank optional fields", () => {
    expect(
      buildSplitPayoutArgs({
        transactionId: "tx-1",
        outAmount: " 0.02 ",
        payoutAsset: "LBTC",
        payoutAmount: "0.0198",
        policy: "carrying-value",
        counterparty: "  ",
        notes: "",
      }),
    ).toEqual({
      tx_out: "tx-1",
      out_amount: "0.02",
      payout_asset: "LBTC",
      payout_amount: "0.0198",
      policy: "carrying-value",
      counterparty: undefined,
      notes: undefined,
    });
  });

  it("keeps populated optional fields", () => {
    const args = buildSplitPayoutArgs({
      transactionId: "tx-2",
      outAmount: "0.1",
      payoutAsset: "BTC",
      payoutAmount: "0.099",
      policy: "taxable",
      counterparty: "Boltz",
      notes: "submarine swap",
    });
    expect(args.counterparty).toBe("Boltz");
    expect(args.notes).toBe("submarine swap");
    expect(args.policy).toBe("taxable");
  });
});

describe("defaultPayoutAsset", () => {
  it("keeps a known source asset and falls back to BTC otherwise", () => {
    expect(defaultPayoutAsset("LBTC")).toBe("LBTC");
    expect(defaultPayoutAsset("BTC")).toBe("BTC");
    expect(defaultPayoutAsset("FOO")).toBe("BTC");
  });
});
