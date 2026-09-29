import type { WalletUtxoRow } from "../UtxosInventoryPanel";

/** The existing inventory projection, also accepting explicitly unknown amounts. */
export type UtxoBlockInput = Pick<WalletUtxoRow, "outpoint" | "confirmation_status" | "asset"> & {
  amount_msat: WalletUtxoRow["amount_msat"] | null;
};

export const BLOCK_WIDTH = 0.8;
export const BLOCK_DEPTH = 0.6;
export const MIN_BLOCK_HEIGHT = 0.12;
export const MAX_BLOCK_HEIGHT = 2.8;
export const UNIFORM_BLOCK_HEIGHT = 1.2;
export const MAX_UTXO_BLOCKS = 200;

export type UtxoBlock = {
  kind: "utxo" | "more";
  outpoint: string | null;
  omittedCount: number;
  frosted: boolean;
  x: number;
  y: number;
  z: number;
  width: number;
  height: number;
  depth: number;
};

export type UtxoBlocksLayout = {
  blocks: UtxoBlock[];
  hidden: boolean;
  omittedCount: number;
};

function knownAmount(row: UtxoBlockInput): number | null {
  const amount = row.amount_msat;
  // Never guess from decimal BTC, sat fallbacks, or rounded/invalid integers.
  return amount !== null && Number.isSafeInteger(amount) && amount >= 0 ? amount : null;
}

function compareOutpoint(a: UtxoBlockInput, b: UtxoBlockInput) {
  return a.outpoint < b.outpoint ? -1 : a.outpoint > b.outpoint ? 1 : 0;
}

/**
 * Same footprint for every coin. Height is value / largest value * 2.8, with
 * only a dust floor; the scale is relative to this inventory, not other wallets.
 * Rows sit on separate shelves in the xy plane so small coins aren't occluded.
 * Hidden mode never reads amounts: ordering, selection, geometry and frosting
 * are amount-independent. Confirmation state remains visible.
 */
export function utxoBlocksLayout(
  rows: readonly UtxoBlockInput[],
  hidden = false,
): UtxoBlocksLayout {
  if (new Set(rows.map((row) => row.asset)).size > 1) {
    throw new Error("UTXO blocks require a single asset inventory");
  }
  const ordered = [...rows].sort((a, b) => {
    if (hidden) return compareOutpoint(a, b);
    const left = knownAmount(a);
    const right = knownAmount(b);
    if (left === null && right !== null) return 1;
    if (right === null && left !== null) return -1;
    return (right ?? 0) - (left ?? 0) || compareOutpoint(a, b);
  });
  const largest = hidden ? 0 : (ordered.length ? knownAmount(ordered[0]) ?? 0 : 0);
  const omittedCount = Math.max(0, rows.length - MAX_UTXO_BLOCKS);
  const blocks: UtxoBlock[] = ordered.slice(0, MAX_UTXO_BLOCKS).map((row) => {
    const amount = hidden ? null : knownAmount(row);
    return {
      kind: "utxo",
      outpoint: row.outpoint,
      omittedCount: 0,
      frosted: row.confirmation_status !== "confirmed" || (!hidden && amount === null),
      x: 0, y: 0, z: 0,
      width: BLOCK_WIDTH,
      depth: BLOCK_DEPTH,
      height: hidden || amount === null
        ? UNIFORM_BLOCK_HEIGHT
        : Math.max(MIN_BLOCK_HEIGHT, largest ? amount / largest * MAX_BLOCK_HEIGHT : 0),
    };
  });
  if (omittedCount) {
    blocks.push({
      kind: "more", outpoint: null, omittedCount, frosted: true,
      x: 0, y: 0, z: 0,
      width: BLOCK_WIDTH, depth: BLOCK_DEPTH, height: UNIFORM_BLOCK_HEIGHT,
    });
  }
  // The panel is wide and short: one shelf of up to 30 coins, then more shelves.
  const columns = Math.max(1, Math.min(blocks.length, 30));
  for (const [index, block] of blocks.entries()) {
    const row = Math.floor(index / columns);
    const rowSize = Math.min(columns, blocks.length - row * columns);
    block.x = (index % columns - (rowSize - 1) / 2) * (BLOCK_WIDTH + 0.5);
    block.y = -row * (MAX_BLOCK_HEIGHT + 0.7) + block.height / 2;
  }
  return { blocks, hidden, omittedCount };
}
