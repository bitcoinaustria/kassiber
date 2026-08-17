/**
 * Block-deck chrome art — the chain, as wallpaper.
 *
 * This replaces the ledger-paper art that used to sit on these surfaces. Ruled
 * accounting stock said what the app *is*; blocks of confirmed transactions say
 * what it is *about*, which is the more interesting half and the one a Bitcoin
 * user recognises on sight. One motif now, on all three surfaces that carry
 * chrome art: the side-nav header band, the lock screen and setup.
 *
 * Geometry follows the block view in the mempool project, by way of the
 * marketing site's `LeakGoggles`: a block is a square grid of cells, every
 * transaction is a square some whole number of cells wide, and the deck is
 * generated once at module scope from a fixed seed so it is byte-identical every
 * launch and costs nothing per render.
 *
 * Toned down against both of those, because this is chrome and not an argument:
 * no colour at all, no fee-rate scale, no highlighted outputs, no tooltips. The
 * whole field is ink on paper from the same `--stage-*` palette the app already
 * had (see `.kb-stage-blocks` in globals.css), so light and dark follow the theme
 * for free, and the only variation is how dark a square sits.
 *
 * It comes in two treatments, and the split is by how close the art sits to
 * something you have to read:
 *
 * - The two full surfaces (lock screen, setup) get the field unmasked, edge to
 *   edge, drifting slowly. They are backdrops — nothing else is on them.
 * - The nav's 56px strip gets it still, zoomed in, and masked into the nav
 *   surface. It sits beside the wordmark and above every nav row, where a moving
 *   edge is something to look at instead of the nav.
 *
 * The drift is one CSS animation on the tiled rect, so the loop is seamless,
 * costs no script, and stops dead under `prefers-reduced-motion` (globals.css
 * already zeroes every animation there, and `.kb-stage-blocks-field` is covered
 * by that rule).
 */
import * as React from "react";

import { cn } from "@/lib/utils";

/** A wide viewBox holds the art at a fixed scale while its container resizes. */
const STAGE_WIDTH = 8192;

/** Block geometry, in viewBox units. */
const COLS = 24;
const ROWS = 24;
const CELL = 15;
/** Gap between squares, taken out of every side. */
const PAD = 2;
const BLOCK = COLS * CELL;
const BLOCK_GAP = 22;
/**
 * Blocks per deck, and it has to stay ODD. The second row of the tile shows the
 * deck shifted by half its length, which is what stops a block sitting directly
 * above the block it repeats — and with an even count that half-length is a
 * whole number of block pitches, so the two rows line up and the field reads as
 * one flat lattice. An odd count makes the same shift a half-pitch too, which
 * lays the rows like brickwork for free.
 */
const BLOCKS = 7;
/** The tile includes a trailing gap, so repeats keep their spacing. */
const DECK_W = BLOCKS * (BLOCK + BLOCK_GAP);
/** Two block rows per tile, the second offset, so a repeat is never adjacent. */
const TILE_H = 2 * (BLOCK + BLOCK_GAP);
/**
 * CSS pixels per viewBox unit: `BLOCK * ART_SCALE` is a block's on-screen size,
 * ~225px with ~9px cells, near the mempool block view's own proportions.
 */
const ART_SCALE = 0.63;
/**
 * The nav strip is zoomed further in, which is the only way a 56px band carries
 * a legible motif: at the field scale it shows ~6 rows of small cells and reads
 * as generic noise, while at this scale a dozen big squares cross it and the
 * shapes are plainly transactions. Zooming in *is* the detail reduction — same
 * art, fewer things in frame.
 */
const NAV_SCALE = 1.2;

export type DeckSquare = {
  x: number;
  y: number;
  s: number;
  cells: number;
  ink: number;
};

// xorshift32 on a fixed seed: exact 32-bit integer maths, so the deck is the
// same on every platform and in the test, unlike a float LCG that overflows
// past 2^53.
let seed = 20090103 >>> 0;
const rnd = () => {
  seed ^= seed << 13;
  seed >>>= 0;
  seed ^= seed >>> 17;
  seed ^= seed << 5;
  seed >>>= 0;
  return seed / 0x100000000;
};

/**
 * Fill strengths, faintest first, by square size — the only variation in the
 * field, and all of it. Protocol colours were tried here (orange/teal/violet on
 * a small minority of squares, as on the marketing site) and pulled: what works
 * as an argument on a landing page reads as confetti behind a passphrase prompt.
 */
const INK = [0.05, 0.07, 0.09, 0.12];

function buildDeck(): DeckSquare[] {
  const squares: DeckSquare[] = [];
  for (let b = 0; b < BLOCKS; b++) {
    const ox = b * (BLOCK + BLOCK_GAP);
    const occ = Array.from({ length: ROWS }, () => Array(COLS).fill(false));

    const free = (r: number, c: number, n: number) => {
      for (let y = r; y < r + n; y++)
        for (let x = c; x < c + n; x++) if (occ[y][x]) return false;
      return true;
    };
    const put = (r: number, c: number, n: number) => {
      for (let y = r; y < r + n; y++)
        for (let x = c; x < c + n; x++) occ[y][x] = true;
      squares.push({
        x: ox + c * CELL + PAD,
        y: r * CELL + PAD,
        s: n * CELL - PAD * 2,
        cells: n,
        // Size sets the tier, and a coin flip lifts some to the next one, so
        // equal-sized neighbours are not all the same shade.
        ink: INK[Math.min(INK.length - 1, n - 1 + (rnd() > 0.6 ? 1 : 0))],
      });
    };
    /*
     * Mempool places largest-first into the lowest free slot, because it orders
     * by fee. With no fee to order by that rule stacks every big square along
     * one edge, so scatter them and let the single-cell pass close the gaps —
     * which leaves the block just as full.
     */
    const place = (n: number) => {
      for (let t = 0; t < 60; t++) {
        const r = Math.floor(rnd() * (ROWS - n + 1));
        const c = Math.floor(rnd() * (COLS - n + 1));
        if (free(r, c, n)) {
          put(r, c, n);
          return true;
        }
      }
      for (let r = 0; r + n <= ROWS; r++)
        for (let c = 0; c + n <= COLS; c++)
          if (free(r, c, n)) {
            put(r, c, n);
            return true;
          }
      return false;
    };

    // Composition varies per block, so seven of them do not read as one tile.
    const plan = [
      { n: 4, k: Math.floor(rnd() * 3) },
      { n: 3, k: 4 + Math.floor(rnd() * 6) },
      { n: 2, k: 62 + Math.floor(rnd() * 26) },
    ];
    for (const { n, k } of plan)
      for (let i = 0; i < k; i++) if (!place(n)) break;
    // Whatever is still open is a single cell, which is most of a real block.
    for (let r = 0; r < ROWS; r++)
      for (let c = 0; c < COLS; c++) if (!occ[r][c]) put(r, c, 1);
  }
  return squares;
}

/** The deck, built once. Exported for the packing test. */
export const BLOCK_DECK = buildDeck();
export const DECK_GEOMETRY = {
  COLS,
  ROWS,
  CELL,
  PAD,
  BLOCK,
  BLOCK_GAP,
  BLOCKS,
};

/**
 * The nav header's accent strip.
 *
 * Still, zoomed in, and dissolved into the nav by `.kb-stage-backdrop`'s mask —
 * the opposite treatment to the full-surface bands, and for one reason: this
 * sits two centimetres from the wordmark and behind the book switcher, where a
 * moving edge is something to look at instead of the nav. It is texture on the
 * chrome, not a picture.
 */
export function SidebarStageBackdrop() {
  return <BlockDeckBand className="h-14" scale={NAV_SCALE} still faded />;
}

/**
 * The block field as a band across a surface.
 *
 * The band measures itself rather than taking a fixed viewBox, because
 * `preserveAspectRatio="slice"` fits the shorter axis: a fixed viewBox on a
 * viewport-tall band would magnify the blocks until a single one filled the
 * screen. Deriving the height in viewBox units from the measured height instead
 * holds the block at exactly `BLOCK * scale` px in every window. The bottom row
 * is free to be cut off mid-block — real block views do that at every viewport
 * edge, and rounding to whole rows would make the block size jump between
 * window heights instead.
 *
 * By default the art runs the full band with no mask: it is the surface's own
 * backdrop, so it should reach the edges the way a block view does. `faded`
 * hands it back to `.kb-stage-backdrop`, which dissolves it into `--card`, and
 * that is what the nav strip wants — a 56px band ending on a visible horizontal
 * edge would read as a stripe glued across the top of the nav.
 */
export function BlockDeckBand({
  className,
  scale = ART_SCALE,
  still = false,
  faded = false,
}: {
  className?: string;
  scale?: number;
  /** Hold the field still instead of letting it drift. */
  still?: boolean;
  /** Mask the art into `--card` instead of letting it fill the band. */
  faded?: boolean;
}) {
  const hostRef = React.useRef<HTMLDivElement | null>(null);
  const [artHeight, setArtHeight] = React.useState(TILE_H);

  React.useLayoutEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const measure = () => {
      const height = host.getBoundingClientRect().height;
      if (height) setArtHeight(Math.max(1, height / scale));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => observer.disconnect();
  }, [scale]);

  return (
    <div
      ref={hostRef}
      aria-hidden="true"
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 z-0 overflow-hidden select-none",
        faded && "kb-stage-backdrop",
        className,
      )}
    >
      <BlockDeckArt artHeight={artHeight} still={still} />
    </div>
  );
}

function BlockDeckArt({
  artHeight,
  still,
}: {
  artHeight: number;
  still: boolean;
}) {
  // `useId` output can contain ":" which is invalid inside a url(#…) reference.
  const idPrefix = React.useId().replace(/:/g, "");
  const paperId = `${idPrefix}-paper`;
  const glowId = `${idPrefix}-glow`;
  const glowFarId = `${idPrefix}-glow-far`;
  const deckId = `${idPrefix}-deck`;
  const fieldId = `${idPrefix}-field`;

  return (
    <svg
      className="kb-stage-blocks h-full w-full"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
      viewBox={`0 0 ${STAGE_WIDTH} ${artHeight}`}
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        {/* The stock the blocks are printed on. Reflected so the tone breathes
            across a wide band instead of banding once and going flat. */}
        <linearGradient
          id={paperId}
          x1="0"
          y1="0"
          x2="420"
          y2={artHeight}
          gradientUnits="userSpaceOnUse"
          spreadMethod="reflect"
        >
          <stop style={{ stopColor: "var(--stage-paper-top)" }} />
          <stop offset="0.55" style={{ stopColor: "var(--stage-paper-mid)" }} />
          <stop offset="1" style={{ stopColor: "var(--stage-paper-bottom)" }} />
        </linearGradient>

        {/* Light falling across the field, so it is not a flat grid. Two
            dissimilar lights, not a repeating tile: `slice` fits the height, so
            only the first few thousand units are ever on screen and a tiled
            highlight would be the one thing that reads as texture. */}
        <radialGradient
          id={glowId}
          cx="0"
          cy="0"
          r="1"
          gradientTransform={`translate(340 ${artHeight * 0.1}) rotate(132) scale(760 620)`}
          gradientUnits="userSpaceOnUse"
        >
          <stop
            style={{
              stopColor: "var(--stage-glow)",
              stopOpacity: "var(--stage-glow-peak)",
            }}
          />
          <stop
            offset="0.55"
            style={{
              stopColor: "var(--stage-glow)",
              stopOpacity: "var(--stage-glow-soft)",
            }}
          />
          <stop
            offset="1"
            style={{ stopColor: "var(--stage-glow)" }}
            stopOpacity="0"
          />
        </radialGradient>
        <radialGradient
          id={glowFarId}
          cx="0"
          cy="0"
          r="1"
          gradientTransform={`translate(2600 ${artHeight * 0.52}) rotate(118) scale(1300 900)`}
          gradientUnits="userSpaceOnUse"
        >
          <stop
            style={{
              stopColor: "var(--stage-glow)",
              stopOpacity: "var(--stage-glow-soft)",
            }}
          />
          <stop
            offset="1"
            style={{ stopColor: "var(--stage-glow)" }}
            stopOpacity="0"
          />
        </radialGradient>

        <g id={deckId}>
          {BLOCK_DECK.map((q) => (
            <rect
              key={`${q.x}-${q.y}`}
              x={q.x}
              y={q.y}
              width={q.s}
              height={q.s}
              rx="1.6"
              fill="var(--stage-ink)"
              fillOpacity={q.ink}
            />
          ))}
        </g>

        {/*
         * The field: one deck along the top row, and the same deck shifted half
         * its length along the second, so a block is never directly above the
         * block it repeats. Content outside the tile is clipped, so the second
         * row needs the copy on both sides to cover the full width.
         */}
        <pattern
          id={fieldId}
          width={DECK_W}
          height={TILE_H}
          patternUnits="userSpaceOnUse"
        >
          <use href={`#${deckId}`} />
          <use
            href={`#${deckId}`}
            transform={`translate(${-DECK_W / 2} ${BLOCK + BLOCK_GAP})`}
          />
          <use
            href={`#${deckId}`}
            transform={`translate(${DECK_W / 2} ${BLOCK + BLOCK_GAP})`}
          />
        </pattern>
      </defs>

      <rect width="100%" height={artHeight} fill={`url(#${paperId})`} />
      {/*
       * The drift. Travelling exactly one deck and looping is what makes it
       * seamless, so the distance has to be the pattern's own period — handed to
       * the keyframes as a variable, since only this module knows it.
       *
       * The rect is overhung by that period at BOTH ends, which is what it takes
       * to still cover `0…STAGE_WIDTH` at the far end of the cycle: one period
       * of overhang is spent getting the leading edge past 0, so the trailing
       * edge needs its own, or it walks into view on a wide enough window.
       */}
      <rect
        className={still ? undefined : "kb-stage-blocks-field"}
        x={-DECK_W}
        width={STAGE_WIDTH + 2 * DECK_W}
        height={artHeight}
        fill={`url(#${fieldId})`}
        style={{ "--kb-deck-w": `${DECK_W}px` } as React.CSSProperties}
      />
      <rect width="100%" height={artHeight} fill={`url(#${glowId})`} />
      <rect width="100%" height={artHeight} fill={`url(#${glowFarId})`} />
    </svg>
  );
}
