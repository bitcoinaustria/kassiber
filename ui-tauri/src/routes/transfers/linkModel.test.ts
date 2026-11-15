import { describe, expect, it } from "vitest";

import type { Tx } from "@/mocks/seed";

import {
  byTypedAmount,
  checkFee,
  defaultPairKind,
  differenceOf,
  feeShareBps,
  hasOpenPart,
  legFromTx,
  linkableMsat,
  linkAnswer,
  linkQuestion,
  listCandidates,
  msatFromSats,
  openParts,
  pairWarnings,
  parseAmountInput,
  rankCandidates,
  relinkPortion,
  searchIntent,
  sharesWith,
  widerWindow,
  windowBounds,
  windowBoundsBefore,
  withOpenPart,
  yearBounds,
  type LinkLeg,
} from "./linkModel";
import { formatMsatAsAsset } from "./satsFormat";

function leg(id: string, overrides: Partial<LinkLeg> = {}): LinkLeg {
  return {
    id,
    externalId: null,
    wallet: `wallet-${id}`,
    asset: "BTC",
    rail: "onchain",
    occurredAt: "2024-09-17T02:14:00Z",
    amountMsat: 4_200_000_000n,
    feeMsat: null,
    side: "receipt",
    linked: false,
    ...overrides,
  };
}

function tx(overrides: Partial<Tx>): Tx {
  return { id: "t1", date: "2024-09-17", type: "Withdrawal", account: "Cold", counter: "", amountSat: -4200000, eur: -2400, rate: 57000, tag: "", conf: 1, ...overrides };
}

describe("manual linking", () => {
  it("reads sats from their decimal text, never through a float", () => {
    expect(msatFromSats(-4200000)).toBe(-4_200_000_000n);
    expect(msatFromSats(1234.567)).toBe(1_234_567n);
    expect(msatFromSats(0.001)).toBe(1n);
    expect(msatFromSats("12.5")).toBe(12_500n);
    expect(msatFromSats("1.2345")).toBeNull();
    expect(msatFromSats(null)).toBeNull();
  });

  it("parses a typed amount in bitcoin or sats", () => {
    expect(parseAmountInput("0.042")).toBe(4_200_000_000n);
    expect(parseAmountInput("0,042")).toBe(4_200_000_000n);
    expect(parseAmountInput("₿0.04224910")).toBe(4_224_910_000n);
    expect(parseAmountInput("4 200 000 sats")).toBe(4_200_000_000n);
    expect(parseAmountInput("12.5 sat")).toBe(12_500n);
    expect(parseAmountInput("abc")).toBeNull();
    expect(parseAmountInput("")).toBeNull();
  });

  it("reads the one search field as an amount only when it is written as one", () => {
    expect(searchIntent("0.042")).toEqual({ query: "", amountMsat: 4_200_000_000n });
    expect(searchIntent("0,042")).toEqual({ query: "", amountMsat: 4_200_000_000n });
    expect(searchIntent("4200000 sats")).toEqual({ query: "", amountMsat: 4_200_000_000n });
    expect(searchIntent("₿1")).toEqual({ query: "", amountMsat: 100_000_000_000n });
    // A txid made of digits, a wallet name and a note still search.
    expect(searchIntent("4200000")).toEqual({ query: "4200000", amountMsat: null });
    expect(searchIntent(" Cold storage ")).toEqual({ query: "Cold storage", amountMsat: null });
    expect(searchIntent("v1.2 backup")).toEqual({ query: "v1.2 backup", amountMsat: null });
    expect(searchIntent("")).toEqual({ query: "", amountMsat: null });
  });

  it("reads a transaction as a leg: its side, what it is booked as, and whether it can be picked", () => {
    expect(legFromTx(tx({}))).toMatchObject({ side: "send", wallet: "Cold", amountMsat: 4_200_000_000n, fiat: 2400, linked: false, held: false, excluded: false, bookedAs: "Withdrawal" });
    expect(legFromTx(tx({ amountSat: 5, eur: null, asset: "l-btc", quarantineReason: "missing_price" }))).toMatchObject({ side: "receipt", asset: "LBTC", fiat: null, held: true });
    expect(legFromTx(tx({ excluded: true }))?.excluded).toBe(true);
    expect(legFromTx(tx({ amountSat: Number.NaN }))).toBeNull();
  });

  it("takes a linked list row's own leg out of the move the list shows", () => {
    // The list shows a linked transfer as its whole move: both wallets, the fee as the amount.
    const row = tx({
      account: "Cold -> Spending",
      amountSat: -1200,
      type: "Transfer",
      internal: true,
      pair: { id: "p", type: "transfer", kind: "journal-derived", outWallet: "Cold", inWallet: "Spending", outAsset: "BTC", inAsset: "BTC", outAmountSat: 4200000, inAmountSat: 4198800 },
    });
    expect(legFromTx(row, "send")).toMatchObject({ wallet: "Cold", amountMsat: 4_200_000_000n, linked: true, link: { wallet: "Spending", automatic: true }, fiat: undefined });
    expect(legFromTx(row, "receipt")).toMatchObject({ wallet: "Spending", amountMsat: 4_198_800_000n, link: { wallet: "Cold", automatic: true } });
    // Read on its own, a transaction is already one leg.
    expect(legFromTx({ ...row, account: "Cold", amountSat: -4200000 })).toMatchObject({ wallet: "Cold", amountMsat: 4_200_000_000n, side: "send", link: { wallet: "Spending", automatic: true } });
    expect(legFromTx({ ...row, pair: { ...row.pair!, kind: "manual" } }, "send")?.link?.automatic).toBe(false);
  });

  it("flags a fee past what the matcher suggests on its own, and coins that grew", () => {
    // The peg-out: 144,910 sats on 4,224,910 sats sent, about 3.4%.
    expect(checkFee(4_224_910_000n, 144_910_000n)).toBe("high");
    expect(feeShareBps(4_224_910_000n, 144_910_000n)).toBe(342n);
    expect(checkFee(4_200_000_000n, 2_000_000n)).toBe("plausible");
    // Under 2,500 sats is never flagged, whatever the share.
    expect(checkFee(10_000_000n, 2_000_000n)).toBe("plausible");
    expect(checkFee(4_200_000_000n, 0n)).toBe("none");
    expect(checkFee(4_200_000_000n, -1n)).toBe("negative");
    // 83% missing is no fee: the amounts don't match.
    expect(checkFee(7_370_000_000n, 6_118_083_000n)).toBe("mismatch");
    expect(checkFee(4_200_000_000n, 420_000_001n)).toBe("mismatch");
    expect(checkFee(4_200_000_000n, 420_000_000n)).toBe("high");
  });

  it("names the difference between two sides, and none between different assets", () => {
    const send = leg("send", { side: "send", feeMsat: 1_000_000n });
    expect(differenceOf(send, leg("in", { amountMsat: 4_200_000_000n }))).toEqual({ kind: "plausible", amountMsat: 1_000_000n, shareBps: 2n });
    expect(differenceOf(leg("s", { side: "send" }), leg("in"))).toMatchObject({ kind: "none", amountMsat: 0n });
    expect(differenceOf(send, leg("in", { amountMsat: 5_000_000_000n }))).toMatchObject({ kind: "negative", amountMsat: 799_000_000n });
    // Bitcoin on Liquid compares with bitcoin on chain; a stablecoin does not.
    expect(differenceOf(send, leg("in", { asset: "LBTC", amountMsat: 4_000_000_000n })).kind).toBe("high");
    expect(differenceOf(send, leg("in", { asset: "USDT", amountMsat: 250_000_000_000_000n }))).toEqual({ kind: "otherAsset" });
  });

  it("ranks receipts: the matcher's suggestion, then close amount and time, then the rest, what can't be picked last", () => {
    const send = leg("send", { side: "send" });
    const late = leg("late", { amountMsat: 4_199_000_000n, occurredAt: "2024-09-25T00:00:00Z" });
    const close = leg("close", { amountMsat: 4_199_000_000n, occurredAt: "2024-09-17T03:00:00Z" });
    const closer = leg("closer", { amountMsat: 4_199_500_000n, occurredAt: "2024-09-17T05:00:00Z" });
    const grew = leg("grew", { amountMsat: 5_000_000_000n, occurredAt: "2024-09-17T02:20:00Z" });
    const other = leg("other", { asset: "USDT", amountMsat: 1n, occurredAt: "2024-09-17T02:16:00Z" });
    const linked = leg("linked", { linked: true, occurredAt: "2024-09-17T02:15:00Z" });
    const ignored = leg("ignored", { excluded: true, linked: true, occurredAt: "2024-09-17T02:15:00Z" });
    const suggested = leg("suggested", { amountMsat: 1_000_000_000n, occurredAt: "2024-09-18T00:00:00Z" });
    const ranked = rankCandidates(send, [late, close, closer, grew, other, linked, ignored, suggested, send], new Set(["suggested"]));
    expect(ranked.map((row) => row.leg.id)).toEqual(["suggested", "closer", "close", "late", "other", "grew", "ignored", "linked"]);
    expect(ranked[1]).toMatchObject({ close: true, difference: { kind: "plausible", amountMsat: 500_000n }, minutes: 166, blocked: null });
    expect(ranked.find((row) => row.leg.id === "grew")?.difference?.kind).toBe("negative");
    expect(ranked.at(-1)?.blocked).toBe("linked");
    expect(ranked.at(-2)?.blocked).toBe("ignored");
  });

  it("puts a receipt with the send's own transaction id first, unless it is the same wallet", () => {
    const send = leg("send", { side: "send", externalId: "ABC123", wallet: "Cold" });
    const near = leg("near", { occurredAt: "2024-09-17T02:15:00Z" });
    const same = leg("same", { externalId: "abc123", amountMsat: 1_000_000n, occurredAt: "2024-09-19T02:14:00Z" });
    const change = leg("change", { externalId: "abc123", wallet: "Cold" });
    const ranked = rankCandidates(send, [near, change, same]);
    expect(ranked.map((row) => row.leg.id)).toEqual(["same", "near", "change"]);
    expect(ranked[0].sameTxid).toBe(true);
    expect(ranked[2].notes).toEqual(["sameWallet"]);
  });

  it("ranks sends for a receipt the same way, and marks a send dated after it", () => {
    const receipt = leg("receipt", { amountMsat: 4_080_000_000n, occurredAt: "2024-09-17T03:22:00Z", wallet: "Treasury" });
    const pegOut = leg("peg-out", { side: "send", asset: "LBTC", rail: "liquid", wallet: "Liquid" });
    const tiny = leg("tiny", { side: "send", amountMsat: 1_000n, occurredAt: "2024-09-17T03:00:00Z" });
    const after = leg("after", { side: "send", amountMsat: 4_080_000_000n, occurredAt: "2024-09-18T03:22:00Z" });
    const ranked = rankCandidates(receipt, [tiny, after, pegOut]);
    expect(ranked[0].leg.id).toBe("peg-out");
    expect(ranked[0]).toMatchObject({ difference: { kind: "high", amountMsat: 120_000_000n }, minutes: 68 });
    expect(ranked.find((row) => row.leg.id === "tiny")?.difference?.kind).toBe("negative");
    expect(ranked.find((row) => row.leg.id === "after")).toMatchObject({ notes: ["before"], minutes: -1440 });
  });

  it("lists a first pick as the book has it, what can't be picked last, the typed amount's neighbours first", () => {
    const rows = listCandidates([leg("a", { linked: true }), leg("b", { amountMsat: 1n }), leg("c", { excluded: true }), leg("d")]);
    expect(rows.map((row) => `${row.leg.id}:${row.blocked}`)).toEqual(["b:null", "d:null", "a:linked", "c:ignored"]);
    expect(rows[0].difference).toBeNull();
    expect(byTypedAmount(rows, 4_200_000_000n).map((row) => row.leg.id)).toEqual(["d", "b", "a", "c"]);
    expect(byTypedAmount(rows, null)).toEqual(rows);
  });

  it("says what to look at again before linking, and which of it is serious", () => {
    const send = leg("s", { side: "send", wallet: "Cold" });
    expect(pairWarnings(send, leg("r"))).toEqual([]);
    expect(pairWarnings(send, leg("r", { amountMsat: 5_000_000_000n }))).toEqual([{ key: "negative", serious: true }]);
    expect(pairWarnings(send, leg("r", { amountMsat: 1_000_000_000n }))).toEqual([{ key: "mismatch", serious: true }]);
    // 3% within one network is worth a look; across networks it is what a peg or swap costs.
    expect(pairWarnings(send, leg("r", { amountMsat: 4_070_000_000n }))).toEqual([{ key: "high", serious: false }]);
    expect(pairWarnings(send, leg("r", { amountMsat: 4_070_000_000n, asset: "LBTC", rail: "liquid" }))).toEqual([]);
    expect(pairWarnings(send, leg("r", { wallet: "Cold", occurredAt: "2024-09-16T02:14:00Z" }))).toEqual([
      { key: "before", serious: true },
      { key: "sameWallet", serious: true },
    ]);
    // An hour of clock drift is no warning.
    expect(pairWarnings(send, leg("r", { occurredAt: "2024-09-17T01:20:00Z" }))).toEqual([]);
    expect(pairWarnings({ ...send, linked: true }, leg("r", { excluded: true, linked: true }))).toEqual([
      { key: "sendLinked", serious: true },
      { key: "receiptIgnored", serious: true },
    ]);
  });

  it("asks about a sale only between two assets", () => {
    expect(linkQuestion(leg("a"), leg("b"))).toBe("none");
    expect(linkQuestion(leg("a"), leg("b", { rail: "lightning" }))).toBe("none");
    expect(linkQuestion(leg("a", { asset: "LBTC" }), leg("b"))).toBe("bitcoin");
    expect(linkQuestion(leg("a"), leg("b", { asset: "USDT" }))).toBe("asset");
    expect(linkQuestion(leg("a", { asset: "USDT" }), leg("b", { asset: "USDT" }))).toBe("none");
    expect(linkAnswer("taxable")).toBe("taxable");
    expect(linkAnswer("carrying-value")).toBe("carrying-value");
    expect(linkAnswer("auto")).toBeNull();
    expect(linkAnswer(null)).toBeNull();
  });

  it("records a rail change by its kind and keeps a plain move manual", () => {
    expect(defaultPairKind(leg("a", { asset: "LBTC", rail: "liquid" }), leg("b"))).toBe("peg-out");
    expect(defaultPairKind(leg("a"), leg("b", { asset: "LBTC", rail: "liquid" }))).toBe("peg-in");
    expect(defaultPairKind(leg("a", { rail: "lightning" }), leg("b"))).toBe("reverse-submarine-swap");
    expect(defaultPairKind(leg("a"), leg("b", { rail: "lightning" }))).toBe("submarine-swap");
    expect(defaultPairKind(leg("a"), leg("b"))).toBe("manual");
    expect(defaultPairKind(leg("a"), leg("b", { asset: "USDT", rail: "liquid" }))).toBe("manual");
  });

  it("asks the transaction list for a window around the chosen side, or one year", () => {
    expect(windowBounds("2024-09-17T02:14:00Z", 24)).toEqual({ since: "2024-09-17T01:14:00Z", until: "2024-09-18T02:14:00Z" });
    expect(windowBounds("2024-09-17T02:14:00Z", null)).toEqual({});
    expect(windowBounds(null, 24)).toEqual({});
    expect(windowBoundsBefore("2024-09-17T02:14:00Z", 24)).toEqual({ since: "2024-09-16T02:14:00Z", until: "2024-09-17T03:14:00Z" });
    expect(yearBounds(2024)).toEqual({ since: "2024-01-01T00:00:00Z", until: "2024-12-31T23:59:59Z" });
    expect(yearBounds(null)).toEqual({});
  });

  it("writes another asset's amount with its code, bitcoin as bitcoin", () => {
    expect(formatMsatAsAsset(12_050_000_000_000n, "USDT", false)).toBe("120.5 USDT");
    expect(formatMsatAsAsset(100_000_000_000n, "USDT", false)).toBe("1 USDT");
    expect(formatMsatAsAsset(600_000_000n, "BTC", true)).toBe("₿0.00600000");
  });
});

describe("widening the search", () => {
  it("goes one step wider each time and ends at any time", () => {
    expect(widerWindow(6)).toBe(24);
    expect(widerWindow(72)).toBe(168);
    expect(widerWindow(168)).toBe(720);
    expect(widerWindow(720)).toBeNull();
    // Any time has nothing wider.
    expect(widerWindow(null)).toBeUndefined();
  });
});

describe("a transaction linked in part", () => {
  const pair = (id: string, out: [string, number, number | null], inbound: [string, number, number | null], shares = true) => ({
    id,
    shares_legs: shares,
    out: { transaction_id: out[0], amount_msat: out[1], unlinked_msat: out[2] },
    in: { transaction_id: inbound[0], amount_msat: inbound[1], unlinked_msat: inbound[2] },
  });

  it("adds up what a transaction's links cover and keeps what the core says is left", () => {
    const sat = 1000;
    const parts = openParts([
      pair("p1", ["send", 600_000 * sat, 100_000 * sat], ["r1", 600_000 * sat, 0]),
      pair("p2", ["send", 300_000 * sat, 100_000 * sat], ["r2", 300_000 * sat, 0]),
    ]);
    expect(parts.send.get("send")).toEqual({ linkedMsat: 900_000_000n, unlinkedMsat: 100_000_000n, canAdd: true });
    // Covered in full: nothing to add.
    expect(parts.receipt.get("r1")).toEqual({ linkedMsat: 600_000_000n, unlinkedMsat: 0n, canAdd: false });
    // A send and a receipt with the same id are different sides.
    expect(parts.receipt.get("send")).toBeUndefined();
  });

  it("takes no further link where the core does not allow one or does not say what is left", () => {
    // A swap, or any pair that stays one to one.
    const big = 1_000_000_000;
    expect(openParts([pair("p", ["s", 6 * big, 4 * big], ["r", 5 * big, 0], false)]).send.get("s")?.canAdd).toBe(false);
    expect(openParts([pair("p", ["s", 6 * big, 4 * big], ["r", 6 * big, 0])]).send.get("s")?.canAdd).toBe(true);
    // One of its pairs is enough to close it.
    const mixed = openParts([pair("a", ["s", 6 * big, 4 * big], ["r", 6 * big, 0]), pair("b", ["s", big, 4 * big], ["x", big, 0], false)]);
    expect(mixed.send.get("s")?.canAdd).toBe(false);
    // An older core without the field: nothing is assumed to be left.
    const old = openParts([pair("p", ["s", 600, null], ["r", 600, null])]);
    expect(old.send.get("s")).toEqual({ linkedMsat: 600n, unlinkedMsat: 0n, canAdd: false });
    expect(hasOpenPart(withOpenPart(leg("s", { side: "send" }), old))).toBe(false);
  });

  it("is about the part that is left: pickable, compared by it, and without its network fee again", () => {
    const parts = openParts([pair("p", ["send", 2_500_000_000, 1_700_000_000], ["r1", 2_500_000_000, 0])]);
    const send = withOpenPart(leg("send", { side: "send", amountMsat: 4_200_000_000n, feeMsat: 1_000_000n, linked: true }), parts);
    expect(hasOpenPart(send)).toBe(true);
    expect(linkableMsat(send)).toBe(1_700_000_000n);
    // The rest arrives whole: no difference, though the send as a whole is far larger.
    expect(differenceOf(send, leg("r2", { amountMsat: 1_700_000_000n }))).toEqual({ kind: "none", amountMsat: 0n, shareBps: 0n });
    // Not flagged as linked already, and listed where it can be picked.
    expect(pairWarnings(send, leg("r2", { amountMsat: 1_700_000_000n }))).toEqual([]);
    const receipt = leg("r3", { amountMsat: 1_700_000_000n });
    expect(rankCandidates(receipt, [send])[0].blocked).toBeNull();
    // A transaction its links cover in full stays where it was: linked, not pickable.
    const full = withOpenPart(leg("r1", { amountMsat: 2_500_000_000n }), parts);
    expect(full.linked).toBe(true);
    expect(hasOpenPart(full)).toBe(false);
    expect(linkableMsat(full)).toBe(2_500_000_000n);
    expect(listCandidates([full])[0].blocked).toBe("linked");
    expect(pairWarnings(leg("x", { side: "send" }), full).map((warning) => warning.key)).toContain("receiptLinked");
  });

  it("takes what a fee could be for the difference between the two sides, not for a part to link", () => {
    const sat = 1000;
    // 20,000 sats of 4,200,000: under 1%.
    const fee = openParts([pair("p", ["s", 4_180_000 * sat, 20_000 * sat], ["r", 4_180_000 * sat, 0])]);
    expect(fee.send.get("s")).toEqual({ linkedMsat: 4_180_000_000n, unlinkedMsat: 20_000_000n, canAdd: false });
    expect(hasOpenPart(withOpenPart(leg("s", { side: "send" }), fee))).toBe(false);
    // 2,000 sats of 50,000 is 4%, but under the 2,500 sats any fee may be.
    expect(openParts([pair("p", ["s", 48_000 * sat, 2_000 * sat], ["r", 48_000 * sat, 0])]).send.get("s")?.canAdd).toBe(false);
    // 3% and 126,000 sats: more than a fee, so a part.
    expect(openParts([pair("p", ["s", 4_074_000 * sat, 126_000 * sat], ["r", 4_074_000 * sat, 0])]).send.get("s")?.canAdd).toBe(true);
  });

  it("offers the rest only to what the core takes as a further link", () => {
    const parts = openParts([pair("p", ["send", 2_500_000_000, 1_700_000_000], ["r1", 2_500_000_000, 0])]);
    const send = withOpenPart(leg("send", { side: "send", linked: true }), parts);
    const liquid = leg("liquid", { asset: "LBTC", rail: "liquid", amountMsat: 1_700_000_000n });
    const lightning = leg("ln", { rail: "lightning", amountMsat: 1_700_000_000n });
    const plain = leg("plain", { amountMsat: 1_700_000_000n });
    expect(sharesWith(send, plain)).toBe(true);
    expect(sharesWith(send, liquid)).toBe(false);
    expect(sharesWith(send, lightning)).toBe(false);
    const ranked = rankCandidates(send, [liquid, lightning, plain]);
    expect(ranked.map((row) => [row.leg.id, row.blocked])).toEqual([
      ["plain", null],
      ["liquid", "rest"],
      ["ln", "rest"],
    ]);
    // Chosen anyway, the send reads as linked, which is what the core will say.
    expect(pairWarnings(send, liquid).map((warning) => warning.key)).toContain("sendLinked");
    // A send that is not linked in part is not held back from a swap.
    expect(rankCandidates(leg("free", { side: "send" }), [liquid])[0].blocked).toBeNull();
  });

  it("leaves a transaction no pair names as it was", () => {
    const free = leg("free");
    expect(withOpenPart(free, openParts([]))).toBe(free);
    expect(withOpenPart(free, null)).toBe(free);
  });
});

describe("relinkPortion", () => {
  const send = 20_000_000_000_000_000n;
  it("sends a part of the send exactly, and the whole send as no part", () => {
    expect(relinkPortion("4200000000", send, true)).toEqual({ kind: "part", msat: 4_200_000_000 });
    expect(relinkPortion(4_200_000_000, send, true)).toEqual({ kind: "part", msat: 4_200_000_000 });
    expect(relinkPortion(send.toString(), send, true)).toEqual({ kind: "whole" });
    expect(relinkPortion(null, send, true)).toEqual({ kind: "whole" });
    // Within one asset no part applies.
    expect(relinkPortion("4200000000", send, false)).toEqual({ kind: "whole" });
  });

  it("refuses a part it cannot send exactly instead of dropping it", () => {
    expect(relinkPortion("10000000000000000", send, true)).toEqual({ kind: "unsendable" });
    expect(relinkPortion(10_000_000_000_000_000, send, true)).toEqual({ kind: "unsendable" });
    expect(relinkPortion("1.5", send, true)).toEqual({ kind: "unsendable" });
    expect(relinkPortion("0", send, true)).toEqual({ kind: "unsendable" });
  });
});
