/**
 * Linking two transactions by hand: which transactions can be a leg, how
 * the candidates for the open side are ranked, what the two amounts differ
 * by, what to look at again before linking and what kind of move linking
 * records by default. None of this decides accounting: the core plans and
 * books the pair, and refuses the ones it cannot.
 *
 * Ranking follows rotki's "Find match" (close amounts first, then time
 * proximity) but never hides a candidate behind a tolerance: Kassiber links
 * exact amounts and the owner decides, so every receipt in the window stays
 * listed and an odd fee is flagged rather than filtered.
 *
 * Amounts are exact millisatoshis as BigInt. The transaction list sends
 * sats as a JSON number that is whole for chain amounts and carries at most
 * three decimals for Lightning; they are read from their decimal text, never
 * multiplied as floats.
 */
import type { Tx } from "@/mocks/seed";
import { isBitcoinAsset } from "@/components/review/transferModel";
import { railForLeg, type SwapRail } from "@/lib/bitcoinRail";

/** How a leg is linked today, when it is. */
export interface LegLink {
  /** The wallet on the other side of that link. */
  wallet: string | null;
  /** Linked by the journals on their own, not by the owner or a rule. */
  automatic: boolean;
}

/**
 * What the pair list says about a transaction that is linked already: how
 * much of it its links cover, how much is left, and whether it can take
 * another link. A send that arrived in two receipts is linked part by part
 * this way.
 *
 * It can take another link only when its links are of the kind the core
 * lets share a transaction (`shares_legs`) and what is left is more than a
 * fee could be: over 1% of the transaction and over 2,500 sats, the limits
 * past which the matcher itself stops reading a difference as a fee. A
 * smaller shortfall on an ordinary link (an exchange that took its fee off
 * the amount) is a difference between the two sides, not a part waiting for
 * a second receipt.
 */
export interface OpenPart {
  linkedMsat: bigint;
  unlinkedMsat: bigint;
  canAdd: boolean;
}

/** One side of a link as the picker and the check show it. */
export interface LinkLeg {
  id: string;
  /** The on-chain or provider id, when the source has one. */
  externalId: string | null;
  wallet: string;
  asset: string;
  rail: SwapRail;
  occurredAt: string | null;
  /** Always positive. */
  amountMsat: bigint;
  /** Network fee the transaction records, when it does. */
  feeMsat: bigint | null;
  /** Which side it can be: money that left is a send, money that came is a receipt. */
  side: LegSide;
  /** Linked already, as a transfer or a swap. */
  linked: boolean;
  /** That link, when the row says what it is. */
  link?: LegLink | null;
  /** What its links leave open, once the pair list is read. */
  open?: OpenPart | null;
  /** The journals hold it for a decision. */
  held?: boolean;
  /** The owner took it out of the book. */
  excluded?: boolean;
  /** What it is booked as today, in the daemon's stable English ("Withdrawal"). */
  bookedAs?: string | null;
  /** Its value in the display currency; null is unpriced, undefined unknown here. */
  fiat?: number | null;
  /** Where it was found, for a leg the matcher offers. */
  network?: string | null;
}

export type LegSide = "send" | "receipt";

/** Sats as the daemon writes them (a JSON number), as exact millisatoshis. */
export function msatFromSats(value: number | string | null | undefined): bigint | null {
  if (value === null || value === undefined) return null;
  const text = typeof value === "number" ? numberText(value) : value.trim();
  const match = /^(-?)(\d+)(?:\.(\d{1,3}))?$/.exec(text);
  if (!match) return null;
  const [, sign, whole, fraction = ""] = match;
  const msat = BigInt(whole) * 1000n + BigInt(fraction.padEnd(3, "0") || "0");
  return sign ? -msat : msat;
}

/** A finite number's plain decimal text, without exponent notation. */
function numberText(value: number): string {
  if (!Number.isFinite(value)) return "";
  const text = String(value);
  if (!/e/i.test(text)) return text;
  // Only tiny or huge values use an exponent; neither is a sat amount with
  // at most three decimals, so a fixed rendering is exact where it matters.
  return value.toFixed(3).replace(/\.?0+$/, "");
}

export function absMsat(value: bigint): bigint {
  return value < 0n ? -value : value;
}

type ExactMsat = string | number | null | undefined;

/**
 * The part of a send a pair linked again swaps, as the core takes it: a
 * whole number of msat. `whole` when the pair swapped all of the send (or
 * no part applies); `unsendable` when the saved part cannot be sent exactly
 * (unreadable, not positive, or past the largest whole number JSON keeps
 * exact). Left out, the core would take the whole send, so `unsendable`
 * must stop the link rather than drop the part.
 */
export type RelinkPortion = { kind: "whole" } | { kind: "part"; msat: number } | { kind: "unsendable" };

export function relinkPortion(saved: ExactMsat, sendMsat: bigint, applies: boolean): RelinkPortion {
  if (!applies || saved === null || saved === undefined || saved === "") return { kind: "whole" };
  let msat: bigint;
  try {
    // A number past the safe range was already rounded when it was read.
    msat = BigInt(typeof saved === "number" ? (Number.isSafeInteger(saved) ? saved : Number.NaN) : saved.trim());
  } catch {
    return { kind: "unsendable" };
  }
  if (msat <= 0n) return { kind: "unsendable" };
  // The whole send is no portion (and naming it would leave the network fee out of the swap fee).
  if (msat >= sendMsat) return { kind: "whole" };
  if (msat > BigInt(Number.MAX_SAFE_INTEGER)) return { kind: "unsendable" };
  return { kind: "part", msat: Number(msat) };
}

function exactMsat(value: ExactMsat): bigint | null {
  if (value === null || value === undefined) return null;
  try {
    return BigInt(typeof value === "number" ? Math.trunc(value) : value);
  } catch {
    return null;
  }
}

interface PairPart {
  transaction_id: string;
  amount_msat: ExactMsat;
  unlinked_msat?: ExactMsat;
}

/** Per side, what each linked transaction has left, from `ui.transfers.list`. */
export function openParts(
  pairs: ReadonlyArray<{ shares_legs?: boolean | null; out: PairPart; in: PairPart }>,
): Record<LegSide, Map<string, OpenPart>> {
  const parts: Record<LegSide, Map<string, OpenPart>> = { send: new Map(), receipt: new Map() };
  // Whether every pair of the transaction lets it take another, before amounts are judged.
  const shared: Record<LegSide, Map<string, boolean>> = { send: new Map(), receipt: new Map() };
  for (const pair of pairs) {
    for (const [side, leg] of [["send", pair.out], ["receipt", pair.in]] as const) {
      const unlinked = exactMsat(leg.unlinked_msat);
      // An older core says nothing about what is left: such a leg stays one to one.
      const before = parts[side].get(leg.transaction_id);
      shared[side].set(leg.transaction_id, (shared[side].get(leg.transaction_id) ?? true) && pair.shares_legs === true && unlinked !== null);
      parts[side].set(leg.transaction_id, {
        linkedMsat: (before?.linkedMsat ?? 0n) + absMsat(exactMsat(leg.amount_msat) ?? 0n),
        unlinkedMsat: unlinked ?? 0n,
        canAdd: false,
      });
    }
  }
  for (const side of ["send", "receipt"] as const) {
    for (const [id, part] of parts[side]) {
      const size = checkFee(part.linkedMsat + part.unlinkedMsat, part.unlinkedMsat);
      part.canAdd = Boolean(shared[side].get(id)) && (size === "high" || size === "mismatch");
    }
  }
  return parts;
}

/** The leg with what its links leave open, when the pair list knows it. */
export function withOpenPart(leg: LinkLeg, parts: Record<LegSide, Map<string, OpenPart>> | null): LinkLeg {
  const open = parts?.[leg.side].get(leg.id) ?? null;
  return open ? { ...leg, linked: true, open } : leg;
}

/** Linked already, with a part left that another link may take. */
export function hasOpenPart(leg: Pick<LinkLeg, "linked" | "open">): boolean {
  return Boolean(leg.linked && leg.open?.canAdd && leg.open.unlinkedMsat > 0n);
}

/**
 * Whether the core would take these two as a further link of a transaction
 * that is linked in part: only an ordinary move within one asset shares a
 * transaction, so a swap, a peg or a Lightning hop does not.
 */
export function sharesWith(send: Pick<LinkLeg, "rail" | "asset">, receipt: Pick<LinkLeg, "rail" | "asset">): boolean {
  return send.asset === receipt.asset && defaultPairKind(send, receipt) === "manual";
}

/** What a new link of this leg is about: the part left of it, or all of it. */
export function linkableMsat(leg: Pick<LinkLeg, "linked" | "open" | "amountMsat">): bigint {
  return hasOpenPart(leg) ? (leg.open as OpenPart).unlinkedMsat : leg.amountMsat;
}

/**
 * A transaction as a leg, or null when its amount is unreadable.
 *
 * The transaction list shows a linked transaction as its whole move (both
 * wallets as the account, the fee as the amount), so a row read for one
 * `side` takes that side's wallet, asset and amount from the link instead.
 * A transaction read on its own (`ui.transactions.resolve`) is already one
 * leg; leave `side` out and its sign decides.
 */
export function legFromTx(tx: Tx, side?: LegSide): LinkLeg | null {
  const own = msatFromSats(tx.amountSat);
  const as: LegSide = side ?? (own !== null && own < 0n ? "send" : "receipt");
  // Only a list row (read for a side) needs its leg taken out of the link.
  const pair = side ? tx.pair : undefined;
  const amount = (pair ? msatFromSats(as === "send" ? pair.outAmountSat : pair.inAmountSat) : null) ?? own;
  if (amount === null) return null;
  const fee = pair ? null : msatFromSats(tx.feeSat ?? null);
  return {
    id: tx.id,
    externalId: tx.externalId?.trim() || null,
    wallet: (pair ? (as === "send" ? pair.outWallet : pair.inWallet) : null) || tx.account,
    // One spelling for Liquid bitcoin, so the two sides compare as the core compares them.
    asset: ((pair ? (as === "send" ? pair.outAsset : pair.inAsset) : null) ?? tx.asset ?? "BTC").toUpperCase().replace(/^L-BTC$/, "LBTC"),
    rail: railForTx(tx),
    occurredAt: tx.occurredAt ?? null,
    amountMsat: absMsat(amount),
    feeMsat: fee === null ? null : absMsat(fee),
    side: as,
    linked: Boolean(tx.pair) || Boolean(tx.internal),
    link: tx.pair
      ? { wallet: (as === "send" ? tx.pair.inWallet : tx.pair.outWallet) ?? null, automatic: tx.pair.kind === "journal-derived" }
      : tx.internal
        ? { wallet: null, automatic: true }
        : null,
    held: Boolean(tx.quarantineReason) || tx.queueStatus === "review",
    excluded: Boolean(tx.excluded),
    bookedAs: tx.type ?? null,
    // A linked list row carries its move's value, not this leg's.
    fiat: pair ? undefined : typeof tx.eur === "number" ? Math.abs(tx.eur) : null,
    network: tx.network ?? null,
  };
}

function railForTx(tx: Tx): SwapRail {
  const method = tx.paymentMethod;
  if (method === "Lightning") return "lightning";
  if (method === "Liquid") return "liquid";
  return railForLeg(tx.asset ?? "BTC", tx.chain ?? null);
}

/**
 * What was paid between the two: sent, plus the send's own network fee,
 * less received. The core books a pair's fee the same way when the send's
 * amount excludes its fee.
 */
export function impliedFeeMsat(
  send: Pick<LinkLeg, "amountMsat" | "feeMsat" | "linked" | "open">,
  receipt: Pick<LinkLeg, "amountMsat" | "linked" | "open">,
): bigint {
  // The part left of a send no longer carries its network fee: the first link booked it.
  const fee = hasOpenPart(send) ? 0n : (send.feeMsat ?? 0n);
  return linkableMsat(send) + fee - linkableMsat(receipt);
}

export type FeeCheck =
  /** Nothing lost on the way, the usual move between wallets of one chain. */
  | "none"
  | "plausible"
  /** More arrived than left: these are probably not the same coins. */
  | "negative"
  /**
   * Over 1% of the send and over 2,500 sats: past what the matcher suggests
   * on its own (`fee_pct_max`, `fee_sats_min`), so worth a second look. High
   * for a transfer, usual for a peg-out or swap.
   */
  | "high"
  /** Over 10% of the send missing: the amounts don't match, whatever it was. */
  | "mismatch";

const HIGH_FEE_SHARE_BPS = 100n; // 1%, the matcher's default fee_pct_max
const HIGH_FEE_FLOOR_MSAT = 2_500_000n; // 2,500 sats, its fee_sats_min
const MISMATCH_SHARE_BPS = 1_000n; // 10%: no fee costs this much

/**
 * Whether a fee looks like a fee. The flag only asks for a second look:
 * a peg-out or a swap can cost several percent, and the owner decides.
 */
export function checkFee(sendMsat: bigint, feeMsat: bigint): FeeCheck {
  if (feeMsat < 0n) return "negative";
  if (feeMsat === 0n) return "none";
  if (feeMsat > HIGH_FEE_FLOOR_MSAT && feeMsat * 10_000n > sendMsat * MISMATCH_SHARE_BPS) return "mismatch";
  if (feeMsat > HIGH_FEE_FLOOR_MSAT && feeMsat * 10_000n > sendMsat * HIGH_FEE_SHARE_BPS) return "high";
  return "plausible";
}

/** The fee as basis points of the send (100 = 1%), rounded down. */
export function feeShareBps(sendMsat: bigint, feeMsat: bigint): bigint | null {
  if (sendMsat <= 0n) return null;
  return (absMsat(feeMsat) * 10_000n) / sendMsat;
}

/** Basis points as a percentage with up to two decimals ("3.43"). */
export function bpsText(bps: bigint): string {
  const whole = bps / 100n;
  const rest = (bps % 100n).toString().padStart(2, "0").replace(/0+$/, "");
  return rest ? `${whole}.${rest}` : `${whole}`;
}

function epoch(value: string | null): number | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

/** Signed minutes from the send to the receipt; null when either lacks a time. */
export function minutesApart(send: Pick<LinkLeg, "occurredAt">, receipt: Pick<LinkLeg, "occurredAt">): number | null {
  const a = epoch(send.occurredAt);
  const b = epoch(receipt.occurredAt);
  if (a === null || b === null) return null;
  return Math.round((b - a) / 60_000);
}

/** Same rail and both bitcoin, or a known rail change: the default kind to record. */
export function defaultPairKind(send: Pick<LinkLeg, "rail" | "asset">, receipt: Pick<LinkLeg, "rail" | "asset">): string {
  if (!isBitcoinAsset(send.asset) || !isBitcoinAsset(receipt.asset)) return "manual";
  if (send.rail === "lightning" && receipt.rail !== "lightning") return "reverse-submarine-swap";
  if (receipt.rail === "lightning" && send.rail !== "lightning") return "submarine-swap";
  const lbtc = (asset: string) => asset.toUpperCase().replace("-", "") === "LBTC";
  if (!lbtc(send.asset) && lbtc(receipt.asset)) return "peg-in";
  if (lbtc(send.asset) && !lbtc(receipt.asset)) return "peg-out";
  return "manual";
}

/**
 * Whether linking leaves the owner something to decide. Within one asset
 * it does not: the core only links such a pair as the owner's own coins
 * moving, with their cost basis. Between two assets the core records either
 * that or a sale and a purchase, so the owner is asked: about bitcoin that
 * changed its form (BTC and L-BTC), or about a swap into another asset.
 */
export type LinkQuestion = "none" | "bitcoin" | "asset";

export function linkQuestion(send: Pick<LinkLeg, "asset">, receipt: Pick<LinkLeg, "asset">): LinkQuestion {
  if (send.asset === receipt.asset) return "none";
  return isBitcoinAsset(send.asset) && isBitcoinAsset(receipt.asset) ? "bitcoin" : "asset";
}

/** The owner's answer, as the core names the two ways to record a link. */
export type LinkAnswer = "carrying-value" | "taxable";

export function linkAnswer(policy: string | null | undefined): LinkAnswer | null {
  return policy === "carrying-value" || policy === "taxable" ? policy : null;
}

/** Whether two legs' amounts can be compared: the same asset, or bitcoin on both sides. */
export function comparable(send: Pick<LinkLeg, "asset">, receipt: Pick<LinkLeg, "asset">): boolean {
  return send.asset === receipt.asset || (isBitcoinAsset(send.asset) && isBitcoinAsset(receipt.asset));
}

/**
 * What lies between a send and a receipt, for showing: how much less (or
 * more) arrived and whether that reads as a fee. Another asset on one side
 * has no difference to name. The core books the pair's fee itself.
 */
export type Difference =
  | { kind: "otherAsset" }
  | { kind: FeeCheck; /** Always positive. */ amountMsat: bigint; shareBps: bigint | null };

export function differenceOf(send: LinkLeg, receipt: LinkLeg): Difference {
  if (!comparable(send, receipt)) return { kind: "otherAsset" };
  const fee = impliedFeeMsat(send, receipt);
  const sent = linkableMsat(send);
  return { kind: checkFee(sent, fee), amountMsat: absMsat(fee), shareBps: feeShareBps(sent, fee) };
}

/** A receipt dated this long before its send is no clock drift. */
const BEFORE_MINUTES = 60;

/** A transaction offered for the open side, with how it compares to the chosen one. */
export interface Candidate {
  leg: LinkLeg;
  /** Null while no other side is chosen to compare with. */
  difference: Difference | null;
  /** Minutes from the send to the receipt; negative when the receipt is first. */
  minutes: number | null;
  /** The matcher suggests this pair on its own. */
  suggested: boolean;
  /** Both wallets saw the same transaction id: the same coins, as far as a chain can say. */
  sameTxid: boolean;
  /** Close in amount and time: listed first, rotki's "Recommended". */
  close: boolean;
  /**
   * Why it can't be picked: unlink it or take it back into the book first.
   * `rest`: one of the two is linked in part, and the rest of it can't go
   * into a swap, a peg or a Lightning hop.
   */
  blocked: "linked" | "ignored" | "rest" | null;
  notes: Array<"sameWallet" | "before">;
}

function blockedBy(leg: LinkLeg): Candidate["blocked"] {
  return leg.excluded ? "ignored" : leg.linked && !hasOpenPart(leg) ? "linked" : null;
}

/**
 * Candidates for the open side, best first: one the matcher suggests or
 * that shares the chosen side's transaction id, then those close in amount
 * (a plausible fee) and time, then the rest by time proximity, then amounts
 * that don't fit (more arrived, far less arrived, another asset), the same
 * wallet or a receipt before its send. Linked and ignored transactions stay
 * listed last and say why they can't be picked.
 */
export function rankCandidates(
  anchor: LinkLeg,
  legs: readonly LinkLeg[],
  suggestedIds: ReadonlySet<string> = new Set(),
): Candidate[] {
  const ranked = legs
    .filter((leg) => leg.id !== anchor.id)
    .map((leg): Candidate => {
      const [send, receipt] = anchor.side === "send" ? [anchor, leg] : [leg, anchor];
      const difference = differenceOf(send, receipt);
      const minutes = minutesApart(send, receipt);
      const notes: Candidate["notes"] = [];
      if (leg.wallet === anchor.wallet) notes.push("sameWallet");
      if (minutes !== null && minutes < -BEFORE_MINUTES) notes.push("before");
      const blocked = blockedBy(leg) ?? ((hasOpenPart(anchor) || hasOpenPart(leg)) && !sharesWith(send, receipt) ? "rest" : null);
      const close =
        !blocked &&
        notes.length === 0 &&
        (difference.kind === "none" || difference.kind === "plausible") &&
        minutes !== null &&
        Math.abs(minutes) <= 24 * 60;
      const sameTxid = Boolean(anchor.externalId && leg.externalId && anchor.externalId.toLowerCase() === leg.externalId.toLowerCase());
      return { leg, difference, minutes, suggested: suggestedIds.has(leg.id), sameTxid, close, blocked, notes };
    });
  const odd = (row: Candidate) =>
    row.notes.length > 0 || row.difference?.kind === "negative" || row.difference?.kind === "mismatch" || row.difference?.kind === "otherAsset";
  const tier = (row: Candidate) =>
    row.blocked ? 4 : row.suggested || (row.sameTxid && !row.notes.includes("sameWallet")) ? 0 : row.close ? 1 : odd(row) ? 3 : 2;
  const gap = (row: Candidate) => (row.difference && "amountMsat" in row.difference ? row.difference.amountMsat : 0n);
  return ranked.sort((a, b) => {
    const byTier = tier(a) - tier(b);
    if (byTier) return byTier;
    if (tier(a) === 1 && gap(a) !== gap(b)) return gap(a) < gap(b) ? -1 : 1;
    const time = Math.abs(a.minutes ?? Number.MAX_SAFE_INTEGER) - Math.abs(b.minutes ?? Number.MAX_SAFE_INTEGER);
    if (time) return time;
    return a.leg.id < b.leg.id ? -1 : a.leg.id > b.leg.id ? 1 : 0;
  });
}

/** Transactions for a first pick, with nothing to compare to: pickable ones first, as listed (newest first). */
export function listCandidates(legs: readonly LinkLeg[]): Candidate[] {
  return legs
    .map((leg, index) => ({ leg, index, blocked: blockedBy(leg) }))
    .sort((a, b) => (Boolean(a.blocked) === Boolean(b.blocked) ? a.index - b.index : a.blocked ? 1 : -1))
    .map(({ leg, blocked }) => ({ leg, difference: null, minutes: null, suggested: false, sameTxid: false, close: false, blocked, notes: [] }));
}

/** Rows reordered by how near their amount is to one the owner typed; what can't be picked stays last. */
export function byTypedAmount(rows: readonly Candidate[], targetMsat: bigint | null): Candidate[] {
  if (targetMsat === null) return [...rows];
  return rows
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      if (Boolean(a.row.blocked) !== Boolean(b.row.blocked)) return a.row.blocked ? 1 : -1;
      const left = absMsat(linkableMsat(a.row.leg) - targetMsat);
      const right = absMsat(linkableMsat(b.row.leg) - targetMsat);
      if (left !== right) return left < right ? -1 : 1;
      return a.index - b.index;
    })
    .map(({ row }) => row);
}

/**
 * What to look at again before linking a chosen pair. `serious` ones turn
 * the confirmation into "Link anyway"; none of them refuses the link, the
 * core decides that.
 */
export type PairWarning =
  | "negative"
  | "mismatch"
  | "high"
  | "before"
  | "sameWallet"
  | "sendLinked"
  | "receiptLinked"
  | "sendIgnored"
  | "receiptIgnored";

export function pairWarnings(send: LinkLeg, receipt: LinkLeg): Array<{ key: PairWarning; serious: boolean }> {
  const warnings: Array<{ key: PairWarning; serious: boolean }> = [];
  const difference = differenceOf(send, receipt);
  // The part left of a send may arrive in several receipts: less than it is no mismatch.
  const part = hasOpenPart(send) && sharesWith(send, receipt);
  if (difference.kind === "negative" || (difference.kind === "mismatch" && !part)) warnings.push({ key: difference.kind, serious: true });
  // A percent or more is usual when bitcoin changes rails; within one rail it is worth a look.
  if (difference.kind === "high" && !part && send.rail === receipt.rail && send.asset === receipt.asset) warnings.push({ key: "high", serious: false });
  const minutes = minutesApart(send, receipt);
  if (minutes !== null && minutes < -BEFORE_MINUTES) warnings.push({ key: "before", serious: true });
  if (send.wallet === receipt.wallet) warnings.push({ key: "sameWallet", serious: true });
  if (send.excluded) warnings.push({ key: "sendIgnored", serious: true });
  else if (send.linked && !(hasOpenPart(send) && sharesWith(send, receipt))) warnings.push({ key: "sendLinked", serious: true });
  if (receipt.excluded) warnings.push({ key: "receiptIgnored", serious: true });
  else if (receipt.linked && !(hasOpenPart(receipt) && sharesWith(send, receipt))) warnings.push({ key: "receiptLinked", serious: true });
  return warnings;
}

/** Windows that end at the receipt: the send came before it. */
export function windowBoundsBefore(occurredAt: string | null, hours: ReceiptWindow): { since?: string; until?: string } {
  const at = epoch(occurredAt);
  if (at === null || hours === null) return {};
  return {
    since: new Date(at - hours * 3_600_000).toISOString().replace(/\.\d{3}Z$/, "Z"),
    until: new Date(at + 3_600_000).toISOString().replace(/\.\d{3}Z$/, "Z"),
  };
}

/** One calendar year (UTC) as list bounds; null is any time. */
export function yearBounds(year: number | null): { since?: string; until?: string } {
  return year === null ? {} : { since: `${year}-01-01T00:00:00Z`, until: `${year}-12-31T23:59:59Z` };
}

/**
 * What the one search field means: text for the daemon's search (wallet,
 * txid, note), or an amount to bring the nearest amounts first. Only a
 * number with a decimal mark, a ₿ or a sats unit reads as an amount, so a
 * txid made of digits still searches.
 */
export function searchIntent(raw: string): { query: string; amountMsat: bigint | null } {
  const text = raw.trim();
  const marked = /^₿/.test(text) || /sats?$/i.test(text) || /^\d[\d\s_'’]*[.,]\d+$/.test(text);
  const amountMsat = marked ? parseAmountInput(text) : null;
  return amountMsat === null ? { query: text, amountMsat: null } : { query: "", amountMsat };
}

/** Rows one picker read asks for, newest first. */
export const PICK_LIMIT = 200;

/** A typed amount in ₿ ("0.042", "0,042") or sats ("4 200 000 sats"), as msat. */
export function parseAmountInput(raw: string): bigint | null {
  const text = raw.trim().toLowerCase().replace(/[\s_'’]/g, "");
  if (!text) return null;
  const sats = /sats?$/.test(text);
  const number = text.replace(/sats?$/, "").replace(/^₿/, "").replace(",", ".");
  const match = /^(\d+)(?:\.(\d+))?$/.exec(number);
  if (!match) return null;
  const [, whole, fraction = ""] = match;
  if (sats) {
    if (fraction.length > 3) return null;
    return BigInt(whole) * 1000n + BigInt(fraction.padEnd(3, "0") || "0");
  }
  if (fraction.length > 11) return null;
  return BigInt(whole) * 100_000_000_000n + BigInt(fraction.padEnd(11, "0") || "0");
}

/** Window choices around the send, in hours; `null` is any time. */
export const RECEIPT_WINDOWS = [6, 24, 72, 168, 720, null] as const;
export type ReceiptWindow = (typeof RECEIPT_WINDOWS)[number];

/**
 * The next wider window, for "Widen search" (after rotki's
 * `matching/search-widening.ts`): one step at a time, ending at any time.
 * Any time has nothing wider, which `undefined` says.
 */
export function widerWindow(hours: ReceiptWindow): ReceiptWindow | undefined {
  const index = RECEIPT_WINDOWS.indexOf(hours);
  return hours === null || index < 0 ? undefined : RECEIPT_WINDOWS[index + 1];
}

/** The ISO bounds the transaction list is asked for. */
export function windowBounds(occurredAt: string | null, hours: ReceiptWindow): { since?: string; until?: string } {
  const at = epoch(occurredAt);
  if (at === null || hours === null) return {};
  const span = hours * 3_600_000;
  // A receipt is never more than an hour before its send, but clocks drift.
  return {
    since: new Date(at - Math.min(span, 3_600_000)).toISOString().replace(/\.\d{3}Z$/, "Z"),
    until: new Date(at + span).toISOString().replace(/\.\d{3}Z$/, "Z"),
  };
}

/** A txid shortened the way the Transactions table shows one. */
export function shortId(value: string | null | undefined, head = 8, tail = 6): string {
  if (!value) return "";
  return value.length > head + tail + 1 ? `${value.slice(0, head)}…${value.slice(-tail)}` : value;
}

/** The date of an ISO time, or a dash. */
export function datePart(value: string | null | undefined): string {
  return value ? value.slice(0, 10) : "—";
}

export function timePart(value: string | null | undefined): string | null {
  const match = value?.match(/[T ](\d{2}:\d{2})/);
  return match ? match[1] : null;
}

/** "2024-09-17 02:14" from an ISO time, or the date alone. */
export function dateTime(value: string | null | undefined): string {
  const time = timePart(value);
  return time ? `${datePart(value)} ${time}` : datePart(value);
}

/** The translation key for a pair kind; an unknown kind reads as a move. */
export function kindKey(kind: string | null | undefined): "manual" | "pegIn" | "pegOut" | "submarine" | "reverseSubmarine" | "chainSwap" | "refund" | "coinjoin" {
  switch (kind) {
    case "peg-in":
      return "pegIn";
    case "peg-out":
      return "pegOut";
    case "submarine-swap":
      return "submarine";
    case "reverse-submarine-swap":
      return "reverseSubmarine";
    case "chain-swap":
      return "chainSwap";
    case "swap-refund":
      return "refund";
    case "coinjoin":
    case "whirlpool":
      return "coinjoin";
    default:
      return "manual";
  }
}
