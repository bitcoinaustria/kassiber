import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChevronDown, Network } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  formatPrivacyMsat as fmtMsat,
  privacyEvidenceTone as evidenceTone,
  privacySeverityTone,
  shortPrivacyId as shortId,
  type EvidenceLevel,
  type PrivacySeverity,
  type WalletPrivacyRow,
} from "@/lib/privacyMirror";
import {
  AIE_HEURISTIC_COVERAGE,
  GRADE_HEX,
  HEURISTIC_STATUS_HEX,
  SEVERITY_HEX,
  SEVERITY_ORDER,
  type HeuristicStatus,
  type PrivacyScoreModel,
  type ScoreFactor,
  type ScoreFinding,
} from "@/lib/privacyScore";
import { cn } from "@/lib/utils";

function usePrefersReducedMotion() {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return undefined;
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReduced(query.matches);
    update();
    query.addEventListener?.("change", update);
    return () => query.removeEventListener?.("change", update);
  }, []);
  return reduced;
}

// Counts up 0 -> target once, like am-i-exposed's score reveal.
function useCountUp(target: number, durationMs = 1000) {
  const reduced = usePrefersReducedMotion();
  const [value, setValue] = useState(0);
  const frame = useRef<number | null>(null);
  useEffect(() => {
    if (reduced) {
      setValue(target);
      return undefined;
    }
    let start: number | null = null;
    const step = (now: number) => {
      if (start === null) start = now;
      const progress = Math.min(1, (now - start) / durationMs);
      const eased = 1 - Math.pow(1 - progress, 3);
      setValue(Math.round(target * eased));
      if (progress < 1) frame.current = requestAnimationFrame(step);
    };
    frame.current = requestAnimationFrame(step);
    return () => {
      if (frame.current !== null) cancelAnimationFrame(frame.current);
    };
  }, [target, durationMs, reduced]);
  return value;
}

const GRADE_ZONES: Array<{ grade: keyof typeof GRADE_HEX; from: number }> = [
  { grade: "F", from: 0 },
  { grade: "D", from: 25 },
  { grade: "C", from: 50 },
  { grade: "B", from: 75 },
  { grade: "A+", from: 90 },
];

export function PrivacyScoreHero({ model }: { model: PrivacyScoreModel }) {
  const { t } = useTranslation("privacyMirror");
  const reduced = usePrefersReducedMotion();
  const shown = useCountUp(model.score);
  const [mounted, setMounted] = useState(false);
  useEffect(() => setMounted(true), []);
  const hex = GRADE_HEX[model.grade];

  return (
    <div className="flex flex-col items-center gap-4 rounded-md border bg-muted/10 p-6 text-center">
      <div className="flex items-baseline gap-3">
        <span
          className={cn(
            "font-mono text-6xl font-bold leading-none transition-all duration-500 lg:text-7xl",
            !reduced && !mounted && "scale-90 opacity-0",
          )}
          style={{ color: hex, textShadow: model.grade === "A+" ? `0 0 24px ${hex}66` : undefined }}
          data-testid="privacy-score-grade"
        >
          {model.grade}
        </span>
        <span className="font-mono text-3xl tabular-nums text-muted-foreground">
          {shown}
          <span className="text-lg">{t("score.of")}</span>
        </span>
      </div>
      <p className="text-sm font-medium" style={{ color: hex }}>
        {t(`gradeHint.${model.grade}`)}
      </p>

      {/* Horizontal grade bar with zone markers and a pointer at the score. */}
      <div className="w-full max-w-md">
        <div className="relative h-2.5 w-full overflow-hidden rounded-full">
          <div className="flex h-full w-full">
            {GRADE_ZONES.map((zone, index) => {
              const next = GRADE_ZONES[index + 1]?.from ?? 100;
              const width = next - zone.from;
              return (
                <span
                  key={zone.grade}
                  className="h-full"
                  style={{ width: `${width}%`, backgroundColor: `${GRADE_HEX[zone.grade]}` , opacity: 0.35 }}
                />
              );
            })}
          </div>
          <span
            className="absolute top-1/2 size-3 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-background shadow transition-all duration-1000"
            style={{ left: `${reduced ? model.score : shown}%`, backgroundColor: hex }}
            aria-hidden="true"
          />
        </div>
        <div className="mt-1 flex justify-between font-mono text-[10px] text-muted-foreground">
          {GRADE_ZONES.map((zone) => (
            <span key={zone.grade}>{zone.grade}</span>
          ))}
          <span>100</span>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">{t("score.local")}</p>
    </div>
  );
}

export function SeverityRing({
  census,
}: {
  census: Record<PrivacySeverity, number>;
}) {
  const { t } = useTranslation("privacyMirror");
  const total = SEVERITY_ORDER.reduce((sum, key) => sum + census[key], 0);
  const radius = 52;
  const stroke = 14;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;
  const segments = SEVERITY_ORDER.filter((key) => census[key] > 0).map((key) => {
    const fraction = total ? census[key] / total : 0;
    const seg = {
      key,
      dash: fraction * circumference,
      gap: circumference - fraction * circumference,
      rotation: (offset / circumference) * 360,
    };
    offset += fraction * circumference;
    return seg;
  });

  return (
    <div className="flex flex-col items-center gap-3 rounded-md border bg-background p-4">
      <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {t("score.census")}
      </p>
      <div className="relative">
        <svg width="140" height="140" viewBox="0 0 140 140" role="img" aria-label={t("score.census")}>
          <circle
            cx="70"
            cy="70"
            r={radius}
            fill="none"
            stroke="currentColor"
            strokeWidth={stroke}
            className="text-muted/40"
          />
          {total === 0 ? (
            <circle cx="70" cy="70" r={radius} fill="none" stroke={GRADE_HEX["A+"]} strokeWidth={stroke} />
          ) : (
            segments.map((seg) => (
              <circle
                key={seg.key}
                cx="70"
                cy="70"
                r={radius}
                fill="none"
                stroke={SEVERITY_HEX[seg.key]}
                strokeWidth={stroke}
                strokeDasharray={`${seg.dash} ${seg.gap}`}
                strokeDashoffset={0}
                transform={`rotate(${seg.rotation - 90} 70 70)`}
                strokeLinecap="butt"
              />
            ))
          )}
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <span className="font-mono text-2xl font-bold tabular-nums">{total}</span>
          <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
            {t("score.findings")}
          </span>
        </div>
      </div>
      <div className="flex flex-wrap justify-center gap-x-4 gap-y-1 font-mono text-[11px]">
        {SEVERITY_ORDER.map((key) => (
          <span key={key} className="flex items-center gap-1.5">
            <span className="size-2 rounded-sm" style={{ backgroundColor: SEVERITY_HEX[key] }} />
            <span className="text-muted-foreground">{census[key]}</span>
            <span>{t(`severity.${key}`)}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

function factorDetail(factor: ScoreFactor) {
  if (factor.key === "wallet_linkage") return `${factor.linked ?? 0}/${factor.total ?? 0}`;
  if (factor.key === "transaction_leaks") return `${factor.leaking ?? 0}/${factor.total ?? 0}`;
  return factor.total != null ? String(factor.total) : "";
}

export function ScoreWaterfall({
  factors,
  score,
  base,
  coverageRatio,
}: {
  factors: ScoreFactor[];
  score: number;
  base: number;
  coverageRatio?: number;
}) {
  const { t } = useTranslation("privacyMirror");
  return (
    <div className="rounded-md border bg-background p-4">
      <p className="mb-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {t("score.waterfall")}
      </p>
      <div className="grid gap-2 text-sm">
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground">{t("score.base")}</span>
          <span className="font-mono tabular-nums">{base}</span>
        </div>
        {factors.map((factor) => (
          <div key={factor.key} className="flex items-center justify-between gap-2">
            <span className="flex min-w-0 items-center gap-2 truncate">
              <span>{t(`score.factor.${factor.key}`, { defaultValue: factor.key })}</span>
              <span className="font-mono text-xs text-muted-foreground">{factorDetail(factor)}</span>
            </span>
            <span
              className="font-mono tabular-nums"
              style={{ color: factor.points < 0 ? SEVERITY_HEX.warning : undefined }}
            >
              {factor.points > 0 ? `+${factor.points}` : factor.points}
            </span>
          </div>
        ))}
        <div className="mt-1 flex items-center justify-between gap-2 border-t pt-2 font-medium">
          <span>{t("score.result")}</span>
          <span className="font-mono tabular-nums">{score}</span>
        </div>
        {typeof coverageRatio === "number" ? (
          <div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
            <span>{t("score.coverage")}</span>
            <span className="font-mono tabular-nums">{Math.round(coverageRatio * 100)}%</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function tellLabel(kind: string | undefined, t: ReturnType<typeof useTranslation<"privacyMirror">>["t"]) {
  if (!kind) return "-";
  return t(`tellKind.${kind}`, { defaultValue: kind.replace(/_/g, " ") });
}

export function PrivacyFindingCard({
  finding,
  onViewFlow,
  action,
}: {
  finding: ScoreFinding;
  onViewFlow?: (txid: string) => void;
  action?: { label: string; icon?: ReactNode; onClick: () => void };
}) {
  const { t } = useTranslation("privacyMirror");
  const tone = privacySeverityTone(finding.severity);
  const reco = t(`reco.${finding.kind}`, { defaultValue: t("reco.fallback") });
  return (
    <Collapsible className={cn("rounded-md border border-l-2 bg-background", tone.stripe)}>
      <CollapsibleTrigger className="group flex w-full items-center justify-between gap-3 px-4 py-3 text-left hover:bg-muted/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <span className="flex min-w-0 items-center gap-2.5">
          <span className={cn("inline-block size-2.5 shrink-0 rounded-full", tone.dot)} aria-hidden="true" />
          <span className="min-w-0">
            <span className="block truncate text-sm font-medium">{tellLabel(finding.kind, t)}</span>
            <span className={cn("font-mono text-[10px] font-semibold uppercase tracking-wide", tone.text)}>
              {t(`severity.${finding.severity}`)}
            </span>
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-2">
          <Badge variant="outline" className={cn("rounded-md", evidenceTone(finding.evidenceLevel))}>
            {t(`evidence.${finding.evidenceLevel === "exact" || finding.evidenceLevel === "derived" ? finding.evidenceLevel : "unknown"}`)}
          </Badge>
          <ChevronDown className="size-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" aria-hidden="true" />
        </span>
      </CollapsibleTrigger>
      <CollapsibleContent className="border-t px-4 py-3">
        <p className="text-sm text-muted-foreground">{reco}</p>
        {(finding.txid && onViewFlow) || action ? (
          <div className="mt-3 flex flex-wrap gap-2">
            {finding.txid && onViewFlow ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-7 gap-1 px-2 text-xs"
                onClick={() => onViewFlow(finding.txid ?? "")}
              >
                <Network className="size-3.5" aria-hidden="true" />
                {t("flow.view")}
              </Button>
            ) : null}
            {action ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="h-7 gap-1 px-2 text-xs"
                onClick={action.onClick}
              >
                {action.icon}
                {action.label}
              </Button>
            ) : null}
          </div>
        ) : null}
      </CollapsibleContent>
    </Collapsible>
  );
}

export function HeuristicCoverage() {
  const { t } = useTranslation("privacyMirror");
  const order: HeuristicStatus[] = ["computed", "partial", "not_local"];
  const counts = AIE_HEURISTIC_COVERAGE.reduce(
    (acc, h) => ({ ...acc, [h.status]: (acc[h.status] ?? 0) + 1 }),
    {} as Record<HeuristicStatus, number>,
  );
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">{t("heuristics.note")}</p>
      <div className="flex flex-wrap gap-1.5">
        {AIE_HEURISTIC_COVERAGE.map((h) => (
          <span
            key={h.id}
            className="inline-flex items-center gap-1.5 rounded-md border bg-muted/20 px-2 py-1 text-xs"
            title={t(`heuristics.status.${h.status}`)}
          >
            <span
              className="size-2 shrink-0 rounded-full"
              style={{ backgroundColor: HEURISTIC_STATUS_HEX[h.status] }}
              aria-hidden="true"
            />
            {h.name}
          </span>
        ))}
      </div>
      <div className="flex flex-wrap gap-x-4 gap-y-1 font-mono text-[11px] text-muted-foreground">
        {order.map((status) => (
          <span key={status} className="flex items-center gap-1.5">
            <span
              className="size-2 rounded-full"
              style={{ backgroundColor: HEURISTIC_STATUS_HEX[status] }}
              aria-hidden="true"
            />
            {counts[status] ?? 0} {t(`heuristics.status.${status}`)}
          </span>
        ))}
      </div>
    </div>
  );
}

// A schematic linkage map: wallets that carry linkage edges are drawn linked to
// a passive-observer node; isolated wallets stand alone. Edge thickness tracks
// the wallet's linkage_edge_count. Derived purely from wallet_view.
export function LinkageGraph({
  wallets,
  evidenceLevel,
}: {
  wallets: WalletPrivacyRow[];
  evidenceLevel?: EvidenceLevel;
}) {
  const { t } = useTranslation("privacyMirror");
  const rows = wallets.slice(0, 8);
  if (!rows.length) {
    return <p className="text-sm text-muted-foreground">{t("linkage.empty")}</p>;
  }
  const rowHeight = 46;
  const height = Math.max(rows.length * rowHeight + 20, 120);
  const width = 480;
  const leftX = 150;
  const observerX = width - 120;
  const observerY = height / 2;
  const maxEdges = Math.max(1, ...rows.map((row) => row.linkage_edge_count ?? 0));
  const linkedCount = rows.filter((row) => (row.linkage_edge_count ?? 0) > 0).length;

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto">
        <svg
          viewBox={`0 0 ${width} ${height}`}
          width="100%"
          style={{ maxWidth: width }}
          role="img"
          aria-label={t("linkage.title")}
          className="min-w-[420px]"
        >
          {rows.map((row, index) => {
            const y = 22 + index * rowHeight;
            const edges = row.linkage_edge_count ?? 0;
            const linked = edges > 0;
            return (
              <line
                key={`edge-${row.wallet_id ?? index}`}
                x1={leftX}
                y1={y}
                x2={observerX}
                y2={observerY}
                stroke={linked ? SEVERITY_HEX.warning : "transparent"}
                strokeWidth={linked ? 1.5 + (edges / maxEdges) * 4 : 0}
                strokeOpacity={0.55}
              />
            );
          })}
          {linkedCount > 0 ? (
            <>
              <circle cx={observerX} cy={observerY} r={26} fill={`${SEVERITY_HEX.warning}22`} stroke={SEVERITY_HEX.warning} strokeWidth={1.5} />
              <text x={observerX} y={observerY + 40} textAnchor="middle" className="fill-muted-foreground text-[10px]">
                {t("linkage.observer")}
              </text>
            </>
          ) : null}
          {rows.map((row, index) => {
            const y = 22 + index * rowHeight;
            const linked = (row.linkage_edge_count ?? 0) > 0;
            const color = linked ? SEVERITY_HEX.warning : "#22c55e";
            return (
              <g key={`node-${row.wallet_id ?? index}`}>
                <rect x={8} y={y - 16} width={leftX - 8} height={32} rx={6} fill={`${color}18`} stroke={color} strokeWidth={1} />
                <text x={16} y={y - 2} className="fill-foreground text-[11px] font-medium">
                  {shortId(row.wallet_id)}
                </text>
                <text x={16} y={y + 11} className="fill-muted-foreground text-[10px]">
                  {t("linkage.edges", { count: row.linkage_edge_count ?? 0 })} · {fmtMsat(row.amount_msat)}
                </text>
              </g>
            );
          })}
        </svg>
      </div>
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <span className="size-2 rounded-sm" style={{ backgroundColor: SEVERITY_HEX.warning }} />
        {t("linkage.legendLinked")}
        <span className="ml-2 size-2 rounded-sm" style={{ backgroundColor: "#22c55e" }} />
        {t("linkage.legendIsolated")}
        {evidenceLevel ? (
          <Badge variant="outline" className={cn("ml-auto rounded-md", evidenceTone(evidenceLevel))}>
            {t(`evidence.${evidenceLevel === "exact" || evidenceLevel === "derived" ? evidenceLevel : "unknown"}`)}
          </Badge>
        ) : null}
      </p>
    </div>
  );
}
