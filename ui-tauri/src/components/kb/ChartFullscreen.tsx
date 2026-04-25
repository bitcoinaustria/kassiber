/**
 * ChartFullscreen — expanded balance chart modal.
 *
 * Translated from claude-design/screens/overview.jsx (`ChartFullscreen`).
 * Surfaced when the expand icon on the Balance & performance card is
 * clicked. Goes deeper than the inline `BalanceChart`:
 *   - Larger SVG with a hover scrubber: vertical guide + value readout
 *     following the cursor; floating tooltip near the cursor.
 *   - "By connection" mode: stacked composition area chart so the user
 *     can see where the balance is concentrated over time.
 *   - Range-aware KPIs (high, low, change, % change) and a legend.
 *
 * Mounted via the shared shadcn `Dialog` shell with the kassiber
 * hard-edge override, mirroring `SettingsModal` so the visual language
 * stays consistent.
 */
import * as React from "react";

import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { MOCK_OVERVIEW } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";

type Range = "d" | "w" | "m" | "ytd" | "1y" | "5y" | "all";
type Ccy = "btc" | "eur";

interface RangeConfig {
  n: number;
  start: number;
  jit: number;
  tickLabels: string[];
  scrubFmt: (i: number, n: number) => string;
}

const MONTHS_SHORT = [
  "Jan",
  "Feb",
  "Mar",
  "Apr",
  "May",
  "Jun",
  "Jul",
  "Aug",
  "Sep",
  "Oct",
  "Nov",
  "Dec",
] as const;

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;

const ONE_Y_LABELS = [
  "May 25",
  "Jun 25",
  "Jul 25",
  "Aug 25",
  "Sep 25",
  "Oct 25",
  "Nov 25",
  "Dec 25",
  "Jan 26",
  "Feb 26",
  "Mar 26",
  "Apr 26",
] as const;

const RANGE_CFG: Record<Range, RangeConfig> = {
  d: {
    n: 24,
    start: 0.98,
    jit: 0.004,
    tickLabels: ["00:00", "04:00", "08:00", "12:00", "16:00", "20:00", "24:00"],
    scrubFmt: (i, n) =>
      `${String(Math.round((i / (n - 1)) * 24)).padStart(2, "0")}:00`,
  },
  w: {
    n: 7,
    start: 0.93,
    jit: 0.015,
    tickLabels: [...WEEKDAYS],
    scrubFmt: (i) => WEEKDAYS[i] ?? "",
  },
  m: {
    n: 30,
    start: 0.8,
    jit: 0.02,
    tickLabels: ["1", "5", "10", "15", "20", "25", "30"],
    scrubFmt: (i) => `Apr ${i + 1}`,
  },
  ytd: {
    n: 12,
    start: 0.22,
    jit: 0,
    tickLabels: [...MONTHS_SHORT],
    scrubFmt: (i) => `${MONTHS_SHORT[i] ?? ""} 2026`,
  },
  "1y": {
    n: 12,
    start: 0.18,
    jit: 0,
    tickLabels: ["M", "A", "M", "J", "J", "A", "S", "O", "N", "D", "J", "F"],
    scrubFmt: (i) => ONE_Y_LABELS[i] ?? "",
  },
  "5y": {
    n: 20,
    start: 0.05,
    jit: 0.03,
    tickLabels: ["2021", "2022", "2023", "2024", "2025", "2026"],
    scrubFmt: (i) => `Q${(i % 4) + 1} · ${2021 + Math.floor(i / 4)}`,
  },
  all: {
    n: 24,
    start: 0.0,
    jit: 0.03,
    tickLabels: ["2019", "2020", "2021", "2022", "2023", "2024", "2025", "2026"],
    scrubFmt: (i, n) => `${2019 + Math.floor((i / (n - 1)) * 7)}`,
  },
};

const RANGE_BUTTONS: Array<[Range, string]> = [
  ["d", "D"],
  ["w", "W"],
  ["m", "M"],
  ["ytd", "YTD"],
  ["1y", "1Y"],
  ["5y", "5Y"],
  ["all", "ALL"],
];

const CCY_BUTTONS: Array<[Ccy, string]> = [
  ["btc", "₿"],
  ["eur", "€"],
];

interface ChartFullscreenProps {
  open: boolean;
  onClose: () => void;
  totalBtc: number;
}

interface HoverState {
  i: number;
  x: number;
  y: number;
  val: number;
  date: string;
}

export function ChartFullscreen({
  open,
  onClose,
  totalBtc,
}: ChartFullscreenProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const [range, setRange] = React.useState<Range>("ytd");
  const [stacked, setStacked] = React.useState(false);
  const [ccy, setCcy] = React.useState<Ccy>("btc");
  const [hover, setHover] = React.useState<HoverState | null>(null);

  // Reset hover whenever the modal opens; the dialog primitive already
  // wires Esc to close, so no extra keydown listener is needed.
  React.useEffect(() => {
    if (!open) setHover(null);
  }, [open]);

  // Replicate the per-range synthesis used by BalanceChart so deltas
  // line up between the inline chart and the fullscreen take-over.
  const cfg = RANGE_CFG[range];
  const end = totalBtc;
  const startVal = end * cfg.start;
  const n = cfg.n;
  const rand = (i: number, salt = 0) => {
    const x =
      Math.sin(
        (i + 1) * 9301 + (range.charCodeAt(0) || 0) * 97 + salt * 13,
      ) * 43758.5453;
    return x - Math.floor(x);
  };
  const totalSeries = Array.from({ length: n }, (_, i) => {
    const t = i / (n - 1);
    const linear = startVal + (end - startVal) * t;
    const jitter = (rand(i) - 0.5) * cfg.jit * end;
    return Math.max(0, linear + jitter);
  });

  // Per-connection composition. Use current % of total as the
  // steady-state mix and back-fade newer accounts so the stacked story
  // has movement.
  const conns = MOCK_OVERVIEW.connections;
  const mix = conns.map((c) => c.balance / totalBtc);
  const stack = conns.map((c, ci) => {
    const lateness =
      c.kind === "cashu" || c.kind === "nwc"
        ? 0.8
        : c.kind === "core-ln" || c.kind === "lnd"
          ? 0.4
          : 0.0;
    return totalSeries.map((tot, i) => {
      const t = i / (n - 1);
      const ramp =
        lateness === 0
          ? 1
          : Math.max(0, (t - lateness) / Math.max(0.0001, 1 - lateness));
      const j = (rand(i, ci + 1) - 0.5) * 0.06 * mix[ci];
      const share = Math.max(0, mix[ci] * ramp + j);
      return tot * share;
    });
  });
  // Normalize each timestep so stacked sums match totalSeries.
  for (let i = 0; i < n; i++) {
    const sum = stack.reduce((s, layer) => s + layer[i], 0);
    if (sum > 0) {
      const k = totalSeries[i] / sum;
      for (let ci = 0; ci < stack.length; ci++) stack[ci][i] *= k;
    }
  }

  // Range KPIs (in selected currency).
  const series =
    ccy === "eur"
      ? totalSeries.map((v) => v * MOCK_OVERVIEW.priceEur)
      : totalSeries;
  const max = Math.max(...series);
  const min = Math.min(...series);
  const startV = series[0];
  const endV = series[series.length - 1];
  const delta = endV - startV;
  const pct = startV > 0 ? (delta / startV) * 100 : 0;
  const up = delta >= 0;

  const fmt = (v: number) =>
    ccy === "eur"
      ? "€ " +
        v.toLocaleString("de-AT", {
          minimumFractionDigits: 0,
          maximumFractionDigits: 0,
        })
      : "₿ " + v.toFixed(v < 0.01 ? 8 : 4);

  const W = 1100;
  const H = 440;
  const pad = { t: 28, r: 24, b: 36, l: 80 };
  const yMax = max * 1.08;
  const yMin = 0;
  const stepX = (W - pad.l - pad.r) / (series.length - 1);
  const yToPx = (v: number) =>
    pad.t + (1 - (v - yMin) / (yMax - yMin)) * (H - pad.t - pad.b);
  const pts: Array<[number, number]> = series.map((v, i) => [
    pad.l + i * stepX,
    yToPx(v),
  ]);
  const linePath = pts
    .map((p, i) => (i === 0 ? "M" : "L") + p[0].toFixed(1) + "," + p[1].toFixed(1))
    .join(" ");
  const areaPath =
    linePath +
    ` L ${pts[pts.length - 1][0].toFixed(1)},${(H - pad.b).toFixed(1)}` +
    ` L ${pts[0][0].toFixed(1)},${(H - pad.b).toFixed(1)} Z`;

  // Stacked layers (cumulative from bottom).
  const stackLayers = (() => {
    const cum = Array.from({ length: n }, () => 0);
    return stack.map((layer, ci) => {
      const top = layer.map((v, i) => {
        cum[i] += v;
        return cum[i];
      });
      const bot = top.map((v, i) => v - layer[i]);
      const layerSeries =
        ccy === "eur"
          ? {
              top: top.map((v) => v * MOCK_OVERVIEW.priceEur),
              bot: bot.map((v) => v * MOCK_OVERVIEW.priceEur),
            }
          : { top, bot };
      const topPath = layerSeries.top
        .map(
          (v, i) =>
            (i === 0 ? "M" : "L") +
            (pad.l + i * stepX).toFixed(1) +
            "," +
            yToPx(v).toFixed(1),
        )
        .join(" ");
      const botRev = layerSeries.bot.slice().reverse();
      const botPath = botRev
        .map(
          (v, j) =>
            "L" +
            (pad.l + (n - 1 - j) * stepX).toFixed(1) +
            "," +
            yToPx(v).toFixed(1),
        )
        .join(" ");
      return { ci, conn: conns[ci], d: topPath + " " + botPath + " Z" };
    });
  })();

  // Connection layer colors — single accent + tonal grays so it stays
  // calm. Mirrors the source palette.
  const layerFill = (ci: number, alpha = 0.85) => {
    const palette = [
      `rgba(34,34,34,${alpha})`,
      `rgba(74,74,74,${alpha})`,
      `rgba(138,138,138,${alpha})`,
      `rgba(212,212,212,${alpha})`,
      `rgba(227,0,15,${Math.min(1, alpha)})`,
    ];
    return palette[ci % palette.length];
  };

  const yTicks = [0, 0.25, 0.5, 0.75, 1];

  // Mouse handler converts client x → series index, snaps to nearest point.
  const onMouseMove = (e: React.MouseEvent<SVGSVGElement>) => {
    const svg = e.currentTarget;
    const rect = svg.getBoundingClientRect();
    const xRatio = (e.clientX - rect.left) / rect.width;
    const xVb = xRatio * W;
    const i = Math.max(
      0,
      Math.min(n - 1, Math.round((xVb - pad.l) / stepX)),
    );
    setHover({
      i,
      x: pts[i][0],
      y: pts[i][1],
      val: series[i],
      date: cfg.scrubFmt(i, n),
    });
  };

  const blurCls = hideSensitive ? "sensitive" : "";

  const maxIdx = series.indexOf(max);
  const minIdx = series.indexOf(min);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent
        showCloseButton={false}
        className={cn(
          "flex h-[92vh] w-[96vw] max-w-[1400px] flex-col gap-0 overflow-hidden",
          "rounded-none border-ink bg-paper p-0 shadow-hard-ink",
          "data-[state=open]:zoom-in-100 data-[state=closed]:zoom-out-100",
        )}
      >
        {/* Header */}
        <DialogHeader className="flex h-13 flex-row items-center justify-between gap-2 border-b border-line px-4.5 py-0">
          <div className="flex min-w-0 items-baseline gap-3">
            <DialogTitle className="font-sans text-[17px] font-semibold tracking-[-0.005em] text-ink">
              Balance &amp; performance
            </DialogTitle>
            <span className="font-mono text-[9px] uppercase tracking-[0.16em] text-ink-3">
              fullscreen
            </span>
            <DialogDescription className="sr-only">
              Expanded view of balance over time, with hover scrubber and
              optional by-connection composition mode.
            </DialogDescription>
          </div>

          <div className="flex items-center gap-2.5">
            {/* range chips */}
            <div className="flex gap-0.5">
              {RANGE_BUTTONS.map(([k, lbl]) => {
                const active = range === k;
                return (
                  <button
                    key={k}
                    type="button"
                    onClick={() => {
                      setRange(k);
                      setHover(null);
                    }}
                    className={cn(
                      "cursor-pointer border px-2.5 py-1 font-mono text-[10px] font-semibold uppercase tracking-[0.08em]",
                      active
                        ? "border-ink bg-ink text-paper"
                        : "border-line bg-transparent text-ink-2",
                    )}
                  >
                    {lbl}
                  </button>
                );
              })}
            </div>

            {/* currency toggle */}
            <div className="flex gap-0.5">
              {CCY_BUTTONS.map(([k, lbl]) => {
                const active = ccy === k;
                return (
                  <button
                    key={k}
                    type="button"
                    onClick={() => setCcy(k)}
                    className={cn(
                      "cursor-pointer border px-2.5 py-1 font-mono text-[11px] font-semibold",
                      active
                        ? "border-ink bg-ink text-paper"
                        : "border-line bg-transparent text-ink-2",
                    )}
                  >
                    {lbl}
                  </button>
                );
              })}
            </div>

            {/* mode toggle */}
            <button
              type="button"
              onClick={() => setStacked((s) => !s)}
              className={cn(
                "cursor-pointer border px-2.5 py-1 font-mono text-[10px] font-semibold uppercase tracking-[0.08em]",
                stacked
                  ? "border-ink bg-ink text-paper"
                  : "border-line bg-transparent text-ink-2",
              )}
            >
              {stacked ? "By connection" : "Total"}
            </button>

            {/* close */}
            <button
              type="button"
              onClick={onClose}
              title="Close (Esc)"
              className="ml-1 flex size-7 cursor-pointer items-center justify-center border border-line bg-transparent p-0"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
                <path
                  d="M2 2 L10 10 M10 2 L2 10"
                  stroke="var(--color-ink)"
                  strokeWidth="1.4"
                  strokeLinecap="square"
                />
              </svg>
              <span className="sr-only">Close</span>
            </button>
          </div>
        </DialogHeader>

        {/* KPI strip */}
        <div className="grid grid-cols-[1.4fr_1fr_1fr_1fr_1fr] gap-4.5 border-b border-line bg-paper-2 px-4.5 py-3.5">
          <div>
            <div className="font-mono text-[9px] uppercase tracking-[0.14em] text-ink-3">
              {range.toUpperCase()} · current
            </div>
            <div
              className={cn(
                "mt-0.5 font-sans text-[30px] font-medium leading-[1.05] tracking-[-0.015em] text-ink",
                blurCls,
              )}
            >
              {fmt(endV)}
            </div>
          </div>

          <KpiBlock
            label="Change"
            value={
              <span className={cn(up ? "text-[#3fa66a]" : "text-accent", blurCls)}>
                {up ? "+ " : "− "}
                {fmt(Math.abs(delta))}
              </span>
            }
            sub={
              <span className={up ? "text-[#3fa66a]" : "text-accent"}>
                {up ? "+" : "−"} {Math.abs(pct).toFixed(2)} %
              </span>
            }
          />

          <KpiBlock
            label="High"
            value={<span className={blurCls}>{fmt(max)}</span>}
            sub={cfg.scrubFmt(maxIdx, n)}
          />

          <KpiBlock
            label="Low"
            value={<span className={blurCls}>{fmt(min)}</span>}
            sub={cfg.scrubFmt(minIdx, n)}
          />

          <KpiBlock
            label="Spot · BTC/EUR"
            value={
              "€ " +
              MOCK_OVERVIEW.priceEur.toLocaleString("de-AT", {
                minimumFractionDigits: 2,
                maximumFractionDigits: 2,
              })
            }
            sub={
              <span className="text-[#3fa66a]">+ 1.42 % · 24h</span>
            }
          />
        </div>

        {/* Chart */}
        <div className="flex min-h-0 flex-1 flex-col px-4.5 pt-3">
          <div className="relative min-h-0 flex-1">
            <svg
              viewBox={`0 0 ${W} ${H}`}
              preserveAspectRatio="none"
              className="block size-full cursor-crosshair"
              onMouseMove={onMouseMove}
              onMouseLeave={() => setHover(null)}
            >
              {/* y grid + labels */}
              {yTicks.map((tk, i) => {
                const yy = pad.t + tk * (H - pad.t - pad.b);
                const v = (1 - tk) * yMax;
                return (
                  <g key={i}>
                    <line
                      x1={pad.l}
                      x2={W - pad.r}
                      y1={yy}
                      y2={yy}
                      stroke="var(--color-line)"
                      strokeDasharray={i === yTicks.length - 1 ? "" : "2 3"}
                    />
                    <text
                      x={pad.l - 10}
                      y={yy + 4}
                      textAnchor="end"
                      fontFamily="var(--font-mono)"
                      fontSize="10"
                      fill="var(--color-ink-3)"
                    >
                      {ccy === "eur"
                        ? "€ " + Math.round(v).toLocaleString("de-AT")
                        : v.toFixed(v < 0.1 ? 4 : 2)}
                    </text>
                  </g>
                );
              })}

              {/* x labels */}
              {cfg.tickLabels.map((lbl, i) => {
                const frac =
                  cfg.tickLabels.length === 1
                    ? 0.5
                    : i / (cfg.tickLabels.length - 1);
                const x = pad.l + frac * (W - pad.l - pad.r);
                return (
                  <g key={i}>
                    <line
                      x1={x}
                      x2={x}
                      y1={H - pad.b}
                      y2={H - pad.b + 4}
                      stroke="var(--color-line-2)"
                    />
                    <text
                      x={x}
                      y={H - pad.b + 18}
                      textAnchor="middle"
                      fontFamily="var(--font-mono)"
                      fontSize="10"
                      fill="var(--color-ink-3)"
                    >
                      {lbl}
                    </text>
                  </g>
                );
              })}

              {/* Series */}
              {!stacked && (
                <>
                  <path
                    d={areaPath}
                    fill="var(--color-accent)"
                    fillOpacity="0.08"
                  />
                  <path
                    d={linePath}
                    stroke="var(--color-accent)"
                    strokeWidth="1.5"
                    fill="none"
                  />
                  {pts.map((p, i) => (
                    <circle
                      key={i}
                      cx={p[0]}
                      cy={p[1]}
                      r={n <= 12 ? 2.5 : 1.5}
                      fill="var(--color-paper)"
                      stroke="var(--color-accent)"
                      strokeWidth="1"
                    />
                  ))}
                </>
              )}
              {stacked && (
                <g className={blurCls}>
                  {stackLayers.map((l, idx) => (
                    <path
                      key={l.ci}
                      d={l.d}
                      fill={layerFill(idx, 0.78)}
                      stroke="var(--color-paper)"
                      strokeWidth="0.6"
                    />
                  ))}
                  {/* keep total line on top for reference */}
                  <path
                    d={linePath}
                    stroke="var(--color-ink)"
                    strokeWidth="1"
                    fill="none"
                    strokeOpacity="0.25"
                  />
                </g>
              )}

              {/* Hover crosshair */}
              {hover && (
                <g>
                  <line
                    x1={hover.x}
                    x2={hover.x}
                    y1={pad.t}
                    y2={H - pad.b}
                    stroke="var(--color-ink)"
                    strokeDasharray="2 3"
                    strokeWidth="1"
                  />
                  <circle
                    cx={hover.x}
                    cy={hover.y}
                    r="4"
                    fill="var(--color-paper)"
                    stroke="var(--color-ink)"
                    strokeWidth="1.4"
                  />
                </g>
              )}
            </svg>

            {/* Hover readout — absolute, snaps near the cursor */}
            {hover && (
              <div
                className="pointer-events-none absolute whitespace-nowrap bg-ink px-2.5 py-1.5 font-mono text-[10px] tracking-[0.04em] text-paper"
                style={{
                  left: `${(hover.x / W) * 100}%`,
                  top: 8,
                  transform:
                    hover.x > W * 0.7
                      ? "translateX(-100%) translateX(-12px)"
                      : "translateX(12px)",
                }}
              >
                <div className="text-[9px] uppercase tracking-[0.14em] text-paper/70">
                  {hover.date}
                </div>
                <div
                  className={cn(
                    "mt-px font-sans text-[13px] tracking-[-0.005em]",
                    blurCls,
                  )}
                >
                  {fmt(hover.val)}
                </div>
              </div>
            )}
          </div>

          {/* Legend / footnote */}
          <div className="mt-2 flex items-center justify-between border-t border-line py-2.5 pt-2.5">
            {stacked ? (
              <div className="flex flex-wrap gap-3.5">
                {conns.map((c, ci) => (
                  <div key={c.id} className="flex items-center gap-1.5">
                    <span
                      className="inline-block size-2.5"
                      style={{ background: layerFill(ci, 0.85) }}
                    />
                    <span className="font-sans text-xs text-ink">{c.label}</span>
                    <span
                      className={cn(
                        "font-mono text-[10px] text-ink-3",
                        blurCls,
                      )}
                    >
                      {((c.balance / totalBtc) * 100).toFixed(1)}%
                    </span>
                  </div>
                ))}
              </div>
            ) : (
              <div className="flex items-center gap-2">
                <span className="inline-block h-0.5 w-3.5 bg-accent" />
                <span className="font-sans text-xs text-ink">Total balance</span>
                <span className="ml-2 font-mono text-[10px] text-ink-3">
                  {n} points · synthesized for demo
                </span>
              </div>
            )}
            <span className="font-mono text-[9px] uppercase tracking-[0.12em] text-ink-3">
              hover for detail · esc to close
            </span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

interface KpiBlockProps {
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
}

function KpiBlock({ label, value, sub }: KpiBlockProps) {
  return (
    <div className="min-w-0">
      <div className="font-mono text-[9px] uppercase tracking-[0.14em] text-ink-3">
        {label}
      </div>
      <div className="mt-0.5 overflow-hidden text-ellipsis whitespace-nowrap font-sans text-[18px] font-medium leading-[1.1] tracking-[-0.01em] text-ink">
        {value}
      </div>
      {sub && (
        <div className="mt-0.5 font-mono text-[10px] text-ink-3">{sub}</div>
      )}
    </div>
  );
}
