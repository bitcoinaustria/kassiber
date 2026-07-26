/**
 * Ledger-paper nav art.
 *
 * The mechanism is ported from T3Code's `SidebarStageBackdrop`: a decorative
 * inline SVG behind the side-nav header. Patterns tile across a deliberately
 * huge viewBox so widening the nav reveals more canvas rather than stretching
 * the scene, and a CSS mask dissolves the whole thing into the nav surface (see
 * `.kb-stage-backdrop`).
 *
 * The motif is Kassiber's own, not T3Code's. Where T3Code draws blueprint grid
 * paper and a night sky in its brand blue, this draws **ledger paper** — ruled
 * writing lines, faint column rules and bookkeeping ticks, in the Bitcoin
 * Austria palette. An accounting app's chrome may as well look like the thing
 * the app is about.
 *
 * One page, every build. T3Code varies its art by release channel (nothing on
 * stable, blueprint on dev, night sky on nightly); this deliberately does not —
 * it is chrome, and the channel is already stated in the nav footer's version
 * line, so there is nothing for a second encoding to add.
 *
 * Light and dark ARE different pages, though — that is a surface difference, not
 * a channel one. Light mode is pale stock with dark ink (so the header text stays
 * dark); dark mode is a graphite sheet lifted off the near-black nav with light
 * ink (so the header text is relit to white). See `.kb-stage-ledger`.
 */
import * as React from "react";

import { cn } from "@/lib/utils";

// A wide viewBox keeps the art at a fixed scale while its container resizes, so
// the rules never stretch (T3Code's trick, and its width).
const STAGE_WIDTH = 8192;
/** One ledger page, in viewBox units — the nav band's full height. */
const STAGE_PAGE = 96;

/** Ledger geometry, in viewBox units. */
const RULE_HEIGHT = 12;
const COLUMN_WIDTH = 96;
/** Spacing between repeats of the lighting, in viewBox units. */
const GLOW_TILE = 2048;
/** Base spacing between repeats of the bookkeeping marks, in viewBox units. */
const MARK_TILE = 576;

export function SidebarStageBackdrop() {
  return <LedgerStageBand className="h-14" />;
}

/**
 * The ledger art as a band across the top of any surface.
 *
 * `pages` buys height in *viewBox* units rather than by scaling the art up:
 * `preserveAspectRatio="slice"` fits the shorter axis, so a taller band with a
 * one-page viewBox would simply magnify the rules until the ledger read as
 * stripes. Asking for more pages keeps the ruling at roughly the nav's rhythm
 * and reveals more of the page instead.
 *
 * `fade` is the colour the art dissolves into at its bottom edge — whatever the
 * host surface is painted with, since the mask has to land on it invisibly.
 */
export function LedgerStageBand({
  className,
  fade,
  pages = 1,
}: {
  className?: string;
  fade?: string;
  pages?: number;
}) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "kb-stage-backdrop pointer-events-none absolute inset-x-0 top-0 z-0 overflow-hidden select-none",
        className,
      )}
      style={
        fade ? ({ "--stage-fade": fade } as React.CSSProperties) : undefined
      }
    >
      <LedgerPaperArt pages={pages} />
    </div>
  );
}

function LedgerPaperArt({ pages = 1 }: { pages?: number }) {
  const artHeight = STAGE_PAGE * pages;
  // One tile with one set of marks for the nav; a doubled tile carrying two
  // offset sets for anything larger, which is the same marks at half density.
  const markSpread = pages > 1 ? 2 : 1;
  const markOffsets =
    markSpread === 1
      ? [[0, 0]]
      : [
          [0, 0],
          [MARK_TILE, STAGE_PAGE],
        ];
  // `useId` output can contain ":" which is invalid inside a url(#…) reference.
  const idPrefix = React.useId().replace(/:/g, "");
  const paperId = `${idPrefix}-paper`;
  const rulesId = `${idPrefix}-rules`;
  const columnsId = `${idPrefix}-columns`;
  const marksId = `${idPrefix}-marks`;
  const glowId = `${idPrefix}-glow`;
  const glowFarId = `${idPrefix}-glow-far`;
  const glowsId = `${idPrefix}-glows`;

  return (
    <svg
      className="kb-stage-ledger h-full w-full"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
      viewBox={`0 0 ${STAGE_WIDTH} ${artHeight}`}
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        {/* Paper. Reflected so the tone breathes across a very wide nav instead
            of banding once and going flat. */}
        <linearGradient
          id={paperId}
          x1="0"
          y1="0"
          x2="320"
          y2={artHeight}
          gradientUnits="userSpaceOnUse"
          spreadMethod="reflect"
        >
          <stop style={{ stopColor: "var(--stage-paper-top)" }} />
          <stop offset="0.55" style={{ stopColor: "var(--stage-paper-mid)" }} />
          <stop offset="1" style={{ stopColor: "var(--stage-paper-bottom)" }} />
        </linearGradient>

        {/* Light falling across the page, so the paper is not a flat fill. */}
        <radialGradient
          id={glowId}
          cx="0"
          cy="0"
          r="1"
          gradientTransform="translate(232 6) rotate(142) scale(150 96)"
          gradientUnits="userSpaceOnUse"
        >
          {/* Opacity comes from the palette (`.kb-stage-ledger`), not a literal,
              because the two themes need different glow strengths: the header
              text is dark on light mode's pale stock, but white on dark mode's
              graphite sheet, where the glow's peak is what the wordmark has to
              stay legible against. */}
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
        {/* A second, wider and dimmer light, offset from the first. */}
        <radialGradient
          id={glowFarId}
          cx="0"
          cy="0"
          r="1"
          gradientTransform={`translate(1180 ${artHeight * 0.34}) rotate(118) scale(260 150)`}
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
        {/*
         * GLOW_TILE, not the ~700 the nav needed: the nav is a few hundred units
         * wide so it only ever sees the first light, but a full-screen band is
         * thousands of units across and a short tile repeated the same highlight
         * four or five times in a row — the one thing that reads unmistakably as
         * a tiled texture. A wide tile carrying two dissimilar lights shows at
         * most one repeat on a normal window.
         */}
        <pattern
          id={glowsId}
          width={GLOW_TILE}
          height={artHeight}
          patternUnits="userSpaceOnUse"
        >
          <rect width={GLOW_TILE} height={artHeight} fill={`url(#${glowId})`} />
          <rect
            width={GLOW_TILE}
            height={artHeight}
            fill={`url(#${glowFarId})`}
          />
        </pattern>

        {/* Ruled writing lines — the ledger's defining feature. */}
        <pattern
          id={rulesId}
          width={COLUMN_WIDTH}
          height={RULE_HEIGHT}
          patternUnits="userSpaceOnUse"
        >
          <path
            d={`M0 ${RULE_HEIGHT - 0.5}H${COLUMN_WIDTH}`}
            style={{ stroke: "var(--stage-rule)" }}
            strokeWidth="0.6"
          />
        </pattern>

        {/* Column rules: where the figures would go. Fainter than the writing
            lines, as on real ledger stock. */}
        <pattern
          id={columnsId}
          width={COLUMN_WIDTH}
          height={artHeight}
          patternUnits="userSpaceOnUse"
        >
          <path
            d={`M${COLUMN_WIDTH - 0.5} 0V${artHeight}`}
            style={{ stroke: "var(--stage-column)" }}
            strokeWidth="0.6"
          />
          <path
            d={`M${COLUMN_WIDTH * 0.62} 0V${artHeight}`}
            style={{ stroke: "var(--stage-column)" }}
            strokeOpacity="0.55"
            strokeWidth="0.5"
          />
        </pattern>

        {/*
         * Bookkeeping marks: reconciliation ticks, pencil dashes, and the double
         * rule ruled under a total. Abstract on purpose — no figures, nothing
         * that could be misread as real data.
         *
         * Density is halved on the large surfaces. The nav only ever shows a
         * fraction of one tile, so nine marks read as a few incidental pencil
         * notes there; a full-screen band shows twenty-odd tiles of the same
         * pattern, where the identical ticks stack into a lattice and start
         * competing with the content. Doubling the tile in both axes and
         * placing two diagonally-offset copies inside it keeps the same marks
         * at exactly half the marks-per-area, and the offset also breaks up the
         * grid the single tile produced.
         */}
        <pattern
          id={marksId}
          width={MARK_TILE * markSpread}
          height={STAGE_PAGE * markSpread}
          patternUnits="userSpaceOnUse"
        >
          {markOffsets.map(([dx, dy]) => (
            <g
              key={`${dx}-${dy}`}
              transform={`translate(${dx} ${dy})`}
              style={{ stroke: "var(--stage-ink)" }}
              strokeLinecap="round"
              strokeOpacity="0.5"
              strokeWidth="0.7"
            >
              <path d="M44 33.5l2.6 2.6L51 31" />
              <path d="M212 57.5l2.6 2.6L219 55" />
              <path d="M388 21.5l2.6 2.6L395 19" />
              <path d="M508 69.5l2.6 2.6L515 67" />
              <path d="M132 47h26" strokeDasharray="4 3" strokeOpacity="0.4" />
              <path d="M296 83h34" strokeDasharray="4 3" strokeOpacity="0.35" />
              <path d="M440 47h22" strokeDasharray="4 3" strokeOpacity="0.4" />
              <path d="M84 70h30M84 72h30" strokeOpacity="0.3" strokeWidth="0.5" />
              <path
                d="M340 34h26M340 36h26"
                strokeOpacity="0.28"
                strokeWidth="0.5"
              />
            </g>
          ))}
        </pattern>
      </defs>

      <rect width="100%" height={artHeight} fill={`url(#${paperId})`} />
      <rect width="100%" height={artHeight} fill={`url(#${glowsId})`} />
      <rect width="100%" height={artHeight} fill={`url(#${columnsId})`} />
      <rect width="100%" height={artHeight} fill={`url(#${rulesId})`} />
      <rect width="100%" height={artHeight} fill={`url(#${marksId})`} />
    </svg>
  );
}
