import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  RefreshCw,
  ShieldAlert,
  Wallet,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  LabelList,
  ReferenceLine,
  Tooltip,
  usePlotArea,
  useXAxisScale,
  XAxis,
  YAxis,
} from "recharts";

import { ChartContainer } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { formatBtc, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import {
  blurClass,
  compactCurrencyFormatter,
  currencyFormatter,
  formatDisplayMoney,
  transactionFlow,
  type Transaction,
  type TransactionFlow,
} from "@/components/transactions";
import {
  buildFlowChartRows,
  buildSwapCandidates,
  buildTransferCandidates,
  flowAxisDomain,
  flowBucketLabel,
  flowChartConfig,
  flowChartMetricLabels,
  flowChartModeLabels,
  flowChartSegmentForFlow,
  flowChartSegmentFromDataKey,
  flowChartSegmentLabels,
  flowColorForSegment,
  flowColors,
  flowPointSegmentValue,
  flowPointTotal,
  formatCountBarLabel,
  formatFlowTooltipValue,
  periodKeys,
  periodLabels,
  sumByFlow,
  type BreakdownSelection,
  type FlowChartClickData,
  type FlowChartMetric,
  type FlowChartMode,
  type FlowChartPoint,
  type FlowChartSegment,
  type FlowChartSelection,
  type PeriodKey,
  type ResolvedPeriodKey,
  type SwapCandidateReference,
  type TableQuickFilter,
} from "./model";

function formatMetricValue(
  eur: number,
  btc: number,
  currency: Currency,
  hideSensitive: boolean,
) {
  return (
    <CurrencyToggleText className={blurClass(hideSensitive)}>
      {formatDisplayMoney(eur, btc, currency)}
    </CurrencyToggleText>
  );
}

const PeriodTabs = ({
  activePeriod,
  onPeriodChange,
  periodOptions = periodKeys,
  resolvedPeriod,
}: {
  activePeriod: PeriodKey;
  onPeriodChange: (period: PeriodKey) => void;
  periodOptions?: PeriodKey[];
  resolvedPeriod?: ResolvedPeriodKey | null;
}) => {
  const { t } = useTranslation("transactions");
  const translatePeriod = (key: PeriodKey) =>
    (t as (key: string) => string)(periodLabels[key]);
  return (
    <div className="flex h-8 items-center gap-1 rounded-lg bg-muted p-0.5">
      {periodOptions.map((key) => (
        <button
          key={key}
          type="button"
          onClick={() => onPeriodChange(key)}
          className={cn(
            "h-7 rounded-md px-2.5 text-xs font-medium transition-all sm:px-3 sm:text-sm",
            activePeriod === key
              ? "bg-card text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {key === "auto" && activePeriod === "auto" && resolvedPeriod
            ? t("period.autoResolved", {
                period: translatePeriod(resolvedPeriod),
              })
            : translatePeriod(key)}
        </button>
      ))}
    </div>
  );
};

interface ChartTooltipPayload {
  dataKey?: string | number;
  value?: number | string;
  payload?: FlowChartPoint;
}

interface ChartTooltipProps {
  active?: boolean;
  payload?: ChartTooltipPayload[];
  label?: string | number;
  hideSensitive: boolean;
  currency: Currency;
  metric: FlowChartMetric;
}

function FlowBucketClickAreas({
  rows,
  onBucketClick,
}: {
  rows: FlowChartPoint[];
  onBucketClick: (point: FlowChartPoint) => void;
}) {
  const plotArea = usePlotArea();
  const xScale = useXAxisScale();
  if (!plotArea || !xScale || rows.length === 0) return null;

  const fallbackWidth = plotArea.width / rows.length;

  return (
    <g aria-hidden="true">
      {rows.map((row) => {
        const start = xScale(row.date, { position: "start" });
        const end = xScale(row.date, { position: "end" });
        const x = start ?? xScale(row.date);
        if (x === undefined || !Number.isFinite(x)) return null;
        const width =
          end !== undefined && Number.isFinite(end)
            ? Math.max(1, end - x)
            : fallbackWidth;
        return (
          <rect
            key={`bucket-click-${row.bucketKey}`}
            x={x}
            y={plotArea.y}
            width={width}
            height={plotArea.height}
            fill="transparent"
            cursor="pointer"
            onClick={() => onBucketClick(row)}
          />
        );
      })}
    </g>
  );
}

const TransactionWorkbench = ({
  period,
  records,
  hideSensitive,
  currency,
  onFlowSelectionChange,
  onQuickFilterChange,
  onBreakdownSelectionChange,
  onTableFiltersReset,
  chartSelection,
  swapCandidateRefs,
  swapCandidateTotal,
  isRefreshing,
}: {
  period: ResolvedPeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  onFlowSelectionChange: (selection: FlowChartSelection | null) => void;
  onQuickFilterChange: (filter: TableQuickFilter | null) => void;
  onBreakdownSelectionChange: (selection: BreakdownSelection | null) => void;
  onTableFiltersReset: () => void;
  chartSelection: FlowChartSelection | null;
  swapCandidateRefs?: SwapCandidateReference[];
  swapCandidateTotal?: number | null;
  isRefreshing?: boolean;
}) => {
  const { t } = useTranslation("transactions");
  const navigate = useNavigate();
  const [chartMetric, setChartMetric] =
    React.useState<FlowChartMetric>("amount");
  const [chartMode, setChartMode] = React.useState<FlowChartMode>("all");
  const swapCandidates = React.useMemo(
    () => buildSwapCandidates(records, swapCandidateRefs),
    [records, swapCandidateRefs],
  );
  const transferCandidates = React.useMemo(
    () => buildTransferCandidates(records, swapCandidateRefs),
    [records, swapCandidateRefs],
  );
  const swapCandidateIds = React.useMemo(
    () =>
      new Set(
        swapCandidates.flatMap((candidate) => [
          candidate.in.id,
          candidate.out.id,
        ]),
      ),
    [swapCandidates],
  );
  const transferCandidateIds = React.useMemo(
    () =>
      new Set(
        transferCandidates.flatMap((candidate) => [
          candidate.in.id,
          candidate.out.id,
        ]),
      ),
    [transferCandidates],
  );
  const pairingCandidateIds = React.useMemo(
    () => new Set([...swapCandidateIds, ...transferCandidateIds]),
    [swapCandidateIds, transferCandidateIds],
  );
  const externalRecords = records.filter((txn) => !pairingCandidateIds.has(txn.id));
  const incoming = sumByFlow(externalRecords, "incoming");
  const outgoing = sumByFlow(externalRecords, "outgoing");
  const baseTransfers = sumByFlow(externalRecords, "transfer");
  const markedSwaps = sumByFlow(records, "swap");
  const transferCandidateTotals = transferCandidates.reduce(
    (sum, candidate) => ({
      count: sum.count + 1,
      eur: sum.eur + (candidate.eur ?? 0),
      btc: sum.btc + candidate.btc,
    }),
    { count: 0, eur: 0, btc: 0 },
  );
  const transfers = {
    count: baseTransfers.count + transferCandidateTotals.count,
    eur: baseTransfers.eur + transferCandidateTotals.eur,
    btc: baseTransfers.btc + transferCandidateTotals.btc,
  };
  const swapCandidateTotals = swapCandidates.reduce(
    (sum, candidate) => ({
      count: sum.count + 1,
      eur: sum.eur + (candidate.eur ?? 0),
      btc: sum.btc + candidate.btc,
    }),
    { count: 0, eur: 0, btc: 0 },
  );
  const knownSwapCandidateCount =
    swapCandidateTotal === undefined
      ? swapCandidateTotals.count
      : swapCandidateTotal;
  const swapCandidateCountForTotal = knownSwapCandidateCount ?? 0;
  const swaps = {
    count: markedSwaps.count + swapCandidateCountForTotal,
    eur: markedSwaps.eur + swapCandidateTotals.eur,
    btc: markedSwaps.btc + swapCandidateTotals.btc,
  };
  const netEur = incoming.eur - outgoing.eur;
  const netBtc = incoming.btc - outgoing.btc;
  const reviewCount = records.filter((txn) => txn.status === "review").length;
  const pendingCount = records.filter((txn) => txn.status === "pending").length;
  const failedCount = records.filter((txn) => txn.status === "failed").length;
  const chartRecords =
    chartMode === "external"
      ? externalRecords.filter(
          (txn) =>
            !pairingCandidateIds.has(txn.id) &&
            ["incoming", "outgoing"].includes(transactionFlow(txn)),
        )
      : records;
  const candidateFlowOverrides = React.useMemo(() => {
    const next = new Map<string, TransactionFlow>();
    for (const id of swapCandidateIds) next.set(id, "swap");
    for (const id of transferCandidateIds) next.set(id, "layer-transition");
    return next;
  }, [swapCandidateIds, transferCandidateIds]);
  const chartRows = buildFlowChartRows(
    chartRecords,
    period,
    currency,
    candidateFlowOverrides,
    chartMetric,
  );
  const activeChartRows = chartRows.filter((row) => flowPointTotal(row) > 0);
  const visibleChartRows = activeChartRows.length ? activeChartRows : chartRows;
  const yDomain = flowAxisDomain(visibleChartRows, chartMetric);
  const bucketTitleSuffix = (
    {
      day: "Day",
      week: "Week",
      month: "Month",
      quarter: "Quarter",
    } as const
  )[flowBucketLabel(period)];
  const flowChartCellProps = React.useCallback(
    (row: FlowChartPoint, segment: FlowChartSegment) => {
      const sameBucket =
        chartSelection?.bucketKey === null ||
        chartSelection?.bucketKey === row.bucketKey;
      const selected = Boolean(
        chartSelection &&
          sameBucket &&
          (chartSelection.segment === null ||
            chartSelection.segment === segment),
      );
      const dimmed = Boolean(chartSelection && !selected);
      return {
        fillOpacity: dimmed ? 0.32 : 1,
        stroke: selected ? "var(--foreground)" : "transparent",
        strokeWidth: selected ? 1.5 : 0,
      };
    },
    [chartSelection],
  );
  const selectFlowBucket = React.useCallback(
    (point: FlowChartPoint) => {
      if (flowPointTotal(point) === 0) return;
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      onFlowSelectionChange({
        id: `${period}:${point.bucketKey}:all:${chartMode}`,
        period,
        bucketKey: point.bucketKey,
        bucketLabel: point.date,
        segment: null,
        mode: chartMode,
      });
    },
    [
      chartMode,
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
      period,
    ],
  );
  const handleFlowChartClick = React.useCallback(
    (data: FlowChartClickData, segment: FlowChartSegment) => {
      const point = data.payload ?? data.activePayload?.[0]?.payload;
      if (!point || flowPointSegmentValue(point, segment) === 0) return;
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      onFlowSelectionChange({
        id: `${period}:${point.bucketKey}:${segment}:${chartMode}`,
        period,
        bucketKey: point.bucketKey,
        bucketLabel: point.date,
        segment,
        mode: chartMode,
      });
    },
    [
      chartMode,
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
      period,
    ],
  );
  const handleFlowLegendClick = React.useCallback(
    (segment: FlowChartSegment) => {
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      onFlowSelectionChange({
        id: `${period}:all:${segment}:${chartMode}`,
        period,
        bucketKey: null,
        // loose translator
        bucketLabel: (t as (key: string) => string)(periodLabels[period]),
        segment,
        mode: chartMode,
      });
    },
    [
      chartMode,
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
      period,
      t,
    ],
  );
  const handleSummaryFlowClick = React.useCallback(
    (segment: FlowChartSegment) => {
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      onFlowSelectionChange({
        id: `${period}:summary:${segment}:all`,
        period,
        bucketKey: null,
        // loose translator
        bucketLabel: (t as (key: string) => string)(periodLabels[period]),
        segment,
        mode: "all",
      });
    },
    [
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
      period,
      t,
    ],
  );
  const handleNetFlowClick = React.useCallback(() => {
    onFlowSelectionChange(null);
    onBreakdownSelectionChange(null);
    onTableFiltersReset();
    onQuickFilterChange("external_flow");
  }, [
    onBreakdownSelectionChange,
    onFlowSelectionChange,
    onQuickFilterChange,
    onTableFiltersReset,
  ]);
  const handleReviewQueueClick = React.useCallback(() => {
    onFlowSelectionChange(null);
    onBreakdownSelectionChange(null);
    onTableFiltersReset();
    onQuickFilterChange("review_queue");
    void navigate({ to: "/quarantine" });
  }, [
    navigate,
    onBreakdownSelectionChange,
    onFlowSelectionChange,
    onQuickFilterChange,
    onTableFiltersReset,
  ]);
  const openSwapWorkflow = React.useCallback(() => {
    void navigate({ to: "/swaps" });
  }, [navigate]);
  const handleSwapWorkflowClick = React.useCallback(
    () => {
      onFlowSelectionChange(null);
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      if (knownSwapCandidateCount === null || knownSwapCandidateCount > 0) {
        openSwapWorkflow();
        return;
      }
      handleSummaryFlowClick("swaps");
    },
    [
      handleSummaryFlowClick,
      knownSwapCandidateCount,
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
      openSwapWorkflow,
    ],
  );
  const metricCards = [
    {
      label: t("workbench.metric.incoming"),
      value: incoming,
      meta: t("workbench.meta.txCount", { count: incoming.count }),
      icon: ArrowDownRight,
      tone: "text-emerald-600",
      onClick:
        incoming.count > 0
          ? () => handleSummaryFlowClick("incoming")
          : undefined,
      ariaLabel: t("workbench.aria.showIncoming"),
    },
    {
      label: t("workbench.metric.outgoing"),
      value: outgoing,
      meta: t("workbench.meta.txCount", { count: outgoing.count }),
      icon: ArrowUpRight,
      tone: "text-red-600",
      onClick:
        outgoing.count > 0
          ? () => handleSummaryFlowClick("outgoing")
          : undefined,
      ariaLabel: t("workbench.aria.showOutgoing"),
    },
    {
      label: t("workbench.metric.netFlow"),
      value: { eur: netEur, btc: netBtc },
      meta: netEur >= 0 ? t("workbench.meta.inflow") : t("workbench.meta.outflow"),
      icon: ArrowLeftRight,
      tone: netEur >= 0 ? "text-emerald-600" : "text-red-600",
      onClick:
        incoming.count + outgoing.count > 0
          ? handleNetFlowClick
          : undefined,
      ariaLabel: t("workbench.aria.showNetFlow"),
    },
    {
      label: t("workbench.metric.transfers"),
      value: transfers,
      meta: t("workbench.meta.moveCount", { count: transfers.count }),
      icon: Wallet,
      tone: "text-muted-foreground",
      onClick: () => handleSummaryFlowClick("transfers"),
      ariaLabel: t("workbench.aria.showTransfers"),
    },
    {
      label: t("workbench.metric.swaps"),
      value: swaps,
      meta:
        knownSwapCandidateCount === null
          ? t("workbench.meta.unpairedUnknown")
          : knownSwapCandidateCount > 0 && markedSwaps.count > 0
          ? t("workbench.meta.unpairedAndPaired", {
              unpaired: knownSwapCandidateCount,
              paired: markedSwaps.count,
            })
          : knownSwapCandidateCount > 0
          ? t("workbench.meta.unpaired", { count: knownSwapCandidateCount })
          : t("workbench.meta.paired", { count: markedSwaps.count }),
      icon: RefreshCw,
      tone:
        knownSwapCandidateCount === null || knownSwapCandidateCount > 0
          ? "text-amber-600"
          : "text-muted-foreground",
      onClick: handleSwapWorkflowClick,
      ariaLabel:
        knownSwapCandidateCount === null || knownSwapCandidateCount > 0
          ? t("workbench.aria.openPairingCandidates")
          : t("workbench.aria.showPairedSwaps"),
    },
    {
      label: t("workbench.metric.reviewQueue"),
      value: { eur: reviewCount + pendingCount + failedCount, btc: 0 },
      meta: t("workbench.meta.reviewAndPending", {
        review: reviewCount,
        pending: pendingCount,
      }),
      icon: ShieldAlert,
      tone:
        reviewCount || pendingCount || failedCount
          ? "text-amber-600"
          : "text-emerald-600",
      countOnly: true,
      onClick: handleReviewQueueClick,
      ariaLabel: t("workbench.aria.showReviewQueue"),
    },
  ];

  return (
    <>
      <section
        className="relative z-20 grid grid-cols-2 overflow-visible rounded-xl border bg-card md:grid-cols-3 xl:grid-cols-6"
        role={isRefreshing ? "status" : undefined}
        aria-live={isRefreshing ? "polite" : undefined}
      >
        {metricCards.map((metric, index) => {
          const Icon = metric.icon;
          const className = cn(
            "min-w-0 space-y-2 border-b p-3 text-left sm:p-4",
            index % 2 === 1 && "border-l",
            index % 3 === 0 ? "md:border-l-0" : "md:border-l",
            index > 0 ? "xl:border-l" : "xl:border-l-0",
            metric.onClick &&
              !isRefreshing &&
              "relative isolate w-full cursor-pointer overflow-hidden transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/60 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:before:scale-x-100 [&>*]:relative [&>*]:z-10",
          );
          const content = isRefreshing ? (
            <>
              <div className="flex items-center gap-2">
                <Skeleton className="size-4 rounded-md" />
                <Skeleton className="h-3 w-20" />
              </div>
              <Skeleton className="h-6 w-24" />
              <Skeleton className="h-3 w-16" />
            </>
          ) : (
            <>
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Icon className={cn("size-4", metric.tone)} aria-hidden="true" />
                <span className="truncate">{metric.label}</span>
              </div>
              <div className="truncate text-xl font-semibold tracking-tight">
                {metric.countOnly
                  ? `${metric.value.eur}`
                  : metric.onClick
                    ? (
                      <span className={blurClass(hideSensitive)}>
                        {formatDisplayMoney(
                          metric.value.eur,
                          Math.abs(metric.value.btc),
                          currency,
                        )}
                      </span>
                    )
                    : formatMetricValue(
                        metric.value.eur,
                        Math.abs(metric.value.btc),
                        currency,
                        hideSensitive,
                      )}
              </div>
              <div className="truncate text-xs text-muted-foreground">
                {metric.meta}
              </div>
            </>
          );
          return metric.onClick && !isRefreshing ? (
            <button
              key={metric.label}
              type="button"
              className={className}
              onClick={metric.onClick}
              aria-label={metric.ariaLabel}
            >
              {content}
            </button>
          ) : (
            <div key={metric.label} className={className}>
              {content}
            </div>
          );
        })}

        <div className="relative z-40 col-span-2 flex min-h-[360px] flex-col p-3 sm:p-4 md:col-span-3 xl:col-span-6 xl:min-h-0">
          <div className="mb-3 flex shrink-0 items-start justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">
                {t(`workbench.chart.title${bucketTitleSuffix}`)}
              </h2>
              {isRefreshing ? (
                <Skeleton className="mt-1 h-3 w-36" />
              ) : (
                <p className="text-xs text-muted-foreground">
                  {t(`workbench.chart.subtitle${bucketTitleSuffix}`, {
                    count: chartRecords.length,
                    buckets: activeChartRows.length,
                  })}
                </p>
              )}
            </div>
            <div className="flex flex-col items-end gap-2">
              <div className="flex flex-wrap justify-end gap-x-2 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
                {[
                  ["incoming", t("chartSegment.incoming")],
                  ["outgoing", t("chartSegment.outgoing")],
                  ...(chartMode === "all"
                    ? [
                        ["transfer", t("chartSegment.transfers")],
                        ["swap", t("chartSegment.swaps")],
                      ]
                    : []),
                ].map(([flow, label]) => (
                  <button
                    key={flow}
                    type="button"
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 transition-colors hover:bg-muted hover:text-foreground",
                      chartSelection?.bucketKey === null &&
                        chartSelection.segment === flowChartSegmentForFlow(
                          flow as TransactionFlow,
                        ) &&
                        "bg-muted text-foreground",
                    )}
                    onClick={() => {
                      const segment = flowChartSegmentForFlow(
                        flow as TransactionFlow,
                      );
                      if (segment) handleFlowLegendClick(segment);
                    }}
                  >
                    <span
                      className="size-2.5 rounded-sm"
                      style={{
                        backgroundColor: flowColors[flow as TransactionFlow],
                      }}
                      aria-hidden="true"
                    />
                    {label}
                  </button>
                ))}
              </div>
              <div className="flex flex-wrap justify-end gap-1">
                {(["amount", "count"] satisfies FlowChartMetric[]).map(
                  (metric) => (
                    <button
                      key={metric}
                      type="button"
                      aria-pressed={chartMetric === metric}
                      onClick={() => setChartMetric(metric)}
                      className={cn(
                        "h-7 rounded-md border px-2 text-[10px] font-medium transition-colors sm:text-xs",
                        chartMetric === metric
                          ? "border-primary bg-primary text-primary-foreground"
                          : "border-border bg-background text-muted-foreground hover:text-foreground",
                      )}
                    >
                      {/* loose translator */}
                      {(t as (key: string) => string)(flowChartMetricLabels[metric])}
                    </button>
                  ),
                )}
                {(["external", "all"] satisfies FlowChartMode[]).map((mode) => (
                  <button
                    key={mode}
                    type="button"
                    aria-pressed={chartMode === mode}
                    onClick={() => setChartMode(mode)}
                    className={cn(
                      "h-7 rounded-md border px-2 text-[10px] font-medium transition-colors sm:text-xs",
                      chartMode === mode
                        ? "border-primary bg-primary text-primary-foreground"
                        : "border-border bg-background text-muted-foreground hover:text-foreground",
                    )}
                  >
                    {/* loose translator */}
                    {(t as (key: string) => string)(flowChartModeLabels[mode])}
                  </button>
                ))}
              </div>
            </div>
          </div>
          <div className="relative min-h-[280px] min-w-0 flex-1 overflow-visible">
            {isRefreshing ? (
              <div className="flex h-full min-h-[280px] items-end gap-3 border-b border-l border-border/60 px-3 pb-4">
                {[42, 64, 36, 78, 52, 88, 48, 70].map((height, index) => (
                  <Skeleton
                    key={index}
                    className="min-w-8 flex-1 rounded-t-md"
                    style={{ height: `${height}%` }}
                  />
                ))}
              </div>
            ) : (
              <ChartContainer
                config={flowChartConfig}
                className="aspect-auto h-full w-full overflow-visible [&_.recharts-responsive-container]:!overflow-visible [&_.recharts-tooltip-wrapper]:!z-[80] [&_.recharts-wrapper]:!overflow-visible"
              >
                <BarChart
                  data={visibleChartRows}
                  margin={{ top: 18, right: 42, bottom: 0, left: 0 }}
                >
                <CartesianGrid strokeDasharray="0" vertical={false} />
                <XAxis
                  dataKey="date"
                  axisLine={false}
                  tickLine={false}
                  tick={{ fontSize: 10 }}
                  dy={8}
                />
                <YAxis
                  axisLine={false}
                  tickLine={false}
                  tick={{ fontSize: 10 }}
                  width={56}
                  domain={yDomain}
                  tickFormatter={(value) =>
                    hideSensitive
                      ? ""
                      : chartMetric === "count"
                        ? String(Math.round(Number(value)))
                        : currency === "btc"
                          ? formatBtc(Number(value), {
                              precision: 4,
                              sign: Number(value) !== 0,
                            }).replace(/\s/g, "")
                          : compactCurrencyFormatter.format(Number(value))
                  }
                />
                <Tooltip
                  allowEscapeViewBox={{ x: true, y: true }}
                  cursor={{
                    className: "transition-opacity duration-150",
                    fill: "var(--primary)",
                    fillOpacity: 0.1,
                    stroke: "var(--primary)",
                    strokeOpacity: 0.55,
                    strokeWidth: 1,
                  }}
                  content={
                    <FlowTooltip
                      hideSensitive={hideSensitive}
                      currency={currency}
                      metric={chartMetric}
                    />
                  }
                  offset={32}
                  wrapperStyle={{
                    pointerEvents: "none",
                    zIndex: 80,
                  }}
                />
                <FlowBucketClickAreas
                  rows={visibleChartRows}
                  onBucketClick={selectFlowBucket}
                />
                <Bar
                  dataKey="incoming"
                  fill={flowColors.incoming}
                  isAnimationActive={false}
                  radius={[2, 2, 0, 0]}
                  cursor="pointer"
                  onClick={(data: FlowChartClickData) =>
                    handleFlowChartClick(data, "incoming")
                  }
                >
                  {visibleChartRows.map((row) => (
                    <Cell
                      key={`incoming-${row.bucketKey}`}
                      {...flowChartCellProps(row, "incoming")}
                    />
                  ))}
                  {chartMetric === "count" ? (
                    <LabelList
                      dataKey="incoming"
                      position="top"
                      formatter={formatCountBarLabel}
                    />
                  ) : null}
                </Bar>
                <Bar
                  dataKey="outgoing"
                  fill={flowColors.outgoing}
                  isAnimationActive={false}
                  radius={[0, 0, 2, 2]}
                  cursor="pointer"
                  onClick={(data: FlowChartClickData) =>
                    handleFlowChartClick(data, "outgoing")
                  }
                >
                  {visibleChartRows.map((row) => (
                    <Cell
                      key={`outgoing-${row.bucketKey}`}
                      {...flowChartCellProps(row, "outgoing")}
                    />
                  ))}
                  {chartMetric === "count" ? (
                    <LabelList
                      dataKey="outgoing"
                      position="bottom"
                      formatter={formatCountBarLabel}
                    />
                  ) : null}
                </Bar>
                <Bar
                  dataKey="transfers"
                  fill={flowColors.transfer}
                  isAnimationActive={false}
                  radius={[2, 2, 0, 0]}
                  cursor="pointer"
                  onClick={(data: FlowChartClickData) =>
                    handleFlowChartClick(data, "transfers")
                  }
                >
                  {visibleChartRows.map((row) => (
                    <Cell
                      key={`transfers-${row.bucketKey}`}
                      {...flowChartCellProps(row, "transfers")}
                    />
                  ))}
                  {chartMetric === "count" ? (
                    <LabelList
                      dataKey="transfers"
                      position="top"
                      formatter={formatCountBarLabel}
                    />
                  ) : null}
                </Bar>
                <Bar
                  dataKey="swaps"
                  fill={flowColors.swap}
                  isAnimationActive={false}
                  radius={[2, 2, 0, 0]}
                  cursor="pointer"
                  onClick={(data: FlowChartClickData) =>
                    handleFlowChartClick(data, "swaps")
                  }
                >
                  {visibleChartRows.map((row) => (
                    <Cell
                      key={`swaps-${row.bucketKey}`}
                      {...flowChartCellProps(row, "swaps")}
                    />
                  ))}
                  {chartMetric === "count" ? (
                    <LabelList
                      dataKey="swaps"
                      position="top"
                      formatter={formatCountBarLabel}
                    />
                  ) : null}
                </Bar>
                <ReferenceLine
                  y={0}
                  stroke="var(--foreground)"
                  strokeOpacity={0.55}
                  strokeWidth={2}
                />
                </BarChart>
              </ChartContainer>
            )}
          </div>
        </div>

      </section>

    </>
  );
};

function FlowTooltip({
  active,
  payload,
  label,
  hideSensitive,
  currency,
  metric,
}: ChartTooltipProps) {
  const { t } = useTranslation("transactions");
  if (!active || !payload?.length) return null;
  const rows = payload
    .filter((row) => Number(row.value ?? 0) !== 0)
    .sort((a, b) => Number(b.value ?? 0) - Number(a.value ?? 0));
  return (
    <div className="min-w-[240px] rounded-lg border bg-popover p-3 text-xs shadow-lg">
      <p className="mb-2 font-medium">{label}</p>
      <div className="space-y-2">
        {rows.map((row) => {
          const segment = flowChartSegmentFromDataKey(row.dataKey);
          const stats = segment ? row.payload?.stats[segment] : undefined;
          return (
            <div key={String(row.dataKey)} className="space-y-1.5">
              <div className="flex items-center gap-2">
                <span
                  className="size-2 rounded-sm"
                  style={{
                    backgroundColor: flowColorForSegment(segment),
                  }}
                  aria-hidden="true"
                />
                <span className="text-muted-foreground">
                  {segment
                    ? // loose translator
                      (t as (key: string) => string)(
                        flowChartSegmentLabels[segment],
                      )
                    : String(row.dataKey)}
                </span>
                <span
                  className={cn(
                    "ml-auto font-medium",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatFlowTooltipValue(
                    Number(row.value),
                    currency,
                    metric,
                    // loose translator
                    t as (key: string, opts?: Record<string, unknown>) => string,
                  )}
                </span>
              </div>
              {stats && (
                <div className="space-y-1 pl-4 text-[10px] text-muted-foreground sm:text-xs">
                  <div className="flex justify-between gap-3">
                    <span>{t("workbench.tooltip.txCount", { count: stats.count })}</span>
                    <span className={blurClass(hideSensitive)}>
                      {currency === "btc"
                        ? formatBtc(stats.btc, { precision: 8 })
                        : currencyFormatter.format(stats.eur)}
                    </span>
                  </div>
                  {stats.largest && (
                    <div className="flex justify-between gap-3">
                      <span className="truncate">
                        {t("workbench.tooltip.largest", {
                          label: stats.largest.label,
                        })}
                      </span>
                      <span
                        className={cn(
                          "shrink-0 font-medium",
                          blurClass(hideSensitive),
                        )}
                      >
                        {currency === "btc"
                          ? formatBtc(stats.largest.btc, { precision: 8 })
                          : currencyFormatter.format(stats.largest.eur)}
                      </span>
                    </div>
                  )}
                  {(stats.missingPrice > 0 ||
                    stats.review > 0 ||
                    stats.failed > 0) && (
                    <div className="flex flex-wrap gap-1 pt-0.5">
                      {stats.missingPrice > 0 && (
                        <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-600">
                          {t("workbench.tooltip.missingPrice", {
                            count: stats.missingPrice,
                          })}
                        </span>
                      )}
                      {stats.review > 0 && (
                        <span className="rounded bg-blue-500/10 px-1.5 py-0.5 text-blue-600">
                          {t("workbench.tooltip.review", { count: stats.review })}
                        </span>
                      )}
                      {stats.failed > 0 && (
                        <span className="rounded bg-[var(--kb-accent)]/10 px-1.5 py-0.5 text-[var(--kb-accent)]">
                          {t("workbench.tooltip.failed", { count: stats.failed })}
                        </span>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export { PeriodTabs, TransactionWorkbench };
