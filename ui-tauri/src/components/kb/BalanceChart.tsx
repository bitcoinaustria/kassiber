/**
 * Balance-over-time area chart.
 *
 * Hand-rolled SVG ported from claude-design/screens/overview.jsx. The
 * synthesized-jitter approach gives plausible variation across ranges
 * without needing more underlying data points; once the daemon
 * exposes a real `reports.balance-history` series we'll switch to
 * that and likely drop down to the shadcn `chart` (Recharts) primitive.
 *
 * Sizing: the chart fills its container in both axes via ResizeObserver
 * and matches its viewBox to the measured pixel dimensions, so font
 * sizes and stroke widths render at the expected size regardless of
 * viewport. Callers control size via the wrapping element's CSS.
 */

import { useElementSize } from "@/lib/useElementSize";
import type { Range } from "./RangeTabs";

interface BalanceChartProps {
  series: number[];
  ccy?: "btc" | "eur";
  priceEur?: number;
  range?: Range;
}

interface RangeConfig {
  n: number;
  labels: string[];
  start: number;
  jit: number;
}

const RANGE_CFG: Record<Range, RangeConfig> = {
  d:   { n: 24, labels: ["00", "04", "08", "12", "16", "20", "24"], start: 0.98, jit: 0.004 },
  w:   { n: 7,  labels: ["M", "T", "W", "T", "F", "S", "S"], start: 0.93, jit: 0.015 },
  m:   { n: 30, labels: ["1", "5", "10", "15", "20", "25", "30"], start: 0.80, jit: 0.02 },
  ytd: { n: 12, labels: ["J", "F", "M", "A", "M", "J", "J", "A", "S", "O", "N", "D"], start: 0.22, jit: 0 },
  "1y":{ n: 12, labels: ["M", "A", "M", "J", "J", "A", "S", "O", "N", "D", "J", "F"], start: 0.18, jit: 0 },
  "5y":{ n: 20, labels: ["2021", "2022", "2023", "2024", "2025"], start: 0.05, jit: 0.03 },
  all: { n: 24, labels: ["2019", "2020", "2021", "2022", "2023", "2024", "2025"], start: 0.0, jit: 0.03 },
};

export function BalanceChart({
  series,
  ccy = "btc",
  priceEur = 60000,
  range = "ytd",
}: BalanceChartProps) {
  const [containerRef, { width, height }] = useElementSize<HTMLDivElement>(
    520,
    240,
  );

  const cfg = RANGE_CFG[range];
  const end = series[series.length - 1];
  const startVal = end * cfg.start;
  const n = cfg.n;

  const rand = (i: number) => {
    const x =
      Math.sin((i + 1) * 9301 + (range.charCodeAt(0) || 0) * 97) *
      43758.5453;
    return x - Math.floor(x);
  };

  const arr = Array.from({ length: n }, (_, i) => {
    const t = i / (n - 1);
    const linear = startVal + (end - startVal) * t;
    const jitter = (rand(i) - 0.5) * cfg.jit * end;
    return Math.max(0, linear + jitter);
  });
  const s = ccy === "eur" ? arr.map((v) => v * priceEur) : arr;

  const pad = { t: 14, r: 14, b: 22, l: ccy === "eur" ? 48 : 36 };
  const max = Math.max(...s) * 1.15;
  const stepX = (width - pad.l - pad.r) / (s.length - 1);
  const yFor = (v: number) =>
    pad.t + (1 - v / max) * (height - pad.t - pad.b);

  const pts: Array<[number, number]> = s.map((v, i) => [
    pad.l + i * stepX,
    yFor(v),
  ]);
  const linePath = pts
    .map(
      (p, i) =>
        (i === 0 ? "M" : "L") + p[0].toFixed(1) + "," + p[1].toFixed(1),
    )
    .join(" ");
  const areaPath =
    linePath +
    ` L ${pts[pts.length - 1][0].toFixed(1)},${(height - pad.b).toFixed(1)}` +
    ` L ${pts[0][0].toFixed(1)},${(height - pad.b).toFixed(1)} Z`;
  const yTicks = [0, 0.25, 0.5, 0.75, 1];
  const showDotEvery = n <= 12 ? 1 : Math.ceil(n / 12);

  const fmtY = (v: number) =>
    ccy === "eur"
      ? "€" + Math.round(v).toLocaleString("de-AT")
      : v.toFixed(1);

  return (
    <div ref={containerRef} className="block h-full w-full">
      <svg
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        className="block"
      >
        {yTicks.map((tk, i) => {
          const yy = pad.t + tk * (height - pad.t - pad.b);
          return (
            <g key={i}>
              <line
                x1={pad.l}
                x2={width - pad.r}
                y1={yy}
                y2={yy}
                stroke="var(--color-line)"
                strokeDasharray={i === yTicks.length - 1 ? "" : "2 3"}
              />
              <text
                x={pad.l - 6}
                y={yy + 3}
                textAnchor="end"
                fontFamily="var(--font-mono)"
                fontSize="9"
                fill="var(--color-ink-3)"
              >
                {fmtY((1 - tk) * max)}
              </text>
            </g>
          );
        })}
        <path d={areaPath} fill="var(--color-accent)" fillOpacity="0.08" />
        <path
          d={linePath}
          stroke="var(--color-accent)"
          strokeWidth="1.5"
          fill="none"
        />
        {pts.map(
          (p, i) =>
            (i % showDotEvery === 0 || i === pts.length - 1) && (
              <circle
                key={i}
                cx={p[0]}
                cy={p[1]}
                r="2"
                fill="var(--color-paper-2)"
                stroke="var(--color-accent)"
                strokeWidth="1"
              />
            ),
        )}
        {cfg.labels.map((lbl, i) => {
          const frac =
            cfg.labels.length === 1 ? 0.5 : i / (cfg.labels.length - 1);
          const x = pad.l + frac * (width - pad.l - pad.r);
          return (
            <text
              key={i}
              x={x}
              y={height - 6}
              textAnchor="middle"
              fontFamily="var(--font-mono)"
              fontSize="9"
              fill="var(--color-ink-3)"
            >
              {lbl}
            </text>
          );
        })}
        <text
          x={pad.l - 28}
          y={pad.t + 4}
          fontFamily="var(--font-mono)"
          fontSize="9"
          fill="var(--color-ink-3)"
        >
          {ccy === "eur" ? "€" : "₿"}
        </text>
      </svg>
    </div>
  );
}
