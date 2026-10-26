/*
 * The pixel wordmark, drawn on a cell grid after Omarchy's logo: 3-cell stems
 * and counters, stepped chamfers on the top-left and bottom-right corners,
 * nubs at crossbar height, and an R leg that drops below the baseline. Each
 * `#` is one filled cell, so a letter is redrawn by editing its rows here.
 * The CLI banner (kassiber/cli/banner.py) and the README's SVGs are checked
 * against this grid.
 */
export const WORDMARK_GRID = [
  "..#...#......#######....#######....#######....#..#######......#######....######.",
  ".##...##....########...########...########...##..########....########...########",
  "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
  "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
  "###...###..###...###..###...##...###...##...###..###...###..###...##...###...###",
  "###..###...###...###..###...#....###...#....###..###...###..###...#....###...###",
  "###.###....###...###..###........###........###..###...##...###........###...###",
  "######....##########..########...########..####..########...#######...#########.",
  "######....##########...########...########.####..########...######....#######...",
  "###.###....###...###........###........###..###..###...##...###........###.###..",
  "###..###...###...###....#...###....#...###..###..###...###..###...#....###..###.",
  "###...###..###...###...##...###...##...###..###..###...###..###...##...###...###",
  "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
  "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
  "###...##...###...##...########...########...##...########...########...###...###",
  "###...#....###...#....#######....#######....#....#######....#######....###...###",
  ".............................................................................##.",
  ".............................................................................#..",
];

export const WORDMARK_COLUMNS = WORDMARK_GRID[0].length;
export const WORDMARK_ROWS = WORDMARK_GRID.length;
/** The K's size in cells; the next column is the gap before the A. */
export const WORDMARK_K_COLUMNS = 9;
export const WORDMARK_K_ROWS = 16;

/**
 * How the word arrives. Decrypt settles flickering noise into the letters,
 * left to right; rain drops each cell into place; assemble gathers cells
 * scattered across the mark.
 */
export type WordmarkReveal = "decrypt" | "rain" | "assemble";

export const REVEAL_MS: Record<WordmarkReveal, number> = {
  decrypt: 1400,
  rain: 1600,
  assemble: 1800,
};
/** How often decrypt's noise reshuffles. */
const FLICKER_MS = 70;

/** Settled cells in `currentColor`; cells still moving or decoding, in orange. */
export interface WordmarkFrame {
  solid: string;
  noise: string;
}

function filled(x: number, y: number) {
  return WORDMARK_GRID[y][x] === "#";
}

/**
 * One unit-high rectangle per horizontal run of the cells `include` picks
 * within the first `columns` columns, as SVG path data in grid units.
 */
function cellsPath(
  include: (x: number, y: number) => boolean,
  columns = WORDMARK_COLUMNS,
  rows = WORDMARK_ROWS,
) {
  let d = "";
  for (let y = 0; y < rows; y++) {
    let x = 0;
    while (x < columns) {
      if (!include(x, y)) {
        x++;
        continue;
      }
      const start = x;
      while (x < columns && include(x, y)) x++;
      d += `M${start} ${y}h${x - start}v1h${start - x}z`;
    }
  }
  return d;
}

/** One square per cell, for cells off the grid while they move. */
function loosePath(cells: Array<[number, number]>) {
  let d = "";
  for (const [x, y] of cells) d += `M${x} ${y}h1v1h-1z`;
  return d;
}

export const WORDMARK_PATH = cellsPath(filled);
export const WORDMARK_K_PATH = cellsPath(filled, WORDMARK_K_COLUMNS, WORDMARK_K_ROWS);

const FILLED_CELLS: Array<[number, number]> = [];
for (let y = 0; y < WORDMARK_ROWS; y++) {
  for (let x = 0; x < WORDMARK_COLUMNS; x++) {
    if (filled(x, y)) FILLED_CELLS.push([x, y]);
  }
}

/** A stable pseudo-random value in [0, 1) for a cell and a seed. */
function hash(x: number, y: number, seed: number) {
  let h =
    Math.imul(x, 374761393) +
    Math.imul(y, 668265263) +
    Math.imul(seed, 1442695041);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 2 ** 32;
}

const clamp01 = (t: number) => Math.min(1, Math.max(0, t));
const easeOutCubic = (t: number) => 1 - (1 - t) ** 3;
const easeInOutCubic = (t: number) =>
  t < 0.5 ? 4 * t ** 3 : 1 - (-2 * t + 2) ** 3 / 2;

/**
 * Decrypt, `elapsed` ms in, over the first `columns` columns. Columns settle
 * left to right with a little per-cell jitter, so the word reads as decoded
 * rather than wiped in; unsettled cells flicker.
 */
export function decryptFrame(
  elapsed: number,
  columns = WORDMARK_COLUMNS,
  rows = WORDMARK_ROWS,
): WordmarkFrame {
  const t = elapsed / REVEAL_MS.decrypt;
  const tick = Math.floor(elapsed / FLICKER_MS) + 1;
  const settled = (x: number, y: number) =>
    t >= 0.3 + (0.6 * x) / columns + 0.08 * hash(x, y, 0);
  return {
    solid: cellsPath((x, y) => filled(x, y) && settled(x, y), columns, rows),
    noise: cellsPath(
      (x, y) =>
        !settled(x, y) && hash(x, y, tick) < (filled(x, y) ? 0.55 : 0.1),
      columns,
      rows,
    ),
  };
}

/** Rain: cells fall from above the mark, in a ripple from the left. */
function rainFrame(elapsed: number): WordmarkFrame {
  const t = elapsed / REVEAL_MS.rain;
  const landed: Array<[number, number]> = [];
  const falling: Array<[number, number]> = [];
  for (const [x, y] of FILLED_CELLS) {
    const start = (x / WORDMARK_COLUMNS) * 0.45 + hash(x, y, 1) * 0.15;
    const p = clamp01((t - start) / 0.35);
    if (p <= 0) continue;
    if (p >= 1) {
      landed.push([x, y]);
      continue;
    }
    falling.push([x, Math.round(-4 + (y + 4) * easeOutCubic(p))]);
  }
  return { solid: loosePath(landed), noise: loosePath(falling) };
}

/** Assemble: cells start scattered across the mark and gather into the word. */
function assembleFrame(elapsed: number): WordmarkFrame {
  const t = elapsed / REVEAL_MS.assemble;
  const home: Array<[number, number]> = [];
  const moving: Array<[number, number]> = [];
  for (const [x, y] of FILLED_CELLS) {
    const p = easeInOutCubic(clamp01((t - hash(x, y, 4) * 0.3) / 0.6));
    if (p >= 1) {
      home.push([x, y]);
      continue;
    }
    const fromX = hash(x, y, 2) * (WORDMARK_COLUMNS - 1);
    const fromY = hash(x, y, 3) * (WORDMARK_ROWS - 1);
    moving.push([
      Math.round(fromX + (x - fromX) * p),
      Math.round(fromY + (y - fromY) * p),
    ]);
  }
  return { solid: loosePath(home), noise: loosePath(moving) };
}

/** One frame of `reveal`, `elapsed` ms in; past its length, the plain word. */
export function revealFrame(
  reveal: WordmarkReveal,
  elapsed: number,
): WordmarkFrame {
  if (elapsed >= REVEAL_MS[reveal]) return { solid: WORDMARK_PATH, noise: "" };
  if (reveal === "rain") return rainFrame(elapsed);
  if (reveal === "assemble") return assembleFrame(elapsed);
  return decryptFrame(elapsed);
}

/** The K's working loop: decrypt in, hold, scramble back out, repeat. */
const WORKING_HOLD_MS = 500;
export const WORKING_CYCLE_MS = 2 * REVEAL_MS.decrypt + WORKING_HOLD_MS;

export function workingFrame(elapsed: number): WordmarkFrame {
  const at = elapsed % WORKING_CYCLE_MS;
  const decryptAt =
    at < REVEAL_MS.decrypt
      ? at
      : at < REVEAL_MS.decrypt + WORKING_HOLD_MS
        ? REVEAL_MS.decrypt
        : WORKING_CYCLE_MS - at;
  return decryptFrame(decryptAt, WORDMARK_K_COLUMNS, WORDMARK_K_ROWS);
}
