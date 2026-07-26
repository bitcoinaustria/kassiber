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

// A wide viewBox keeps the 96-unit art height at a fixed scale while the nav
// resizes, so the rules never stretch (T3Code's trick, and its dimensions).
const STAGE_VIEW_BOX = "0 0 8192 96";

/** Ledger geometry, in viewBox units. */
const RULE_HEIGHT = 12;
const COLUMN_WIDTH = 96;

export function SidebarStageBackdrop() {
  return (
    <div
      aria-hidden="true"
      className="kb-stage-backdrop pointer-events-none absolute inset-x-0 top-0 z-0 h-14 overflow-hidden select-none"
    >
      <LedgerPaperArt />
    </div>
  );
}

function LedgerPaperArt() {
  // `useId` output can contain ":" which is invalid inside a url(#…) reference.
  const idPrefix = React.useId().replace(/:/g, "");
  const paperId = `${idPrefix}-paper`;
  const rulesId = `${idPrefix}-rules`;
  const columnsId = `${idPrefix}-columns`;
  const marksId = `${idPrefix}-marks`;
  const glowId = `${idPrefix}-glow`;
  const glowsId = `${idPrefix}-glows`;

  return (
    <svg
      className="kb-stage-ledger h-full w-full"
      fill="none"
      preserveAspectRatio="xMinYMin slice"
      viewBox={STAGE_VIEW_BOX}
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
          y2="96"
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
        <pattern id={glowsId} width="704" height="96" patternUnits="userSpaceOnUse">
          <rect width="704" height="96" fill={`url(#${glowId})`} />
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
          height="96"
          patternUnits="userSpaceOnUse"
        >
          <path
            d={`M${COLUMN_WIDTH - 0.5} 0V96`}
            style={{ stroke: "var(--stage-column)" }}
            strokeWidth="0.6"
          />
          <path
            d={`M${COLUMN_WIDTH * 0.62} 0V96`}
            style={{ stroke: "var(--stage-column)" }}
            strokeOpacity="0.55"
            strokeWidth="0.5"
          />
        </pattern>

        {/* Bookkeeping marks: reconciliation ticks, pencil dashes, and the
            double rule ruled under a total. Abstract on purpose — no figures,
            nothing that could be misread as real data. */}
        <pattern id={marksId} width="576" height="96" patternUnits="userSpaceOnUse">
          <g
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
            <path d="M340 34h26M340 36h26" strokeOpacity="0.28" strokeWidth="0.5" />
          </g>
        </pattern>
      </defs>

      <rect width="100%" height="96" fill={`url(#${paperId})`} />
      <rect width="100%" height="96" fill={`url(#${glowsId})`} />
      <rect width="100%" height="96" fill={`url(#${columnsId})`} />
      <rect width="100%" height="96" fill={`url(#${rulesId})`} />
      <rect width="100%" height="96" fill={`url(#${marksId})`} />
    </svg>
  );
}
