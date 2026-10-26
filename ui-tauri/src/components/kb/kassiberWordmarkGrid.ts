/*
 * The pixel wordmark, drawn on a cell grid after Omarchy's logo: 3-cell stems
 * and counters, stepped chamfers on the top-left and bottom-right corners,
 * nubs at crossbar height, and an R leg that drops below the baseline. Each
 * `#` is one filled cell, so a letter is redrawn by editing its rows here.
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
/** The K's width in cells; the next column is the gap before the A. */
export const WORDMARK_K_COLUMNS = 9;

/** Length of the decrypt reveal, and how often its noise reshuffles. */
export const DECRYPT_MS = 1400;
const FLICKER_MS = 70;

function filled(x: number, y: number) {
  return WORDMARK_GRID[y][x] === "#";
}

/**
 * One unit-high rectangle per horizontal run of the cells `include` picks,
 * as SVG path data in grid units.
 */
function cellsPath(include: (x: number, y: number) => boolean) {
  let d = "";
  for (let y = 0; y < WORDMARK_ROWS; y++) {
    let x = 0;
    while (x < WORDMARK_COLUMNS) {
      if (!include(x, y)) {
        x++;
        continue;
      }
      const start = x;
      while (x < WORDMARK_COLUMNS && include(x, y)) x++;
      d += `M${start} ${y}h${x - start}v1h${start - x}z`;
    }
  }
  return d;
}

export const WORDMARK_PATH = cellsPath(filled);

/** A stable pseudo-random value in [0, 1) for a cell and a seed. */
function hash(x: number, y: number, seed: number) {
  let h =
    Math.imul(x, 374761393) +
    Math.imul(y, 668265263) +
    Math.imul(seed, 1442695041);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 2 ** 32;
}

/**
 * One frame of the decrypt reveal, `elapsed` ms in: the cells that have
 * settled into the word, and the flickering noise still covering the rest.
 * Columns settle left to right with a little per-cell jitter, so the word
 * reads as being decoded rather than wiped in.
 */
export function decryptFrame(elapsed: number) {
  const t = elapsed / DECRYPT_MS;
  const tick = Math.floor(elapsed / FLICKER_MS) + 1;
  const settled = (x: number, y: number) =>
    t >= 0.3 + (0.6 * x) / WORDMARK_COLUMNS + 0.08 * hash(x, y, 0);
  return {
    solid: cellsPath((x, y) => filled(x, y) && settled(x, y)),
    noise: cellsPath(
      (x, y) =>
        !settled(x, y) && hash(x, y, tick) < (filled(x, y) ? 0.55 : 0.1),
    ),
  };
}
