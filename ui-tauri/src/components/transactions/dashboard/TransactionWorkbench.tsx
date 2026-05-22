import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  RefreshCw,
  ShieldAlert,
  Wallet,
} from "lucide-react";
import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  LabelList,
  ReferenceLine,
  Tooltip,
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
  buildBreakdown,
  buildFlowChartRows,
  buildSwapCandidates,
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
}: {
  activePeriod: PeriodKey;
  onPeriodChange: (period: PeriodKey) => void;
}) => {
  return (
    <div className="flex items-center gap-1 rounded-lg bg-muted p-1">
      {periodKeys.map((key) => (
        <button
          key={key}
          type="button"
          onClick={() => onPeriodChange(key)}
          className={cn(
            "rounded-md px-3 py-1.5 text-xs font-medium transition-all sm:text-sm",
            activePeriod === key
              ? "bg-card text-foreground shadow-sm"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {periodLabels[key]}
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
  breakdownSelection,
  swapCandidateRefs,
  swapCandidateTotal,
  isRefreshing,
}: {
  period: PeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  onFlowSelectionChange: (selection: FlowChartSelection | null) => void;
  onQuickFilterChange: (filter: TableQuickFilter | null) => void;
  onBreakdownSelectionChange: (selection: BreakdownSelection | null) => void;
  onTableFiltersReset: () => void;
  chartSelection: FlowChartSelection | null;
  breakdownSelection: BreakdownSelection | null;
  swapCandidateRefs?: SwapCandidateReference[];
  swapCandidateTotal?: number | null;
  isRefreshing?: boolean;
}) => {
  const navigate = useNavigate();
  const [chartMetric, setChartMetric] =
    React.useState<FlowChartMetric>("amount");
  const [chartMode, setChartMode] = React.useState<FlowChartMode>("all");
  const swapCandidates = buildSwapCandidates(records, swapCandidateRefs);
  const swapCandidateIds = new Set(
    swapCandidates.flatMap((candidate) => [
      candidate.in.id,
      candidate.out.id,
    ]),
  );
  const externalRecords = records.filter((txn) => !swapCandidateIds.has(txn.id));
  const incoming = sumByFlow(externalRecords, "incoming");
  const outgoing = sumByFlow(externalRecords, "outgoing");
  const transfers = sumByFlow(externalRecords, "transfer");
  const markedSwaps = sumByFlow(records, "swap");
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
  const withoutExplorer = records.filter((txn) => !txn.explorerId).length;
  const missingPriceCount = records.filter((txn) => !txn.rate).length;
  const chartRecords =
    chartMode === "external"
      ? externalRecords.filter(
          (txn) =>
            !swapCandidateIds.has(txn.id) &&
            ["incoming", "outgoing"].includes(transactionFlow(txn)),
        )
      : records;
  const chartRows = buildFlowChartRows(
    chartRecords,
    period,
    currency,
    swapCandidateIds,
    chartMetric,
  );
  const activeChartRows = chartRows.filter((row) => flowPointTotal(row) > 0);
  const visibleChartRows = activeChartRows.length ? activeChartRows : chartRows;
  const yDomain = flowAxisDomain(visibleChartRows, chartMetric);
  const flowChartCellProps = React.useCallback(
    (row: FlowChartPoint, segment: FlowChartSegment) => {
      const selected =
        chartSelection?.segment === segment &&
        (chartSelection.bucketKey === null ||
          chartSelection.bucketKey === row.bucketKey);
      const dimmed = Boolean(chartSelection && !selected);
      return {
        fillOpacity: dimmed ? 0.32 : 1,
        stroke: selected ? "var(--foreground)" : "transparent",
        strokeWidth: selected ? 1.5 : 0,
      };
    },
    [chartSelection],
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
        bucketLabel: periodLabels[period],
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
  const handleSummaryFlowClick = React.useCallback(
    (segment: FlowChartSegment) => {
      onQuickFilterChange(null);
      onBreakdownSelectionChange(null);
      onTableFiltersReset();
      onFlowSelectionChange({
        id: `${period}:summary:${segment}:all`,
        period,
        bucketKey: null,
        bucketLabel: periodLabels[period],
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
  const handleBreakdownClick = React.useCallback(
    (dimension: BreakdownSelection["dimension"], key: string) => {
      onFlowSelectionChange(null);
      onQuickFilterChange(null);
      onTableFiltersReset();
      onBreakdownSelectionChange({ dimension, key });
    },
    [
      onBreakdownSelectionChange,
      onFlowSelectionChange,
      onQuickFilterChange,
      onTableFiltersReset,
    ],
  );
  const networkRows = buildBreakdown(records, (txn) => txn.paymentMethod);
  const walletRows = buildBreakdown(records, (txn) => txn.wallet ?? "Unassigned");
  const maxNetworkValue = Math.max(...networkRows.map((row) => row.eur), 1);
  const maxWalletValue = Math.max(...walletRows.map((row) => row.eur), 1);
  const metricCards = [
    {
      label: "Incoming",
      value: incoming,
      meta: `${incoming.count} tx`,
      icon: ArrowDownRight,
      tone: "text-emerald-600",
      onClick:
        incoming.count > 0
          ? () => handleSummaryFlowClick("incoming")
          : undefined,
      ariaLabel: "Show incoming transactions",
    },
    {
      label: "Outgoing",
      value: outgoing,
      meta: `${outgoing.count} tx`,
      icon: ArrowUpRight,
      tone: "text-red-600",
      onClick:
        outgoing.count > 0
          ? () => handleSummaryFlowClick("outgoing")
          : undefined,
      ariaLabel: "Show outgoing transactions",
    },
    {
      label: "Net flow",
      value: { eur: netEur, btc: netBtc },
      meta: netEur >= 0 ? "inflow" : "outflow",
      icon: ArrowLeftRight,
      tone: netEur >= 0 ? "text-emerald-600" : "text-red-600",
      onClick:
        incoming.count + outgoing.count > 0
          ? handleNetFlowClick
          : undefined,
      ariaLabel: "Show external flow transactions",
    },
    {
      label: "Transfers",
      value: transfers,
      meta: `${transfers.count} moves`,
      icon: Wallet,
      tone: "text-muted-foreground",
      onClick: () => handleSummaryFlowClick("transfers"),
      ariaLabel: "Show transfer transactions",
    },
    {
      label: "Swaps",
      value: swaps,
      meta:
        knownSwapCandidateCount === null
          ? "unpaired unknown"
          : knownSwapCandidateCount > 0 && markedSwaps.count > 0
          ? `${knownSwapCandidateCount} unpaired · ${markedSwaps.count} paired`
          : knownSwapCandidateCount > 0
          ? `${knownSwapCandidateCount} unpaired`
          : `${markedSwaps.count} paired`,
      icon: RefreshCw,
      tone:
        knownSwapCandidateCount === null || knownSwapCandidateCount > 0
          ? "text-amber-600"
          : "text-muted-foreground",
      onClick: handleSwapWorkflowClick,
      ariaLabel:
        knownSwapCandidateCount === null || knownSwapCandidateCount > 0
          ? "Open pairing candidates"
          : "Show paired swap transactions",
    },
    {
      label: "Review queue",
      value: { eur: reviewCount + pendingCount + failedCount, btc: 0 },
      meta: `${reviewCount} review · ${pendingCount} pending`,
      icon: ShieldAlert,
      tone:
        reviewCount || pendingCount || failedCount
          ? "text-amber-600"
          : "text-emerald-600",
      countOnly: true,
      onClick: handleReviewQueueClick,
      ariaLabel: "Show review queue transactions",
    },
  ];

  return (
    <>
      <section
        className="grid grid-cols-2 overflow-hidden rounded-xl border bg-card md:grid-cols-3 xl:grid-cols-6"
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

        <div className="col-span-2 flex min-h-[360px] flex-col border-b p-3 sm:p-4 md:col-span-3 xl:col-span-4 xl:min-h-0 xl:border-b-0">
          <div className="mb-3 flex shrink-0 items-start justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">
                Flow by active {flowBucketLabel(period)}
              </h2>
              {isRefreshing ? (
                <Skeleton className="mt-1 h-3 w-36" />
              ) : (
                <p className="text-xs text-muted-foreground">
                  {chartRecords.length} tx across {activeChartRows.length} active{" "}
                  {activeChartRows.length === 1
                    ? flowBucketLabel(period)
                    : `${flowBucketLabel(period)}s`}
                </p>
              )}
            </div>
            <div className="flex flex-col items-end gap-2">
              <div className="flex flex-wrap justify-end gap-x-2 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
                {[
                  ["incoming", "Incoming"],
                  ["outgoing", "Outgoing"],
                  ...(chartMode === "all"
                    ? [
                        ["transfer", "Transfers"],
                        ["swap", "Swaps"],
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
                      {flowChartMetricLabels[metric]}
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
                    {flowChartModeLabels[mode]}
                  </button>
                ))}
              </div>
            </div>
          </div>
          <div className="min-h-[280px] min-w-0 flex-1">
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
                className="h-full w-full overflow-visible [&_.recharts-tooltip-wrapper]:!z-30 [&_.recharts-wrapper]:!overflow-visible"
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
                    zIndex: 30,
                  }}
                />
                <Bar
                  dataKey="incoming"
                  fill={flowColors.incoming}
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

        <div className="col-span-2 grid gap-0 sm:grid-cols-2 md:col-span-3 xl:col-span-2 xl:grid-cols-1 xl:border-l">
          <BreakdownPanel
            title="Network mix"
            rows={networkRows}
            maxValue={maxNetworkValue}
            currency={currency}
            hideSensitive={hideSensitive}
            isRefreshing={isRefreshing}
            selectedKey={
              breakdownSelection?.dimension === "network"
                ? breakdownSelection.key
                : null
            }
            onSelect={(key) => handleBreakdownClick("network", key)}
          />
          <BreakdownPanel
            title="Wallet/source mix"
            rows={walletRows.slice(0, 4)}
            maxValue={maxWalletValue}
            currency={currency}
            hideSensitive={hideSensitive}
            isRefreshing={isRefreshing}
            selectedKey={
              breakdownSelection?.dimension === "wallet"
                ? breakdownSelection.key
                : null
            }
            onSelect={(key) => handleBreakdownClick("wallet", key)}
          />
          <div className="border-t p-3 sm:col-span-2 lg:col-span-1 sm:p-4">
            <h3 className="mb-2 text-sm font-semibold">Data quality</h3>
            {isRefreshing ? (
              <div className="space-y-3 py-1">
                {Array.from({ length: 4 }).map((_, index) => (
                  <div
                    key={index}
                    className="grid grid-cols-[minmax(0,1fr)_48px] items-center gap-3"
                  >
                    <Skeleton className="h-3 w-full" />
                    <Skeleton className="h-4 w-8 justify-self-end" />
                  </div>
                ))}
              </div>
            ) : (
              <div className="divide-y text-xs">
                <QualityRow label="No explorer id" value={withoutExplorer} />
                <QualityRow label="Missing price" value={missingPriceCount} />
                <QualityRow label="Failed import" value={failedCount} />
                <QualityRow
                  label="Swap candidates"
                  value={swapCandidateTotals.count}
                  onClick={swapCandidateTotals.count > 0 ? openSwapWorkflow : undefined}
                />
              </div>
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
                    ? flowChartSegmentLabels[segment]
                    : String(row.dataKey)}
                </span>
                <span
                  className={cn(
                    "ml-auto font-medium",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatFlowTooltipValue(Number(row.value), currency, metric)}
                </span>
              </div>
              {stats && (
                <div className="space-y-1 pl-4 text-[10px] text-muted-foreground sm:text-xs">
                  <div className="flex justify-between gap-3">
                    <span>{stats.count} tx</span>
                    <span className={blurClass(hideSensitive)}>
                      {currency === "btc"
                        ? formatBtc(stats.btc, { precision: 8 })
                        : currencyFormatter.format(stats.eur)}
                    </span>
                  </div>
                  {stats.largest && (
                    <div className="flex justify-between gap-3">
                      <span className="truncate">
                        Largest: {stats.largest.label}
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
                          {stats.missingPrice} missing price
                        </span>
                      )}
                      {stats.review > 0 && (
                        <span className="rounded bg-blue-500/10 px-1.5 py-0.5 text-blue-600">
                          {stats.review} review
                        </span>
                      )}
                      {stats.failed > 0 && (
                        <span className="rounded bg-[var(--color-accent)]/10 px-1.5 py-0.5 text-[var(--color-accent)]">
                          {stats.failed} failed
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

function BreakdownPanel({
  title,
  rows,
  maxValue,
  currency,
  hideSensitive,
  isRefreshing,
  selectedKey,
  onSelect,
}: {
  title: string;
  rows: Array<{ key: string; count: number; eur: number; btc: number }>;
  maxValue: number;
  currency: Currency;
  hideSensitive: boolean;
  isRefreshing?: boolean;
  selectedKey?: string | null;
  onSelect?: (key: string) => void;
}) {
  return (
    <div className="border-t p-3 first:border-t-0 sm:p-4">
      <h3 className="mb-3 text-sm font-semibold">{title}</h3>
      {isRefreshing ? (
        <div className="space-y-3">
          {Array.from({ length: title === "Wallet/source mix" ? 4 : 2 }).map(
            (_, index) => (
              <div key={index} className="space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <Skeleton className="h-3 w-28" />
                  <Skeleton className="h-3 w-16" />
                </div>
                <Skeleton className="h-1.5 w-full rounded-full" />
              </div>
            ),
          )}
        </div>
      ) : (
        <div className="space-y-2.5">
          {rows.map((row) => {
          const selected = selectedKey === row.key;
          const content = (
            <>
            <div className="flex items-center justify-between gap-2 text-xs">
              <span className="truncate font-medium">{row.key}</span>
              <span className="shrink-0 text-muted-foreground">
                {row.count} ·{" "}
                <CurrencyToggleText className={blurClass(hideSensitive)}>
                  {formatDisplayMoney(row.eur, row.btc, currency)}
                </CurrencyToggleText>
              </span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full bg-primary"
                style={{ width: `${Math.max(6, (row.eur / maxValue) * 100)}%` }}
              />
            </div>
            </>
          );
          return onSelect ? (
            <button
              key={row.key}
              type="button"
              className={cn(
                "-mx-1.5 block w-[calc(100%+0.75rem)] space-y-1 rounded-md px-1.5 py-1.5 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                selected && "bg-muted/70",
              )}
              onClick={() => onSelect(row.key)}
              aria-pressed={selected}
            >
              {content}
            </button>
          ) : (
            <div key={row.key} className="space-y-1">
              {content}
            </div>
          );
        })}
        </div>
      )}
    </div>
  );
}

function QualityRow({
  label,
  value,
  onClick,
}: {
  label: string;
  value: number;
  onClick?: () => void;
}) {
  const tone = value > 0 ? "text-amber-600" : "text-emerald-600";
  const className = cn(
    "-mx-1 grid min-h-8 w-[calc(100%+0.5rem)] grid-cols-[minmax(0,1fr)_auto] items-center gap-3 rounded-md px-1 py-1.5 text-left",
    onClick &&
      "cursor-pointer transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
  );
  const content = (
    <>
      <span className="min-w-0 truncate text-muted-foreground">{label}</span>
      <span className={cn("shrink-0 font-semibold leading-none tabular-nums", tone)}>
        {value}
      </span>
    </>
  );
  return onClick ? (
    <button type="button" className={className} onClick={onClick}>
      {content}
    </button>
  ) : (
    <div className={className}>{content}</div>
  );
}

export { PeriodTabs, TransactionWorkbench };
