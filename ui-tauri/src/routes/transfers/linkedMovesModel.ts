/**
 * One ledger of the moves between the owner's wallets, built from three
 * reads that each knew only part of it:
 *
 * - `ui.custody.lineage.snapshot`: every finalized same-owner move, automatic
 *   (`internal_verified`) or reviewed (`internal_reviewed`);
 * - `ui.transfers.list`: the pairs the owner made, which carry the pair id
 *   needed to unpair and can include moves the lineage does not list (a
 *   taxable cross-asset swap is not a same-owner move);
 * - `ui.transfers.dismissals.list`: automatic pairings the owner undid.
 *
 * Pairs the owner unlinked (`ui.transfers.list` with `include_unlinked`)
 * and automatic pairings they undid are listed apart, as "recently
 * unlinked", each with the way to link it again.
 *
 * Pure so the merge rules are tested on their own: a reviewed pair is listed
 * once even though both reads name it, and an undone pairing is listed as
 * undone only once the lineage no longer counts it as a move.
 */
import { isDifferentAssetMove } from "@/components/review/transferModel";
import { DISMISS_REASON } from "@/components/review/transferModel";
import { railForLeg, type SwapRail } from "@/lib/bitcoinRail";

import {
  AUTOMATIC_UNDO_REASON,
  canUndoAutomaticPairing,
  type CustodyLineageItem,
} from "../custodyGapsModel";

type Exact = string | number;

/** One leg of a pair, as `ui.transfers.list` returns it (the fields used here). */
export interface TransferPairLeg {
  transaction_id: string;
  wallet: string;
  wallet_kind?: string | null;
  asset: string;
  occurred_at: string;
  amount_msat: Exact;
  /** The leg at its own transaction's price; null when unpriced. */
  fiat_value?: number | null;
  /** What no pair covers of this leg's transaction. */
  unlinked_msat?: Exact | null;
  /** The transaction's amount as stored, beyond what this pair covers. */
  full_amount_msat?: Exact | null;
}

export interface TransferPairSummary {
  id: string;
  out_transaction_id: string;
  in_transaction_id: string;
  kind: string;
  policy: string;
  swap_fee_msat: Exact | null;
  /** The swap fee at the sending transaction's price; null when unpriced. */
  swap_fee_fiat_value?: number | null;
  /** How the pair was made (`manual`, `rule_auto`, `bulk_exact`, …). */
  pair_source?: string | null;
  /** The swapped portion of the send, when only part of it was swapped. */
  out_amount?: Exact | null;
  created_at?: string | null;
  /** When the owner unlinked it; set on an unlinked pair only. */
  deleted_at?: string | null;
  notes?: string | null;
  /** Its transactions may take further pairs (a same-asset link of the owner's own coins). */
  shares_legs?: boolean | null;
  out: TransferPairLeg;
  in: TransferPairLeg;
}

export interface TransferPairsEnvelope {
  pairs: TransferPairSummary[];
  /** Present when asked for with `include_unlinked`. */
  unlinked?: TransferPairSummary[];
}

export interface TransferDismissalLeg {
  transaction_id: string;
  wallet: string;
  wallet_id?: string | null;
  asset: string;
  amount_msat: Exact;
  occurred_at: string | null;
}

export interface TransferDismissal {
  id: string;
  out_transaction_id: string;
  in_transaction_id: string;
  reason: string | null;
  created_at: string;
  expires_at: string | null;
  active: boolean;
  permanent: boolean;
  out: TransferDismissalLeg;
  in: TransferDismissalLeg;
}

export interface TransferDismissalsEnvelope {
  dismissals: TransferDismissal[];
  has_more?: boolean;
}

export type LinkedMoveSource = "automatic" | "reviewed" | "undone";

export interface LinkedMove {
  key: string;
  source: LinkedMoveSource;
  occurredAt: string | null;
  outTransactionId: string;
  inTransactionId: string;
  fromLabel: string;
  toLabel: string;
  fromRail: SwapRail;
  toRail: SwapRail;
  asset: string;
  amountMsat: Exact;
  /** What arrived, when the list knows it apart from what left. */
  inAmountMsat: Exact | null;
  inOccurredAt: string | null;
  feeMsat: Exact | null;
  /** Fiat at the sending transaction's price, when the lineage carries it. */
  amountFiat: number | null;
  feeFiat: number | null;
  /** The lineage evidence code, when the lineage lists the move. */
  evidenceReason: string | null;
  lineage: CustodyLineageItem | null;
  pairId: string | null;
  pairKind: string | null;
  dismissal: TransferDismissal | null;
  /** Settled, but its tax basis waits for an earlier custody boundary. */
  basisBlocked: boolean;
  /** An automatic pairing a dismissal can undo. */
  undoable: boolean;
  /**
   * A swap into another asset (taxable unless its policy says otherwise),
   * not bitcoin changing rails: listed with the other moves, chip apart.
   */
  differentAsset: boolean;
  /** Linked by one of the owner's linking rules. */
  byRule: boolean;
  /** When the owner's pair was made; automatic links have none. */
  linkedAt: string | null;
  /** The treatment the owner's pair records (`carrying-value`, `taxable`). */
  pairPolicy: string | null;
  /** How the owner's pair was made (`manual`, `rule_auto`, `bulk_exact`, …). */
  pairSource: string | null;
}

export function moveKey(outTransactionId: string, inTransactionId: string): string {
  return `${outTransactionId}\u0000${inTransactionId}`;
}

/** A dismissal recorded by undoing an automatic pairing, still in force. */
export function isUndoneAutomaticPairing(dismissal: TransferDismissal): boolean {
  return dismissal.active && dismissal.reason === AUTOMATIC_UNDO_REASON;
}

/**
 * The rail each side of a lineage move is on. Each side has its own rail and
 * asset; a core that sends only the joined rail (`bitcoin->lightning`) is
 * read side by side.
 */
function lineageRails(item: CustodyLineageItem): { from: SwapRail; to: SwapRail } {
  const [joinedFrom, joinedTo = joinedFrom] = item.rail.split("->");
  return {
    from: railForLeg(item.source_asset || item.asset, item.source_rail || joinedFrom),
    to: railForLeg(item.target_asset || item.asset, item.target_rail || joinedTo),
  };
}

function fromLineage(item: CustodyLineageItem): LinkedMove {
  const rails = lineageRails(item);
  return {
    key: moveKey(item.out_transaction_id, item.in_transaction_id),
    source: item.custody_state === "internal_reviewed" ? "reviewed" : "automatic",
    occurredAt: item.occurred_at,
    outTransactionId: item.out_transaction_id,
    inTransactionId: item.in_transaction_id,
    fromLabel: item.from_wallet_label,
    toLabel: item.to_wallet_label,
    fromRail: rails.from,
    toRail: rails.to,
    asset: item.asset,
    amountMsat: item.amount_msat,
    amountFiat: item.fiat_value ?? null,
    feeFiat: item.fee_fiat_value ?? null,
    inAmountMsat: null,
    inOccurredAt: null,
    feeMsat: item.fee_msat ?? null,
    evidenceReason: item.evidence_reason,
    lineage: item,
    pairId: null,
    pairKind: null,
    dismissal: null,
    basisBlocked: item.basis_state === "blocked_by_prior_custody_basis",
    undoable: canUndoAutomaticPairing(item),
    differentAsset: false,
    byRule: item.evidence_reason === "rule_auto",
    linkedAt: null,
    pairPolicy: null,
    pairSource: null,
  };
}

function fromPair(pair: TransferPairSummary): LinkedMove {
  return {
    key: moveKey(pair.out_transaction_id, pair.in_transaction_id),
    source: "reviewed",
    occurredAt: pair.out.occurred_at,
    outTransactionId: pair.out_transaction_id,
    inTransactionId: pair.in_transaction_id,
    fromLabel: pair.out.wallet,
    toLabel: pair.in.wallet,
    fromRail: railForLeg(pair.out.asset, pair.out.wallet_kind),
    toRail: railForLeg(pair.in.asset, pair.in.wallet_kind),
    asset: pair.out.asset,
    amountMsat: pair.out.amount_msat,
    amountFiat: pair.out.fiat_value ?? null,
    feeFiat: pair.swap_fee_fiat_value ?? null,
    inAmountMsat: pair.in.amount_msat,
    inOccurredAt: pair.in.occurred_at,
    feeMsat: pair.swap_fee_msat,
    evidenceReason: null,
    lineage: null,
    pairId: pair.id,
    pairKind: pair.kind,
    dismissal: null,
    basisBlocked: false,
    undoable: false,
    differentAsset: pairIsDifferentAsset(pair),
    byRule: pair.pair_source === "rule_auto",
    linkedAt: pair.created_at ?? null,
    pairPolicy: pair.policy,
    pairSource: pair.pair_source ?? null,
  };
}

/** Another asset on the receiving side, and not bitcoin changing rails (peg, Lightning swap). */
export function pairIsDifferentAsset(pair: Pick<TransferPairSummary, "kind" | "out" | "in">): boolean {
  return isDifferentAssetMove(pair.out.asset, pair.in.asset, pair.kind);
}

function fromDismissal(dismissal: TransferDismissal): LinkedMove {
  return {
    key: moveKey(dismissal.out_transaction_id, dismissal.in_transaction_id),
    source: "undone",
    occurredAt: dismissal.out.occurred_at ?? dismissal.in.occurred_at,
    outTransactionId: dismissal.out_transaction_id,
    inTransactionId: dismissal.in_transaction_id,
    fromLabel: dismissal.out.wallet,
    toLabel: dismissal.in.wallet,
    fromRail: railForLeg(dismissal.out.asset, null),
    toRail: railForLeg(dismissal.in.asset, null),
    asset: dismissal.out.asset,
    amountMsat: dismissal.out.amount_msat,
    amountFiat: null,
    feeFiat: null,
    inAmountMsat: dismissal.in.amount_msat,
    inOccurredAt: dismissal.in.occurred_at,
    feeMsat: null,
    evidenceReason: null,
    lineage: null,
    pairId: null,
    pairKind: null,
    dismissal,
    basisBlocked: false,
    undoable: false,
    differentAsset: false,
    byRule: false,
    linkedAt: null,
    pairPolicy: null,
    pairSource: null,
  };
}

function newestFirst(a: LinkedMove, b: LinkedMove): number {
  const left = a.occurredAt ?? "";
  const right = b.occurredAt ?? "";
  if (left !== right) return left < right ? 1 : -1;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

export function buildLinkedMoves({
  lineage,
  pairs,
  dismissals,
}: {
  lineage: readonly CustodyLineageItem[];
  pairs: readonly TransferPairSummary[];
  dismissals: readonly TransferDismissal[];
}): LinkedMove[] {
  const moves = new Map<string, LinkedMove>();
  for (const item of lineage) {
    const move = fromLineage(item);
    if (!moves.has(move.key)) moves.set(move.key, move);
  }
  for (const pair of pairs) {
    const key = moveKey(pair.out_transaction_id, pair.in_transaction_id);
    const listed = moves.get(key);
    if (listed) {
      // The lineage names the move; the pair adds what unpairing it needs.
      moves.set(key, {
        ...listed,
        source: "reviewed",
        pairId: pair.id,
        pairKind: pair.kind,
        // The sending transaction's network fee stays unless the pair records a swap fee.
        ...(pair.swap_fee_msat !== null && String(pair.swap_fee_msat) !== "0"
          ? { feeMsat: pair.swap_fee_msat, feeFiat: pair.swap_fee_fiat_value ?? null }
          : {}),
        inAmountMsat: pair.in.amount_msat,
        inOccurredAt: pair.in.occurred_at,
        linkedAt: pair.created_at ?? null,
        pairPolicy: pair.policy,
        pairSource: pair.pair_source ?? null,
        fromRail: railForLeg(pair.out.asset, pair.out.wallet_kind),
        toRail: railForLeg(pair.in.asset, pair.in.wallet_kind),
        undoable: false,
        differentAsset: pairIsDifferentAsset(pair),
        byRule: listed.byRule || pair.pair_source === "rule_auto",
      });
    } else {
      moves.set(key, fromPair(pair));
    }
  }
  for (const dismissal of dismissals) {
    if (!isUndoneAutomaticPairing(dismissal)) continue;
    const key = moveKey(dismissal.out_transaction_id, dismissal.in_transaction_id);
    // Until the journals are rebuilt the lineage may still count the move.
    if (moves.has(key)) continue;
    moves.set(key, fromDismissal(dismissal));
  }
  return [...moves.values()].sort(newestFirst);
}

/**
 * What the list shows. "All" is every move that is linked; a pairing the
 * owner undid is not linked, so it shows only under "Unlinked".
 */
export type LinkedMoveFilter = "all" | LinkedMoveSource | "differentAsset";

export function filterLinkedMoves(moves: readonly LinkedMove[], filter: LinkedMoveFilter): LinkedMove[] {
  switch (filter) {
    case "all":
      return moves.filter((move) => move.source !== "undone");
    case "differentAsset":
      return moves.filter((move) => move.source !== "undone" && move.differentAsset);
    default:
      return moves.filter((move) => move.source === filter);
  }
}

export function countLinkedMoves(
  moves: readonly LinkedMove[],
): Record<LinkedMoveSource | "differentAsset", number> {
  const counts = { automatic: 0, reviewed: 0, undone: 0, differentAsset: 0 };
  for (const move of moves) {
    counts[move.source] += 1;
    if (move.differentAsset && move.source !== "undone") counts.differentAsset += 1;
  }
  return counts;
}

/** The calendar year a move happened in, from its ISO timestamp. */
export function moveYear(move: Pick<LinkedMove, "occurredAt">): number | null {
  const year = Number.parseInt(move.occurredAt?.slice(0, 4) ?? "", 10);
  return Number.isFinite(year) ? year : null;
}

/** Years that hold a move, newest first. */
export function moveYears(moves: readonly LinkedMove[]): number[] {
  const years = new Set<number>();
  for (const move of moves) {
    const year = moveYear(move);
    if (year !== null) years.add(year);
  }
  return [...years].sort((a, b) => b - a);
}

/**
 * Moves the list shows: an undone pairing that the journals hold is asked
 * about once, above the list, and is not listed again.
 */
export function withoutHeld(moves: readonly LinkedMove[], held: readonly TransferDismissal[]): LinkedMove[] {
  if (!held.length) return [...moves];
  const keys = new Set(held.map((dismissal) => moveKey(dismissal.out_transaction_id, dismissal.in_transaction_id)));
  return moves.filter((move) => !(move.source === "undone" && keys.has(move.key)));
}

/**
 * Undone automatic pairings whose send or receipt the journals hold: these
 * wait on the owner and block reports. An undone pairing whose rows were
 * since resolved another way is only listed under Undone.
 */
export function heldUndonePairings(
  dismissals: readonly TransferDismissal[],
  heldTransactionIds: ReadonlySet<string>,
): TransferDismissal[] {
  return dismissals.filter(
    (dismissal) =>
      isUndoneAutomaticPairing(dismissal) &&
      (heldTransactionIds.has(dismissal.out_transaction_id) || heldTransactionIds.has(dismissal.in_transaction_id)),
  );
}

export type TransfersVerdict =
  | { state: "loading" }
  | { state: "stale" }
  | { state: "unavailable" }
  | { state: "sortOut"; total: number; held: number; suggested: number }
  | { state: "settled"; linked: number }
  | { state: "empty" };

/**
 * What the screen leads with. Stale journals come first because every count
 * below them can be wrong; then a read that failed, since what it would have
 * said is unknown rather than nothing; then what waits on the owner (held
 * moves and suggestions to confirm); then a settled book, which says how
 * many moves it links.
 */
export function transfersVerdict({
  loading,
  unavailable = false,
  journalsStale,
  held,
  suggested,
  linked,
}: {
  loading: boolean;
  /** Whether the journals are current, or what they hold, could not be read. */
  unavailable?: boolean;
  journalsStale: boolean;
  held: number;
  suggested: number;
  linked: number;
}): TransfersVerdict {
  if (loading) return { state: "loading" };
  if (journalsStale) return { state: "stale" };
  if (unavailable) return { state: "unavailable" };
  if (held + suggested > 0) return { state: "sortOut", total: held + suggested, held, suggested };
  if (linked > 0) return { state: "settled", linked };
  return { state: "empty" };
}

/**
 * A move the owner unlinked whose legs nothing links now: a pair they
 * unlinked (kept with its kind, policy and swapped portion, so linking it
 * again restores exactly what it said) or an automatic pairing they undid.
 */
export interface RecentlyUnlinked {
  key: string;
  outTransactionId: string;
  inTransactionId: string;
  fromLabel: string;
  toLabel: string;
  fromRail: SwapRail;
  toRail: SwapRail;
  occurredAt: string | null;
  /** The sending side's asset, which its amount is in. */
  asset: string;
  amountMsat: Exact;
  inAmountMsat: Exact | null;
  unlinkedAt: string | null;
  /** The pair as it was, to link again with the same terms. */
  pair: TransferPairSummary | null;
  /** The undo of an automatic pairing, removed to link it back. */
  dismissal: TransferDismissal | null;
}

export function recentlyUnlinked({
  unlinked,
  dismissals,
  held,
  linked,
}: {
  unlinked: readonly TransferPairSummary[];
  dismissals: readonly TransferDismissal[];
  /** Undone pairings asked about under "To sort out" instead. */
  held: readonly TransferDismissal[];
  /** Moves linked now, which are not offered again. */
  linked: readonly LinkedMove[];
}): RecentlyUnlinked[] {
  const taken = new Set(
    linked.filter((move) => move.source !== "undone").flatMap((move) => [move.outTransactionId, move.inTransactionId]),
  );
  const heldKeys = new Set(held.map((dismissal) => moveKey(dismissal.out_transaction_id, dismissal.in_transaction_id)));
  const rows = new Map<string, RecentlyUnlinked>();
  for (const pair of unlinked) {
    const key = moveKey(pair.out_transaction_id, pair.in_transaction_id);
    if (rows.has(key) || taken.has(pair.out_transaction_id) || taken.has(pair.in_transaction_id)) continue;
    rows.set(key, {
      key,
      outTransactionId: pair.out_transaction_id,
      inTransactionId: pair.in_transaction_id,
      fromLabel: pair.out.wallet,
      toLabel: pair.in.wallet,
      fromRail: railForLeg(pair.out.asset, pair.out.wallet_kind),
      toRail: railForLeg(pair.in.asset, pair.in.wallet_kind),
      occurredAt: pair.out.occurred_at,
      asset: pair.out.asset,
      amountMsat: pair.out.amount_msat,
      inAmountMsat: pair.in.amount_msat,
      unlinkedAt: pair.deleted_at ?? null,
      pair,
      dismissal: null,
    });
  }
  for (const dismissal of dismissals) {
    if (!isUndoneAutomaticPairing(dismissal)) continue;
    const key = moveKey(dismissal.out_transaction_id, dismissal.in_transaction_id);
    if (rows.has(key) || heldKeys.has(key)) continue;
    if (taken.has(dismissal.out_transaction_id) || taken.has(dismissal.in_transaction_id)) continue;
    rows.set(key, {
      key,
      outTransactionId: dismissal.out_transaction_id,
      inTransactionId: dismissal.in_transaction_id,
      fromLabel: dismissal.out.wallet,
      toLabel: dismissal.in.wallet,
      fromRail: railForLeg(dismissal.out.asset, null),
      toRail: railForLeg(dismissal.in.asset, null),
      occurredAt: dismissal.out.occurred_at ?? dismissal.in.occurred_at,
      asset: dismissal.out.asset,
      amountMsat: dismissal.out.amount_msat,
      inAmountMsat: dismissal.in.amount_msat,
      unlinkedAt: dismissal.created_at,
      pair: null,
      dismissal,
    });
  }
  return [...rows.values()].sort((a, b) => {
    const left = a.unlinkedAt ?? "";
    const right = b.unlinkedAt ?? "";
    if (left !== right) return left < right ? 1 : -1;
    return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
  });
}

/** The longest end of a wallet name kept whole when the name is cut. */
const WALLET_TAIL_CHARS = 14;

/**
 * A wallet name as the part that may be cut and the end that stays, so a
 * long name reads "Regtest Liq… 2024 Rotation" rather than losing the words
 * that tell two wallets apart. The end is whole words, at most 14
 * characters, with its leading space; a short name, or one whose last word
 * alone is longer, keeps no end.
 */
export function splitWalletName(label: string): [string, string] {
  if (label.length <= WALLET_TAIL_CHARS + 4) return [label, ""];
  const words = label.split(" ");
  let cut = words.length;
  while (cut > 1 && words.slice(cut - 1).join(" ").length <= WALLET_TAIL_CHARS) cut -= 1;
  if (cut === words.length) return [label, ""];
  return [words.slice(0, cut).join(" "), ` ${words.slice(cut).join(" ")}`];
}

/** What kind of move a row is, as its short label key. */
export type MoveType = "transfer" | "pegIn" | "pegOut" | "submarine" | "reverseSubmarine" | "chainSwap" | "refund" | "coinjoin";

/**
 * The kind of move: the kind a pair records, otherwise read from the rails
 * the bitcoin moved between (an automatic link on one rail is a transfer).
 */
export function moveType(move: Pick<LinkedMove, "pairKind" | "fromRail" | "toRail">): MoveType {
  switch (move.pairKind) {
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
  }
  const { fromRail, toRail } = move;
  if (fromRail === toRail) return "transfer";
  if (toRail === "liquid" && fromRail !== "lightning") return "pegIn";
  if (fromRail === "liquid" && toRail !== "lightning") return "pegOut";
  if (toRail === "lightning") return "submarine";
  if (fromRail === "lightning") return "reverseSubmarine";
  return "transfer";
}

/**
 * Whether linking it booked a sale and a purchase. Only a pair records that
 * choice; an automatic custody link carries the cost basis over.
 */
export function moveIsTaxable(move: Pick<LinkedMove, "pairPolicy">): boolean {
  return move.pairPolicy === "taxable";
}

/** A move the owner can unlink: a pair, or an automatic link a dismissal can undo. */
export function canUnlinkMove(move: Pick<LinkedMove, "pairId" | "undoable">): boolean {
  return Boolean(move.pairId) || move.undoable;
}

/** How an automatic link was found, grouped by what the owner can do about it. */
export type AutomaticLinkKind = "sameTransaction" | "fanout" | "consolidation" | "channel" | "exchange" | "bark" | "chain";

const AUTOMATIC_LINK_KINDS: Record<string, AutomaticLinkKind> = {
  row_matched: "sameTransaction",
  verified_native_transfer: "sameTransaction",
  recorded_fanout: "fanout",
  multi_source_consolidation: "consolidation",
  channel_lifecycle: "channel",
  exchange_txid_exact: "exchange",
  bark_txid_exact: "bark",
};

/** The kind of an automatic link from its evidence; unknown evidence is plain chain evidence. */
export function automaticLinkKind(move: Pick<LinkedMove, "evidenceReason">): AutomaticLinkKind {
  return (move.evidenceReason && AUTOMATIC_LINK_KINDS[move.evidenceReason]) || "chain";
}

/** Why a link that cannot be unlinked is fixed: the exchange and Bark kinds are always undoable. */
export type FixedLinkKind = Exclude<AutomaticLinkKind, "exchange" | "bark">;

export function fixedLinkKind(move: Pick<LinkedMove, "evidenceReason">): FixedLinkKind {
  const kind = automaticLinkKind(move);
  return kind === "exchange" || kind === "bark" ? "chain" : kind;
}

/** Active dismissals the owner made from a suggestion, newest first. */
export function setAsideSuggestions(dismissals: readonly TransferDismissal[]): TransferDismissal[] {
  return dismissals
    .filter((dismissal) => dismissal.active && dismissal.reason === DISMISS_REASON)
    .sort((a, b) => (a.created_at < b.created_at ? 1 : a.created_at > b.created_at ? -1 : 0));
}
