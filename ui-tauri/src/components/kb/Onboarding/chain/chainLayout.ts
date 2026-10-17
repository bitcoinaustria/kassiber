/**
 * Setup's Bitcoin artwork: a short chain of blocks packed with transactions,
 * and the next block, an outline, filling from the mempool. Pure geometry
 * from fixed seeds, so a block keeps the same transactions wherever it sits
 * in the chain. It is decoration: no block height, fee, or transaction is
 * real or implied.
 */

/** Outer edge of one block. */
export const BLOCK_SIZE = 1.6;
/** Gap between neighbouring blocks in the chain. */
export const BLOCK_GAP = 0.55;
/**
 * The chain recedes diagonally behind the next block (left and back), so the
 * newest block sits in front and the history trails away from it.
 */
const RECEDE_X = -0.62;
const RECEDE_Z = -0.78;
/**
 * Transaction layers in a full block: one per setup step before Review, so
 * the rail's block is full when the user reaches Review.
 */
export const BLOCK_LAYERS = 4;
/** Transaction cells along each side of a layer. */
const GRID = 4;
/** Inset between a shell's outer edge and the transactions inside it. */
const WALL = 0.1;
/** Gap between neighbouring transactions. */
const TX_GAP = 0.035;
/** Share of transactions drawn as high-fee (orange) ones. */
const FEE_SHARE = 0.18;

export type ChainBox = {
  /**
   * `shell` is the next block's outline; `tx` and `fee` are transactions in
   * a block; `pending` are unconfirmed transactions waiting above it.
   */
  kind: "shell" | "tx" | "fee" | "pending";
  x: number;
  y: number;
  z: number;
  width: number;
  height: number;
  depth: number;
};

export type ChainLayout = {
  boxes: ChainBox[];
  /** Layers of transactions in the next block, 0 to `BLOCK_LAYERS`. */
  nextLayers: number;
};

/** A small deterministic PRNG (mulberry32), so the artwork never shifts. */
function random(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Footprints a transaction may take, in grid cells, largest first. */
const FOOTPRINTS: Array<[number, number]> = [
  [2, 2],
  [2, 1],
  [1, 2],
  [1, 1],
];

/**
 * One layer of transactions tiling a GRID×GRID floor without overlap: each
 * free cell takes the largest footprint that fits and the dice allow, so a
 * layer mixes a few large transactions with many small ones, as blocks do.
 */
function layerTiles(next: () => number) {
  const taken = Array.from({ length: GRID }, () => Array<boolean>(GRID).fill(false));
  const tiles: Array<{ col: number; row: number; cols: number; rows: number }> = [];
  for (let row = 0; row < GRID; row += 1) {
    for (let col = 0; col < GRID; col += 1) {
      if (taken[row][col]) continue;
      const roll = next();
      const wanted = roll < 0.2 ? 0 : roll < 0.4 ? 1 : roll < 0.6 ? 2 : 3;
      const [cols, rows] =
        FOOTPRINTS.slice(wanted).find(([width, height]) => {
          if (col + width > GRID || row + height > GRID) return false;
          for (let r = row; r < row + height; r += 1) {
            for (let c = col; c < col + width; c += 1) {
              if (taken[r][c]) return false;
            }
          }
          return true;
        }) ?? [1, 1];
      for (let r = row; r < row + rows; r += 1) {
        for (let c = col; c < col + cols; c += 1) taken[r][c] = true;
      }
      tiles.push({ col, row, cols, rows });
    }
  }
  return tiles;
}

/** Where block `index` back from the next block (0 = the next block) sits. */
export function blockCentre(index: number) {
  const pitch = BLOCK_SIZE + BLOCK_GAP;
  return { x: index * pitch * RECEDE_X, z: index * pitch * RECEDE_Z };
}

/**
 * Transactions for `layers` layers of block `id`, centred on (`cx`, 0, `cz`).
 * The id seeds them, and layers are drawn bottom up from one stream, so a
 * block's lower layers stay put as it fills.
 */
function blockTransactions(
  cx: number,
  cz: number,
  layers: number,
  id: number,
): ChainBox[] {
  const next = random(0x6b617373 ^ Math.imul(id, 0x9e3779b1));
  const inner = BLOCK_SIZE - WALL * 2;
  const cell = inner / GRID;
  const layerHeight = inner / BLOCK_LAYERS;
  const floor = -BLOCK_SIZE / 2 + WALL;
  const left = cx - inner / 2;
  const back = cz - inner / 2;
  const boxes: ChainBox[] = [];
  for (let layer = 0; layer < layers; layer += 1) {
    for (const tile of layerTiles(next)) {
      // Transactions in a layer differ a little in height, as they do in size,
      // but stay close enough to the layer that a full block reads as solid.
      const height = layerHeight * (0.86 + next() * 0.14) - TX_GAP;
      boxes.push({
        kind: next() < FEE_SHARE ? "fee" : "tx",
        x: left + (tile.col + tile.cols / 2) * cell,
        y: floor + layer * layerHeight + TX_GAP / 2 + height / 2,
        z: back + (tile.row + tile.rows / 2) * cell,
        width: tile.cols * cell - TX_GAP,
        height,
        depth: tile.rows * cell - TX_GAP,
      });
    }
  }
  return boxes;
}

/** How many unconfirmed transactions wait above a block that is not full. */
export const PENDING_COUNT = 6;

/**
 * The mempool: a few unconfirmed transactions drifting down toward the next
 * block, scattered over its footprint at staggered heights.
 */
function pendingTransactions(): ChainBox[] {
  const next = random(0x6d656d70);
  const half = BLOCK_SIZE / 2;
  const boxes: ChainBox[] = [];
  for (let index = 0; index < PENDING_COUNT; index += 1) {
    const size = 0.2 + next() * 0.2;
    const reach = half - size / 2 - WALL;
    boxes.push({
      kind: "pending",
      x: (next() * 2 - 1) * reach,
      y: half + 0.3 + (index / PENDING_COUNT) * 1.3 + next() * 0.15,
      z: (next() * 2 - 1) * reach,
      width: size,
      height: size,
      depth: size,
    });
  }
  return boxes;
}

/** One block of the chain, its boxes relative to its own centre. */
export type ChainBlock = {
  /** Stable across the chain advancing: the block's own sequence number. */
  id: number;
  /** Slots back from the next block: 0 is the next block, 1 the newest mined. */
  offset: number;
  layers: number;
  boxes: ChainBox[];
};

/** A block's outline, relative to its centre. */
const SHELL: ChainBox = {
  kind: "shell",
  x: 0,
  y: 0,
  z: 0,
  width: BLOCK_SIZE,
  height: BLOCK_SIZE,
  depth: BLOCK_SIZE,
};

/**
 * The blocks on screen when block `tip` is the next one, holding `layers`
 * layers, with `mined` full blocks behind it, oldest first. `trailing` adds
 * the block just past the last mined slot, so it can fade out as the chain
 * advances instead of vanishing.
 */
export function chainBlocks({
  tip,
  mined,
  layers,
  trailing = false,
}: {
  tip: number;
  mined: number;
  layers: number;
  trailing?: boolean;
}): ChainBlock[] {
  const minedCount = Math.max(0, Math.floor(mined));
  const nextLayers = Math.min(BLOCK_LAYERS, Math.max(0, Math.round(layers)));
  const blocks: ChainBlock[] = [];
  for (let offset = minedCount + (trailing ? 1 : 0); offset >= 0; offset -= 1) {
    const id = tip - offset;
    const blockLayers = offset === 0 ? nextLayers : BLOCK_LAYERS;
    blocks.push({
      id,
      offset,
      layers: blockLayers,
      boxes: [...blockTransactions(0, 0, blockLayers, id), SHELL],
    });
  }
  return blocks;
}

/** The mempool above the next block, relative to its centre. */
export function mempool(): ChainBox[] {
  return pendingTransactions();
}

/**
 * `mined` full blocks behind the next block, which holds `fill` of a block's
 * worth of transactions (0 to 1, rounded to whole layers), flattened into one
 * list in chain coordinates. Only the next block shows its outline; mined
 * blocks are packed solid instead. The mempool waits until the block is full.
 */
export function chainLayout({
  mined,
  fill,
}: {
  mined: number;
  fill: number;
}): ChainLayout {
  const nextLayers = Math.round(Math.min(1, Math.max(0, fill)) * BLOCK_LAYERS);
  const boxes: ChainBox[] = [];
  for (const block of chainBlocks({ tip: 0, mined, layers: nextLayers })) {
    const { x, z } = blockCentre(block.offset);
    for (const box of block.boxes) {
      if (box.kind === "shell" && block.offset !== 0) continue;
      boxes.push({ ...box, x: box.x + x, z: box.z + z });
    }
  }
  if (nextLayers < BLOCK_LAYERS) boxes.push(...pendingTransactions());
  return { boxes, nextLayers };
}

/** Where the live chain is: which block is next, how full, and whether found. */
export type LoopState = { tip: number; layers: number; found: boolean };

/**
 * The live chain's rhythm: the next block fills a layer per beat; full, it is
 * found (one beat, its outline flashing); then the chain moves back one slot
 * and an empty block takes the front.
 */
export function advanceChain(state: LoopState): LoopState {
  if (state.found) return { tip: state.tip + 1, layers: 0, found: false };
  if (state.layers >= BLOCK_LAYERS) return { ...state, found: true };
  return { ...state, layers: state.layers + 1 };
}
