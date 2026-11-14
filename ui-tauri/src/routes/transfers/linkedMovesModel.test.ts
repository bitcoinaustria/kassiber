import { describe, expect, it } from "vitest";

import { AUTOMATIC_UNDO_REASON, type CustodyLineageItem } from "../custodyGapsModel";
import {
  automaticLinkKind,
  fixedLinkKind,
  buildLinkedMoves,
  canUnlinkMove,
  countLinkedMoves,
  filterLinkedMoves,
  heldUndonePairings,
  moveIsTaxable,
  moveType,
  moveYears,
  recentlyUnlinked,
  splitWalletName,
  transfersVerdict,
  withoutHeld,
  type TransferDismissal,
  type TransferPairSummary,
} from "./linkedMovesModel";
import { localeForLanguage } from "@/i18n/config";

import { formatMsatAsBtc, formatMsatAsSats, sumMsat } from "./satsFormat";

const lineageItem = (overrides: Partial<CustodyLineageItem> = {}): CustodyLineageItem => ({
  out_transaction_id: "out-bark",
  in_transaction_id: "in-store",
  occurred_at: "2026-11-13T11:51:50Z",
  asset: "BTC",
  amount_msat: "600000000",
  from_wallet_id: "w-bark",
  from_wallet_label: "BTCPay Bark",
  to_wallet_id: "w-store",
  to_wallet_label: "Store on-chain",
  custody_state: "internal_verified",
  basis_state: "eligible",
  basis_barrier_at: null,
  evidence_reason: "bark_txid_exact",
  network: "regtest",
  rail: "bitcoin",
  ...overrides,
});

const pair = (overrides: Partial<TransferPairSummary> = {}): TransferPairSummary => ({
  id: "pair-1",
  out_transaction_id: "out-cold",
  in_transaction_id: "in-phoenix",
  kind: "manual",
  policy: "carrying-value",
  swap_fee_msat: "2104000",
  out: { transaction_id: "out-cold", wallet: "Cold storage", wallet_kind: "descriptor", asset: "BTC", occurred_at: "2026-10-21T10:00:00Z", amount_msat: "400000000" },
  in: { transaction_id: "in-phoenix", wallet: "Phoenix", wallet_kind: "phoenix", asset: "BTC", occurred_at: "2026-10-21T10:05:00Z", amount_msat: "397896000" },
  ...overrides,
});

const dismissal = (overrides: Partial<TransferDismissal> = {}): TransferDismissal => ({
  id: "dismissal-1",
  out_transaction_id: "out-bark",
  in_transaction_id: "in-store",
  reason: AUTOMATIC_UNDO_REASON,
  created_at: "2026-11-13T12:15:31Z",
  expires_at: "2036-11-10T12:15:31Z",
  active: true,
  permanent: false,
  out: { transaction_id: "out-bark", wallet: "BTCPay Bark", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:51:50Z" },
  in: { transaction_id: "in-store", wallet: "Store on-chain", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:52:02Z" },
  ...overrides,
});

describe("buildLinkedMoves", () => {
  it("lists automatic lineage moves as automatic and undoable", () => {
    const [move] = buildLinkedMoves({ lineage: [lineageItem({ evidence_reason: "exchange_txid_exact" })], pairs: [], dismissals: [] });
    expect(move).toMatchObject({ source: "automatic", undoable: true, fromLabel: "BTCPay Bark", toLabel: "Store on-chain" });
  });

  it("lists a reviewed pair once, with the pair id the lineage lacks", () => {
    const reviewed = lineageItem({
      out_transaction_id: "out-cold",
      in_transaction_id: "in-phoenix",
      custody_state: "internal_reviewed",
      evidence_reason: "manual",
    });
    const moves = buildLinkedMoves({ lineage: [reviewed], pairs: [pair()], dismissals: [] });
    expect(moves).toHaveLength(1);
    expect(moves[0]).toMatchObject({ source: "reviewed", pairId: "pair-1", undoable: false, feeMsat: "2104000", toRail: "lightning" });
  });

  it("carries the sending transaction's fee and prices from the lineage", () => {
    const priced = lineageItem({ fee_msat: "25750000", fiat_value: 3006.68, fee_fiat_value: 13.46, fiat_currency: "EUR" });
    const [move] = buildLinkedMoves({ lineage: [priced], pairs: [], dismissals: [] });
    expect(move).toMatchObject({ feeMsat: "25750000", amountFiat: 3006.68, feeFiat: 13.46 });
    const [unpriced] = buildLinkedMoves({ lineage: [lineageItem()], pairs: [], dismissals: [] });
    expect(unpriced).toMatchObject({ feeMsat: null, amountFiat: null, feeFiat: null });
  });

  it("keeps the network fee on a reviewed pair unless the pair records a swap fee", () => {
    const reviewed = lineageItem({
      out_transaction_id: "out-cold",
      in_transaction_id: "in-phoenix",
      custody_state: "internal_reviewed",
      evidence_reason: "manual",
      fee_msat: "1000000",
      fee_fiat_value: 0.5,
    });
    const [plain] = buildLinkedMoves({ lineage: [reviewed], pairs: [pair({ swap_fee_msat: "0" })], dismissals: [] });
    expect(plain).toMatchObject({ feeMsat: "1000000", feeFiat: 0.5 });
    const [swap] = buildLinkedMoves({ lineage: [reviewed], pairs: [pair({ swap_fee_fiat_value: 1.05 })], dismissals: [] });
    expect(swap).toMatchObject({ feeMsat: "2104000", feeFiat: 1.05 });
  });

  it("prices a pair the lineage does not list from its own legs", () => {
    const priced = pair({
      swap_fee_fiat_value: 1.05,
      out: { ...pair().out, fiat_value: 200 },
    });
    const [move] = buildLinkedMoves({ lineage: [], pairs: [priced], dismissals: [] });
    expect(move).toMatchObject({ amountFiat: 200, feeFiat: 1.05 });
    const [unpriced] = buildLinkedMoves({ lineage: [], pairs: [pair()], dismissals: [] });
    expect(unpriced).toMatchObject({ amountFiat: null, feeFiat: null });
  });

  it("keeps a pair the lineage does not list, such as a taxable swap", () => {
    const moves = buildLinkedMoves({ lineage: [], pairs: [pair({ policy: "taxable" })], dismissals: [] });
    expect(moves).toEqual([expect.objectContaining({ source: "reviewed", pairId: "pair-1" })]);
  });

  it("lists an undone automatic pairing only once the lineage dropped it", () => {
    const stale = buildLinkedMoves({ lineage: [lineageItem()], pairs: [], dismissals: [dismissal()] });
    expect(stale.map((move) => move.source)).toEqual(["automatic"]);
    const rebuilt = buildLinkedMoves({ lineage: [], pairs: [], dismissals: [dismissal()] });
    expect(rebuilt).toEqual([expect.objectContaining({ source: "undone", dismissal: expect.objectContaining({ id: "dismissal-1" }) })]);
  });

  it("ignores ordinary candidate dismissals and expired ones", () => {
    const moves = buildLinkedMoves({
      lineage: [],
      pairs: [],
      dismissals: [dismissal({ reason: "not_a_transfer" }), dismissal({ id: "d2", active: false })],
    });
    expect(moves).toEqual([]);
  });

  it("orders newest first and counts and filters by source", () => {
    const moves = buildLinkedMoves({
      lineage: [lineageItem(), lineageItem({ out_transaction_id: "a", in_transaction_id: "b", occurred_at: "2025-01-01T00:00:00Z", custody_state: "internal_reviewed", evidence_reason: "manual" })],
      pairs: [],
      dismissals: [dismissal({ id: "d3", out_transaction_id: "x", in_transaction_id: "y", out: { ...dismissal().out, occurred_at: "2026-12-01T00:00:00Z" } })],
    });
    expect(moves.map((move) => move.source)).toEqual(["undone", "automatic", "reviewed"]);
    expect(countLinkedMoves(moves)).toEqual({ automatic: 1, reviewed: 1, undone: 1, differentAsset: 0 });
    expect(filterLinkedMoves(moves, "reviewed")).toHaveLength(1);
    // An undone pairing is not linked: it shows under its own filter only.
    expect(filterLinkedMoves(moves, "all").map((move) => move.source)).toEqual(["automatic", "reviewed"]);
    expect(filterLinkedMoves(moves, "undone")).toHaveLength(1);
    expect(moveYears(moves)).toEqual([2026, 2025]);
  });

  it("tags a swap into another asset, but not bitcoin changing rails", () => {
    const swap = pair({
      id: "pair-swap",
      kind: "manual",
      policy: "taxable",
      in: { ...pair().in, asset: "USDT" },
    });
    const peg = pair({ id: "pair-peg", out_transaction_id: "o2", in_transaction_id: "i2", kind: "peg-in", in: { ...pair().in, asset: "LBTC" } });
    const moves = buildLinkedMoves({ lineage: [], pairs: [swap, peg], dismissals: [] });
    const byId = Object.fromEntries(moves.map((move) => [move.pairId, move]));
    expect(byId["pair-swap"].differentAsset).toBe(true);
    expect(byId["pair-peg"].differentAsset).toBe(false);
    expect(filterLinkedMoves(moves, "differentAsset").map((move) => move.pairId)).toEqual(["pair-swap"]);
    expect(countLinkedMoves(moves).differentAsset).toBe(1);
    // Bitcoin changing rails is never another asset, whatever kind it was given.
    const rails = buildLinkedMoves({ lineage: [], pairs: [pair({ id: "pair-rails", kind: "manual", in: { ...pair().in, asset: "LBTC" } })], dismissals: [] });
    expect(rails[0].differentAsset).toBe(false);
  });

  it("marks moves a linking rule made", () => {
    const moves = buildLinkedMoves({ lineage: [], pairs: [pair({ pair_source: "rule_auto" })], dismissals: [] });
    expect(moves[0].byRule).toBe(true);
  });

  it("marks a settled move whose tax basis waits on an earlier boundary", () => {
    const [move] = buildLinkedMoves({ lineage: [lineageItem({ basis_state: "blocked_by_prior_custody_basis" })], pairs: [], dismissals: [] });
    expect(move.basisBlocked).toBe(true);
  });
});

describe("heldUndonePairings", () => {
  it("returns undone pairings whose send or receipt the journals hold", () => {
    expect(heldUndonePairings([dismissal()], new Set(["out-bark"]))).toHaveLength(1);
    expect(heldUndonePairings([dismissal()], new Set(["in-store"]))).toHaveLength(1);
    expect(heldUndonePairings([dismissal()], new Set(["other"]))).toHaveLength(0);
    expect(heldUndonePairings([dismissal({ reason: "not_a_transfer" })], new Set(["out-bark"]))).toHaveLength(0);
  });
});

describe("withoutHeld", () => {
  it("asks about a held undone pairing once instead of also listing it", () => {
    const moves = buildLinkedMoves({ lineage: [lineageItem({ out_transaction_id: "a", in_transaction_id: "b", occurred_at: "2026-01-01T00:00:00Z" })], pairs: [], dismissals: [dismissal()] });
    expect(moves.map((move) => move.source)).toEqual(["undone", "automatic"]);
    expect(withoutHeld(moves, [dismissal()]).map((move) => move.source)).toEqual(["automatic"]);
    expect(withoutHeld(moves, [])).toHaveLength(2);
  });
});

describe("transfersVerdict", () => {
  const base = { loading: false, journalsStale: false, held: 0, suggested: 0, linked: 0 };
  it("leads with stale journals, then what to sort out, then a settled book", () => {
    expect(transfersVerdict({ ...base, loading: true }).state).toBe("loading");
    expect(transfersVerdict({ ...base, journalsStale: true, held: 2 }).state).toBe("stale");
    expect(transfersVerdict({ ...base, held: 1, suggested: 2 })).toEqual({ state: "sortOut", total: 3, held: 1, suggested: 2 });
    expect(transfersVerdict({ ...base, linked: 17 })).toEqual({ state: "settled", linked: 17 });
    expect(transfersVerdict(base).state).toBe("empty");
  });
});

describe("formatMsatAsSats", () => {
  it("formats exact msat amounts without floating point", () => {
    expect(formatMsatAsSats("600000000", "en")).toBe("600,000 sats");
    expect(formatMsatAsSats("600000000", "de")).toBe(`${(600000).toLocaleString(localeForLanguage("de"))} sats`);
    expect(formatMsatAsSats("1500", "en")).toBe("1.5 sats");
    expect(formatMsatAsSats("2100000000000000000", "en")).toBe("2,100,000,000,000,000 sats");
    expect(formatMsatAsSats("-420000", "en")).toBe("−420 sats");
    expect(formatMsatAsSats("not-a-number", "en")).toBe("—");
  });
});

describe("formatMsatAsBtc", () => {
  it("writes exact bitcoin like the Transactions table, without floating point", () => {
    expect(formatMsatAsBtc("600000000")).toBe("₿0.00600000");
    expect(formatMsatAsBtc("420000")).toBe("₿0.00000420");
    expect(formatMsatAsBtc(1500n)).toBe("₿0.000000015");
    expect(formatMsatAsBtc("2100000000000000000")).toBe("₿21000000.00000000");
    expect(formatMsatAsBtc("9007199254740993001")).toBe("₿90071992.54740993001");
    expect(formatMsatAsBtc("nope")).toBe("—");
  });

  it("sums exact amounts and skips unreadable ones", () => {
    expect(sumMsat(["600000000", 420000, null, undefined, "bad"])).toBe(600420000n);
  });
});

describe("recently unlinked", () => {
  const pair = (out: string, inId: string, deleted: string): TransferPairSummary => ({
    id: `pair-${out}`,
    out_transaction_id: out,
    in_transaction_id: inId,
    kind: "peg-out",
    policy: "taxable",
    swap_fee_msat: 144910000,
    out_amount: 4200000000,
    deleted_at: deleted,
    out: { transaction_id: out, wallet: "Liquid", wallet_kind: "descriptor", asset: "LBTC", occurred_at: "2024-09-17T02:14:00Z", amount_msat: 4200000000 },
    in: { transaction_id: inId, wallet: "Treasury", wallet_kind: "address", asset: "BTC", occurred_at: "2024-09-17T03:22:00Z", amount_msat: 4080000000 },
  });
  const dismissal = (out: string, inId: string, created: string): TransferDismissal => ({
    id: `dismissal-${out}`,
    out_transaction_id: out,
    in_transaction_id: inId,
    reason: AUTOMATIC_UNDO_REASON,
    created_at: created,
    expires_at: null,
    active: true,
    permanent: false,
    out: { transaction_id: out, wallet: "Bark", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:51:50Z" },
    in: { transaction_id: inId, wallet: "Store", asset: "BTC", amount_msat: "600000000", occurred_at: "2026-11-13T11:52:02Z" },
  });

  it("lists unlinked pairs with their terms and undone automatic links, newest unlinked first", () => {
    const rows = recentlyUnlinked({
      unlinked: [pair("peg-out", "peg-in", "2026-11-13T14:00:00Z")],
      dismissals: [dismissal("bark-out", "store-in", "2026-11-13T15:00:00Z")],
      held: [],
      linked: [],
    });
    expect(rows.map((row) => row.outTransactionId)).toEqual(["bark-out", "peg-out"]);
    expect(rows[1]).toMatchObject({ fromRail: "liquid", toRail: "onchain", unlinkedAt: "2026-11-13T14:00:00Z" });
    expect(rows[1].pair?.kind).toBe("peg-out");
    expect(rows[0].dismissal?.id).toBe("dismissal-bark-out");
  });

  it("leaves out a held undo (asked under To sort out) and a leg linked again elsewhere", () => {
    const heldUndo = dismissal("bark-out", "store-in", "2026-11-13T15:00:00Z");
    const relinked = buildLinkedMoves({
      lineage: [],
      pairs: [{ ...pair("peg-out", "other-in", "2026-11-13T16:00:00Z"), id: "pair-new", deleted_at: null }],
      dismissals: [],
    });
    expect(
      recentlyUnlinked({
        unlinked: [pair("peg-out", "peg-in", "2026-11-13T14:00:00Z")],
        dismissals: [heldUndo],
        held: [heldUndo],
        linked: relinked,
      }),
    ).toEqual([]);
  });
});

describe("splitWalletName", () => {
  it("keeps the last words whole so a cut name still tells wallets apart", () => {
    expect(splitWalletName("Regtest Liquid Treasury 2024 Rotation")).toEqual(["Regtest Liquid Treasury", " 2024 Rotation"]);
    expect(splitWalletName("Regtest Spending Wallet")).toEqual(["Regtest Spending", " Wallet"]);
  });

  it("leaves short names and an overlong last word alone", () => {
    expect(splitWalletName("Cold storage")).toEqual(["Cold storage", ""]);
    expect(splitWalletName("Treasury Extraordinarilylongname")).toEqual(["Treasury Extraordinarilylongname", ""]);
    expect(splitWalletName("Averyveryverylongsinglewordname")).toEqual(["Averyveryverylongsinglewordname", ""]);
  });
});

describe("moveType", () => {
  it("takes the kind a pair records", () => {
    expect(moveType({ pairKind: "peg-out", fromRail: "onchain", toRail: "onchain" })).toBe("pegOut");
    expect(moveType({ pairKind: "submarine-swap", fromRail: "onchain", toRail: "lightning" })).toBe("submarine");
  });

  it("reads an automatic link from its rails", () => {
    expect(moveType({ pairKind: null, fromRail: "onchain", toRail: "onchain" })).toBe("transfer");
    expect(moveType({ pairKind: null, fromRail: "onchain", toRail: "liquid" })).toBe("pegIn");
    expect(moveType({ pairKind: null, fromRail: "liquid", toRail: "onchain" })).toBe("pegOut");
    expect(moveType({ pairKind: null, fromRail: "lightning", toRail: "onchain" })).toBe("reverseSubmarine");
    // A manual pair records no special kind: a move between wallets.
    expect(moveType({ pairKind: "manual", fromRail: "onchain", toRail: "onchain" })).toBe("transfer");
  });
});

describe("moveIsTaxable and canUnlinkMove", () => {
  it("is taxable only when the pair records it", () => {
    expect(moveIsTaxable({ pairPolicy: "taxable" })).toBe(true);
    expect(moveIsTaxable({ pairPolicy: "carrying-value" })).toBe(false);
    expect(moveIsTaxable({ pairPolicy: null })).toBe(false);
  });

  it("can unlink a pair or an undoable automatic link, not one transaction seen twice", () => {
    expect(canUnlinkMove({ pairId: "pair-1", undoable: false })).toBe(true);
    expect(canUnlinkMove({ pairId: null, undoable: true })).toBe(true);
    expect(canUnlinkMove({ pairId: null, undoable: false })).toBe(false);
  });
});

describe("automaticLinkKind and fixedLinkKind", () => {
  it("names each kind of evidence and falls back to chain evidence", () => {
    expect(automaticLinkKind({ evidenceReason: "row_matched" })).toBe("sameTransaction");
    expect(automaticLinkKind({ evidenceReason: "verified_native_transfer" })).toBe("sameTransaction");
    expect(automaticLinkKind({ evidenceReason: "recorded_fanout" })).toBe("fanout");
    expect(automaticLinkKind({ evidenceReason: "multi_source_consolidation" })).toBe("consolidation");
    expect(automaticLinkKind({ evidenceReason: "channel_lifecycle" })).toBe("channel");
    expect(automaticLinkKind({ evidenceReason: "exchange_txid_exact" })).toBe("exchange");
    expect(automaticLinkKind({ evidenceReason: "something_new" })).toBe("chain");
    expect(automaticLinkKind({ evidenceReason: null })).toBe("chain");
  });

  it("never gives a fixed link an undoable kind", () => {
    expect(fixedLinkKind({ evidenceReason: "bark_txid_exact" })).toBe("chain");
    expect(fixedLinkKind({ evidenceReason: "channel_lifecycle" })).toBe("channel");
  });
});
