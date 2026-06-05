import {
  Maximize2,
  Settings,
  X,
} from "lucide-react";
import * as React from "react";
import {
  Area,
  AreaChart,
  Brush,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceLine,
  Scatter,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";

import { Button } from "@/components/ui/button";
import { type ChartConfig, ChartContainer } from "@/components/ui/chart";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { formatBtc, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import { ActivityScatterDot } from "./ActivityScatterDot";
import { ChartControlsSheet } from "./ChartControlsSheet";
import { ChartStat } from "./ChartStat";
import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  activityMarkerView,
  buildTreasuryChartStats,
  blurClass,
  defaultTreasurySeriesVisibility,
  DEFAULT_INCOMING_MARKER_MIN_BTC,
  DEFAULT_OUTGOING_MARKER_MIN_BTC,
  enrichTreasuryChartData,
  formatBtcAxis,
  formatFiatPrice,
  formatRelativeMarketRateTime,
  formatTreasuryDetailDate,
  formatTreasuryTick,
  fullTreasuryBrushRange,
  getDataForPeriod,
  initialActivityMarkerMinimumFromUrl,
  initialTimePeriodFromUrl,
  INCOMING_MARKER_MIN_PARAM,
  LEGACY_INCOMING_MARKER_MIN_PARAM,
  LEGACY_OUTGOING_MARKER_MIN_PARAM,
  marketRateDetailLabel,
  normalizeTreasuryBrushRange,
  OUTGOING_MARKER_MIN_PARAM,
  periodShortLabels,
  portfolioAxisTicks,
  portfolioChartColors,
  rawTreasuryBrushRange,
  sameTreasuryBrushRange,
  serializeActivityMarkerMinimum,
  treasuryPrimaryValue,
  useHoverHighlight,
  useResolvedColorMode,
  type PortfolioChartMouseState,
  type TimePeriod,
  type TreasuryBrushChange,
  type TreasuryBrushRange,
  type TreasuryChartPoint,
  type TreasuryChartSeriesKey,
  type TreasuryLegendItem,
  type TreasurySeriesVisibility,
} from "./model";
import { PortfolioInspector } from "./PortfolioInspector";
import { ActivityLegendSwatch } from "./ChartControlsSheet";
import { TreasuryTooltip } from "./TreasuryTooltip";

export const BtcActivityChart = ({
  snapshot,
  hideSensitive,
  currency,
  onOpenTransactionDetail,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
  onOpenTransactionDetail?: (transactionId: string) => void;
}) => {
  const [period, setPeriod] =
    React.useState<TimePeriod>(initialTimePeriodFromUrl);
  const [expandedPointDate, setExpandedPointDate] = React.useState<string | null>(
    null,
  );
  const [seriesVisible, setSeriesVisible] =
    React.useState<TreasurySeriesVisibility>(defaultTreasurySeriesVisibility);
  const [incomingMarkerMinimumBtc, setIncomingMarkerMinimumBtc] =
    React.useState(() =>
      initialActivityMarkerMinimumFromUrl(
        INCOMING_MARKER_MIN_PARAM,
        DEFAULT_INCOMING_MARKER_MIN_BTC,
        LEGACY_INCOMING_MARKER_MIN_PARAM,
      ),
    );
  const [outgoingMarkerMinimumBtc, setOutgoingMarkerMinimumBtc] =
    React.useState(() =>
      initialActivityMarkerMinimumFromUrl(
        OUTGOING_MARKER_MIN_PARAM,
        DEFAULT_OUTGOING_MARKER_MIN_BTC,
        LEGACY_OUTGOING_MARKER_MIN_PARAM,
      ),
    );
  const [chartControlsOpen, setChartControlsOpen] = React.useState(false);
  const [expandedChartControlsOpen, setExpandedChartControlsOpen] =
    React.useState(false);
  const [compactBrushRange, setCompactBrushRange] =
    React.useState<TreasuryBrushRange | null>(null);
  const [expandedBrushRange, setExpandedBrushRange] =
    React.useState<TreasuryBrushRange | null>(null);
  const [compactBrushRevision, setCompactBrushRevision] = React.useState(0);
  const [expandedBrushRevision, setExpandedBrushRevision] = React.useState(0);
  const { active: activeSeries, handleHover } =
    useHoverHighlight<TreasuryChartSeriesKey>();
  const colorMode = useResolvedColorMode();
  const chartColors = portfolioChartColors[colorMode];
  const primaryColor = chartColors.value;
  const secondaryColor = chartColors.costBasis;
  const chartConfig = React.useMemo(
    () =>
      ({
        primary: {
          label: "BTC Balance",
          color: primaryColor,
        },
        price: {
          label: "BTC Price",
          color: "#94a3b8",
        },
        basis: {
          label: "Avg Basis",
          color: secondaryColor,
        },
        events: {
          label: "Activity",
          color: "#f97316",
        },
      }) satisfies ChartConfig,
    [primaryColor, secondaryColor],
  );

  const legendItems: TreasuryLegendItem[] = [
    {
      key: "primary" as const,
      label: "BTC Balance",
      color: primaryColor,
      dashed: false,
    },
    {
      key: "events" as const,
      label: "Activity",
      color: "#f97316",
      dashed: false,
    },
    {
      key: "basis" as const,
      label: "Avg Basis",
      color: secondaryColor,
      dashed: true,
    },
    {
      key: "price" as const,
      label: "BTC Price",
      color: "#94a3b8",
      dashed: true,
    },
  ];

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const timeout = window.setTimeout(() => {
      const params = new URLSearchParams(window.location.search);
      params.set("period", period);
      params.delete(LEGACY_INCOMING_MARKER_MIN_PARAM);
      params.delete(LEGACY_OUTGOING_MARKER_MIN_PARAM);
      if (incomingMarkerMinimumBtc === DEFAULT_INCOMING_MARKER_MIN_BTC) {
        params.delete(INCOMING_MARKER_MIN_PARAM);
      } else {
        params.set(
          INCOMING_MARKER_MIN_PARAM,
          serializeActivityMarkerMinimum(incomingMarkerMinimumBtc),
        );
      }
      if (outgoingMarkerMinimumBtc === DEFAULT_OUTGOING_MARKER_MIN_BTC) {
        params.delete(OUTGOING_MARKER_MIN_PARAM);
      } else {
        params.set(
          OUTGOING_MARKER_MIN_PARAM,
          serializeActivityMarkerMinimum(outgoingMarkerMinimumBtc),
        );
      }
      const nextQuery = params.toString();
      const nextUrl = nextQuery
        ? `${window.location.pathname}?${nextQuery}`
        : window.location.pathname;
      window.history.replaceState(null, "", nextUrl);
    }, 150);
    return () => window.clearTimeout(timeout);
  }, [incomingMarkerMinimumBtc, outgoingMarkerMinimumBtc, period]);

  React.useEffect(() => {
    setExpandedPointDate(null);
  }, [currency, period]);

  const chartData = React.useMemo(
    () =>
      enrichTreasuryChartData(
        getDataForPeriod(period, snapshot, "value", currency, "compact"),
        snapshot,
        period,
      ),
    [currency, period, snapshot],
  );
  const expandedChartData = React.useMemo(
    () =>
      enrichTreasuryChartData(
        getDataForPeriod(period, snapshot, "value", currency, "detailed"),
        snapshot,
        period,
      ),
    [currency, period, snapshot],
  );
  const toggleSeries = React.useCallback((key: TreasuryChartSeriesKey) => {
    setSeriesVisible((current) => ({ ...current, [key]: !current[key] }));
  }, []);
  const resetActivityMarkerMinimums = React.useCallback(() => {
    setIncomingMarkerMinimumBtc(DEFAULT_INCOMING_MARKER_MIN_BTC);
    setOutgoingMarkerMinimumBtc(DEFAULT_OUTGOING_MARKER_MIN_BTC);
  }, []);
  const openActivityPointTransaction = React.useCallback(
    (point: unknown) => {
      if (!onOpenTransactionDetail) return;
      const payload =
        (point as { payload?: TreasuryChartPoint } | null)?.payload ??
        (point as TreasuryChartPoint | null);
      const transactionId = payload?.eventTransactionId ?? payload?.eventId;
      if (!transactionId) return;
      onOpenTransactionDetail(transactionId);
    },
    [onOpenTransactionDetail],
  );
  const activityMarkerMinimumForPoint = React.useCallback(
    (point: TreasuryChartPoint) => {
      if (point.eventFlow === "incoming") return incomingMarkerMinimumBtc;
      if (point.eventFlow === "outgoing" || point.eventFlow === "fee") {
        return outgoingMarkerMinimumBtc;
      }
      return Math.min(incomingMarkerMinimumBtc, outgoingMarkerMinimumBtc);
    },
    [incomingMarkerMinimumBtc, outgoingMarkerMinimumBtc],
  );
  const compactMarkerView = React.useMemo(
    () =>
      activityMarkerView(
        chartData,
        seriesVisible.events,
        activityMarkerMinimumForPoint,
        period === "30days",
      ),
    [activityMarkerMinimumForPoint, chartData, period, seriesVisible.events],
  );
  const expandedMarkerView = React.useMemo(
    () =>
      activityMarkerView(
        expandedChartData,
        seriesVisible.events,
        activityMarkerMinimumForPoint,
        period === "30days",
      ),
    [activityMarkerMinimumForPoint, expandedChartData, period, seriesVisible.events],
  );

  React.useEffect(() => {
    const data = compactMarkerView.chartDisplayData;
    setCompactBrushRange((current) => {
      const next = normalizeTreasuryBrushRange(
        data,
        current ?? fullTreasuryBrushRange(data.length),
        current,
      );
      return sameTreasuryBrushRange(current, next) ? current : next;
    });
  }, [compactMarkerView.chartDisplayData]);

  React.useEffect(() => {
    const data = expandedMarkerView.chartDisplayData;
    setExpandedBrushRange((current) => {
      const next = normalizeTreasuryBrushRange(
        data,
        current ?? fullTreasuryBrushRange(data.length),
        current,
      );
      return sameTreasuryBrushRange(current, next) ? current : next;
    });
  }, [expandedMarkerView.chartDisplayData]);

  const renderChartCard = (expanded = false) => {
    const plottedData = expanded ? expandedChartData : chartData;
    const markerView = expanded ? expandedMarkerView : compactMarkerView;
    const brushRange = expanded ? expandedBrushRange : compactBrushRange;
    const setBrushRange = expanded ? setExpandedBrushRange : setCompactBrushRange;
    const brushRevision = expanded ? expandedBrushRevision : compactBrushRevision;
    const bumpBrushRevision = expanded
      ? setExpandedBrushRevision
      : setCompactBrushRevision;
    const controlsOpen = expanded ? expandedChartControlsOpen : chartControlsOpen;
    const setControlsOpen = expanded
      ? setExpandedChartControlsOpen
      : setChartControlsOpen;
    const {
      activityPoints,
      chartDisplayData,
      visibleActivityMarkers,
    } = markerView;
    const latestPoint = plottedData.at(-1);
    const brushGradientId = expanded
      ? "treasuryBrushGradientExpanded"
      : "treasuryBrushGradient";
    const effectiveBrushRange =
      brushRange ?? fullTreasuryBrushRange(chartDisplayData.length);
    const selectedChartDisplayData =
      chartDisplayData.length > 3
        ? chartDisplayData.slice(
            effectiveBrushRange.startIndex,
            effectiveBrushRange.endIndex + 1,
          )
        : chartDisplayData;
    const handleBrushChange = (range: TreasuryBrushChange) => {
      const normalizedRange = normalizeTreasuryBrushRange(
        chartDisplayData,
        range,
        effectiveBrushRange,
      );
      const rawRange = rawTreasuryBrushRange(
        chartDisplayData.length,
        range,
        effectiveBrushRange,
      );
      if (!sameTreasuryBrushRange(rawRange, normalizedRange)) {
        // Recharts keeps the dragged handle's internal position; remount the
        // brush after state settles so the visual handle reflects the clamped range.
        window.setTimeout(() => bumpBrushRevision((revision) => revision + 1), 0);
      }
      setBrushRange((current) =>
        sameTreasuryBrushRange(current, normalizedRange)
          ? current
          : normalizedRange,
      );
    };
    const resetBrushRange = () => {
      const fullRange = fullTreasuryBrushRange(chartDisplayData.length);
      setBrushRange((current) => {
        if (sameTreasuryBrushRange(current, fullRange)) return current;
        window.setTimeout(() => bumpBrushRevision((revision) => revision + 1), 0);
        return fullRange;
      });
    };
    const visibleLatestReserve = snapshot.fiat.eurBalance;
    const visibleCostBasis = snapshot.fiat.eurCostBasis;
    const gainEur = visibleLatestReserve - visibleCostBasis;
    const gainPct = visibleCostBasis
      ? (gainEur / Math.abs(visibleCostBasis)) * 100
      : null;
    const fiatCurrency = activeMarketFiatCurrency(snapshot);
    const fiatRate = activeMarketFiatRate(snapshot);
    const incomingActivityPoints = activityPoints.filter(
      (point) => point.eventFlow === "incoming",
    );
    const visibleIncomingMarkers = visibleActivityMarkers.filter(
      (point) => point.eventFlow === "incoming",
    );
    const outgoingActivityPoints = activityPoints.filter(
      (point) => point.eventFlow === "outgoing" || point.eventFlow === "fee",
    );
    const visibleOutgoingMarkers = visibleActivityMarkers.filter(
      (point) => point.eventFlow === "outgoing" || point.eventFlow === "fee",
    );
    const activityEvents = activityPoints.length;
    const receivedBtc = activityPoints.reduce(
      (sum, point) =>
        point.eventFlow === "incoming" ? sum + point.activityBtc : sum,
      0,
    );
    const spentBtc = activityPoints.reduce(
      (sum, point) =>
        point.eventFlow === "outgoing" ? sum + point.activityBtc : sum,
      0,
    );
    const swapBtc = activityPoints.reduce(
      (sum, point) => (point.eventFlow === "swap" ? sum + point.activityBtc : sum),
      0,
    );
    const feeBtc = activityPoints.reduce(
      (sum, point) => {
        const markerFee = point.eventFeeBtc ?? 0;
        if (markerFee > 0) return sum + markerFee;
        return point.eventFlow === "fee" ? sum + point.activityBtc : sum;
      },
      0,
    );
    const netBtc = receivedBtc - spentBtc - feeBtc;
    const balancePoints = selectedChartDisplayData.filter(
      (point) => !point.isActivityEvent,
    );
    const chartStats = buildTreasuryChartStats(
      balancePoints.length ? balancePoints : selectedChartDisplayData,
    );
    const statPeriodLabel = periodShortLabels[period];
    const selectedPoint = expanded
      ? (selectedChartDisplayData.find(
          (point) => point.date === expandedPointDate,
        ) ??
        selectedChartDisplayData.at(-1) ??
        null)
      : null;
    const selectedPointIndex = selectedPoint
      ? selectedChartDisplayData.findIndex(
          (point) => point.date === selectedPoint.date,
        )
      : -1;
    const previousPoint =
      selectedPointIndex > 0
        ? selectedChartDisplayData[selectedPointIndex - 1]
        : null;
    const handleExpandedChartMove = (state: PortfolioChartMouseState) => {
      if (!expanded) return;
      const point = state.activePayload?.find((item) => item.payload)?.payload;
      if (point) setExpandedPointDate(point.date);
    };
    const isActivityMarkerEvent = (event: React.MouseEvent) =>
      event.target instanceof Element &&
      event.target.closest("[data-activity-marker='true']");
    const handleChartDoubleClick = (event: React.MouseEvent) => {
      if (isActivityMarkerEvent(event)) return;
      if (plottedData.length <= 3) return;
      resetBrushRange();
      event.preventDefault();
    };
    const handleChartClick = (event: React.MouseEvent) => {
      if (isActivityMarkerEvent(event)) return;
      if (event.detail >= 2) handleChartDoubleClick(event);
    };
    const handleChartMouseDown = (event: React.MouseEvent) => {
      if (isActivityMarkerEvent(event)) return;
      if (event.detail >= 2) handleChartDoubleClick(event);
    };
    const xAxisTicks = portfolioAxisTicks(
      balancePoints.length ? balancePoints : selectedChartDisplayData,
      period,
      expanded,
    );
    const detailDate = latestPoint
      ? formatTreasuryDetailDate(latestPoint.date)
      : "Current snapshot";
    const priceSyncedAt =
      snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp;
    const priceSyncLabel = formatRelativeMarketRateTime(priceSyncedAt);
    const priceSyncDetail = marketRateDetailLabel(snapshot);
    return (
      <div className="relative z-10 flex min-w-0 flex-1 flex-col gap-4 overflow-visible rounded-xl border bg-card p-3 sm:p-4">
        <ChartControlsSheet
          open={controlsOpen}
          onOpenChange={setControlsOpen}
          period={period}
          onPeriodChange={setPeriod}
          primaryColor={primaryColor}
          legendItems={legendItems}
          seriesVisible={seriesVisible}
          onToggleSeries={toggleSeries}
          activeSeries={activeSeries}
          onHoverSeries={handleHover}
          markerCount={activityPoints.length}
          visibleMarkerCount={visibleActivityMarkers.length}
          incomingMarkerCount={incomingActivityPoints.length}
          visibleIncomingMarkerCount={visibleIncomingMarkers.length}
          outgoingMarkerCount={outgoingActivityPoints.length}
          visibleOutgoingMarkerCount={visibleOutgoingMarkers.length}
          incomingMarkerMinimumBtc={incomingMarkerMinimumBtc}
          onIncomingMarkerMinimumChange={setIncomingMarkerMinimumBtc}
          outgoingMarkerMinimumBtc={outgoingMarkerMinimumBtc}
          onOutgoingMarkerMinimumChange={setOutgoingMarkerMinimumBtc}
          onResetMarkerMinimums={resetActivityMarkerMinimums}
          hideSensitive={hideSensitive}
        />
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div
            className="min-w-0"
            aria-label="BTC activity chart"
          >
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <p className="text-sm font-semibold text-foreground">
                BTC activity
              </p>
              <span className="text-[10px] text-muted-foreground">
                As of {detailDate}
              </span>
              {priceSyncLabel && (
                <span
                  className="text-[10px] text-muted-foreground"
                  title={priceSyncDetail}
                >
                  priced {priceSyncLabel}
                </span>
              )}
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <span>
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {activityEvents.toLocaleString("en-US")}
                </span>{" "}
                events
              </span>
              <span>
                net{" "}
                <span
                  className={cn(
                    "font-semibold",
                    netBtc >= 0
                      ? "text-emerald-600 dark:text-emerald-400"
                      : "text-[var(--color-accent)]",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatBtc(netBtc, { precision: 4, sign: true })}
                </span>
              </span>
              <span>
                in{" "}
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {formatBtc(receivedBtc, { precision: 4 })}
                </span>
              </span>
              <span>
                out{" "}
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {formatBtc(spentBtc, { precision: 4 })}
                </span>
              </span>
              {swapBtc > 0 && (
                <span>
                  swap{" "}
                  <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                    {formatBtc(swapBtc, { precision: 4 })}
                  </span>
                </span>
              )}
              {gainPct !== null && (
                <span
                  className={cn(
                    "font-semibold",
                    gainEur >= 0
                      ? "text-emerald-600 dark:text-emerald-400"
                      : "text-[var(--color-accent)]",
                    blurClass(hideSensitive),
                  )}
                >
                  {gainEur >= 0 ? "+ " : "- "}
                  {Math.abs(gainPct).toFixed(2)}% unrealized ({fiatCurrency})
                </span>
              )}
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded border bg-muted/30 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
              {periodShortLabels[period]}
            </span>
            <Button
              type="button"
              variant={controlsOpen ? "outline" : "ghost"}
              size="icon"
              className="size-8"
              aria-label="Toggle chart controls"
              aria-expanded={controlsOpen}
              onClick={() => setControlsOpen((open) => !open)}
            >
              <Settings className="size-4" aria-hidden="true" />
            </Button>
            {!expanded && (
              <DialogTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8"
                  aria-label="Expand BTC activity chart"
                >
                  <Maximize2 className="size-4" aria-hidden="true" />
                </Button>
              </DialogTrigger>
            )}
            {expanded && (
              <DialogClose asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-8"
                  aria-label="Close BTC activity chart"
                >
                  <X className="size-4" aria-hidden="true" />
                </Button>
              </DialogClose>
            )}
          </div>
        </div>

        {expanded && chartStats && (
          <div className="grid gap-2 rounded-lg border bg-muted/25 p-2 sm:grid-cols-3">
            <ChartStat
              label="Change in BTC balance"
              value={formatBtc(Math.abs(chartStats.delta), { precision: 4 })}
              detail={
                chartStats.pct === null
                  ? statPeriodLabel
                  : `${statPeriodLabel} · ${chartStats.pct >= 0 ? "+" : "-"}${Math.abs(
                      chartStats.pct,
                    ).toFixed(1)}%`
              }
              tone={chartStats.delta >= 0 ? "good" : "bad"}
              prefix={chartStats.delta >= 0 ? "+ " : "- "}
              hidden={hideSensitive}
            />
            <ChartStat
              label="Highest BTC position"
              value={formatBtc(treasuryPrimaryValue(chartStats.highPoint), {
                precision: 4,
              })}
              detail={`${statPeriodLabel} · ${chartStats.highPoint.detailLabel}`}
              hidden={hideSensitive}
            />
            <ChartStat
              label="Lowest BTC position"
              value={formatBtc(treasuryPrimaryValue(chartStats.lowPoint), {
                precision: 4,
              })}
              detail={`${statPeriodLabel} · ${chartStats.lowPoint.detailLabel}`}
              hidden={hideSensitive}
            />
          </div>
        )}

        <div className="mt-1 flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {legendItems.map((item) => (
            <div
              key={item.key}
              className={cn(
                "flex items-center gap-1.5 transition-opacity duration-200 motion-reduce:transition-none",
                !seriesVisible[item.key] && "opacity-30",
                activeSeries !== null &&
                  activeSeries !== item.key &&
                  "opacity-40",
              )}
              onMouseEnter={() => handleHover(item.key)}
              onMouseLeave={() => handleHover(null)}
            >
              {item.key === "events" ? (
                <ActivityLegendSwatch muted={!seriesVisible.events} />
              ) : (
                <span
                  className={cn(
                    "h-0.5 w-5 rounded-full",
                    item.dashed && "border-t border-dashed bg-transparent",
                  )}
                  style={{
                    backgroundColor: item.dashed ? "transparent" : item.color,
                    borderColor: item.color,
                  }}
                />
              )}
              <span>{item.label}</span>
            </div>
          ))}
        </div>

        <div
          className={cn(
            "min-w-0",
            expanded
              ? "grid gap-3 xl:grid-cols-[minmax(0,1fr)_300px]"
              : "relative",
          )}
        >
          <div
            className={cn(
              "relative flex w-full min-w-0 flex-col",
              expanded
                ? "h-[min(64vh,620px)]"
                : "h-[380px] sm:h-[456px]",
            )}
          >
            <div className="grid min-h-0 flex-1 grid-cols-[18px_minmax(0,1fr)_20px]">
              <div className="pointer-events-none flex items-center justify-center">
                <span className="-rotate-90 whitespace-nowrap text-[10px] font-semibold text-muted-foreground">
                  BTC Balance
                </span>
              </div>
              <div
                className="flex h-full min-h-0 w-full flex-col overflow-visible"
                onClickCapture={handleChartClick}
                onDoubleClickCapture={handleChartDoubleClick}
                onMouseDownCapture={handleChartMouseDown}
              >
                <ChartContainer
                  config={chartConfig}
                  className="min-h-0 flex-1 w-full overflow-visible [&_.recharts-active-dot]:pointer-events-none [&_.recharts-active-dot_*]:pointer-events-none [&_.recharts-tooltip-wrapper]:!z-[100]"
                >
                  <ComposedChart
                    data={selectedChartDisplayData}
                    onMouseMove={
                      expanded
                        ? (state) =>
                            handleExpandedChartMove(
                              state as PortfolioChartMouseState,
                            )
                        : undefined
                    }
                    margin={{
                      top: expanded ? 12 : 2,
                      right: expanded ? 8 : 4,
                      bottom: plottedData.length > 3 ? (expanded ? 14 : 8) : 0,
                      left: expanded ? 52 : 48,
                    }}
                  >
                  <CartesianGrid
                    strokeDasharray="0"
                    vertical
                    strokeOpacity={0.45}
                  />
                  <XAxis
                    dataKey="date"
                    // Scatter has its own marker data; category lookup must use
                    // the date value instead of the marker array index.
                    allowDuplicatedCategory={false}
                    axisLine={false}
                    tickLine={false}
                    tick={{ fontSize: 10 }}
                    dy={8}
                    minTickGap={expanded ? 18 : 24}
                    ticks={xAxisTicks}
                    tickFormatter={(value) =>
                      plottedData.find((point) => point.date === value)?.month ??
                      formatTreasuryTick(String(value))
                    }
                  />
                  <YAxis
                    yAxisId="btc"
                    orientation="left"
                    axisLine={false}
                    tickLine={false}
                    tick={{ fontSize: 10, dx: 64 }}
                    tickMargin={8}
                    tickFormatter={(value) =>
                      hideSensitive ? "" : formatBtcAxis(Number(value))
                    }
                    width={2}
                  />
                  <YAxis
                    yAxisId="price"
                    orientation="right"
                    axisLine={false}
                    tickLine={false}
                    tick={{ fontSize: 10 }}
                    tickMargin={4}
                    tickFormatter={(value) =>
                      hideSensitive
                        ? ""
                        : formatFiatPrice(Number(value), fiatCurrency)
                    }
                    width={64}
                  />
                  <ZAxis
                    dataKey="eventSize"
                    range={[80, expanded ? 620 : 480]}
                  />
                  {expanded && selectedPoint && (
                    <ReferenceLine
                      yAxisId="btc"
                      x={selectedPoint.date}
                      stroke={chartColors.focus}
                      strokeDasharray="2 3"
                      strokeOpacity={0.5}
                      strokeWidth={1.5}
                    />
                  )}
                  <Tooltip
                    allowEscapeViewBox={{ x: false, y: true }}
                    content={
                      <TreasuryTooltip
                        hideSensitive={hideSensitive}
                        priceEur={fiatRate}
                        fiatCurrency={fiatCurrency}
                      />
                    }
                    cursor={{ strokeOpacity: 0.2 }}
                    isAnimationActive={false}
                    offset={32}
                    wrapperStyle={{
                      pointerEvents: "none",
                      zIndex: 30,
                    }}
                  />
                  {seriesVisible.primary && (
                    <Line
                      yAxisId="btc"
                      type="stepAfter"
                      dataKey="lineBalanceBtc"
                      name={legendItems[0]?.label}
                      stroke={primaryColor}
                      strokeWidth={activeSeries === "primary" ? 3 : 2.5}
                      strokeOpacity={
                        activeSeries === null || activeSeries === "primary"
                          ? 1
                          : 0.3
                      }
                      dot={false}
                      activeDot={expanded ? { r: 4 } : { r: 3 }}
                      connectNulls
                      isAnimationActive={false}
                    />
                  )}
                  {seriesVisible.price && (
                    <Line
                      yAxisId="price"
                      type="linear"
                      dataKey="lineBitcoinPriceEur"
                      name={legendItems[3]?.label}
                      stroke="#94a3b8"
                      strokeWidth={activeSeries === "price" ? 2.4 : 1.6}
                      strokeDasharray="3 5"
                      strokeOpacity={
                        activeSeries === null || activeSeries === "price" ? 0.72 : 0.2
                      }
                      dot={false}
                      activeDot={expanded ? { r: 3 } : { r: 2 }}
                      connectNulls
                      isAnimationActive={false}
                    />
                  )}
                  {seriesVisible.basis && (
                    <Line
                      yAxisId="price"
                      type="stepAfter"
                      dataKey="lineAvgCostEur"
                      name={legendItems[2]?.label}
                      connectNulls
                      stroke={secondaryColor}
                      strokeWidth={activeSeries === "basis" ? 3 : 2}
                      strokeDasharray="5 5"
                      strokeOpacity={
                        activeSeries === null || activeSeries === "basis"
                          ? 1
                          : 0.32
                      }
                      dot={false}
                      activeDot={expanded ? { r: 4 } : { r: 3 }}
                      isAnimationActive={false}
                    />
                  )}
                  {seriesVisible.events && (
                    <Scatter
                      yAxisId="btc"
                      data={visibleActivityMarkers}
                      dataKey="markerBalanceBtc"
                      name="Activity"
                      fill="transparent"
                      onClick={openActivityPointTransaction}
                      shape={(props) => (
                        <ActivityScatterDot
                          {...props}
                          activeSeries={activeSeries}
                          onOpenTransactionDetail={onOpenTransactionDetail}
                        />
                      )}
                      isAnimationActive={false}
                    />
                  )}
                  </ComposedChart>
                </ChartContainer>
                {plottedData.length > 3 && (
                  <ChartContainer
                    config={chartConfig}
                    className={cn(
                      "w-full overflow-visible",
                      expanded ? "h-[60px]" : "h-[74px]",
                    )}
                  >
                    <AreaChart
                      data={chartDisplayData}
                      margin={{
                        top: 0,
                        right: expanded ? 72 : 68,
                        bottom: 0,
                        left: expanded ? 52 : 48,
                      }}
                    >
                      <Brush
                        key={`treasury-brush-${expanded ? "expanded" : "compact"}-${brushRevision}`}
                        className="text-muted-foreground"
                        dataKey="date"
                        endIndex={effectiveBrushRange.endIndex}
                        fill="color-mix(in oklch, var(--muted) 70%, var(--background))"
                        height={expanded ? 60 : 74}
                        onClick={handleChartClick}
                        onDoubleClick={handleChartDoubleClick}
                        onMouseDownCapture={handleChartMouseDown}
                        onDragEnd={handleBrushChange}
                        padding={{ top: 8, right: 1, bottom: 8, left: 1 }}
                        startIndex={effectiveBrushRange.startIndex}
                        travellerWidth={10}
                        stroke={primaryColor}
                        tickFormatter={(value) =>
                          plottedData.find((point) => point.date === value)?.month ??
                          formatTreasuryTick(String(value))
                        }
                      >
                        <AreaChart>
                          <XAxis dataKey="date" hide />
                          <YAxis hide domain={["dataMin", "dataMax"]} />
                          <defs>
                            <linearGradient
                              id={brushGradientId}
                              x1="0"
                              y1="0"
                              x2="0"
                              y2="1"
                            >
                              <stop
                                offset="0%"
                                stopColor={primaryColor}
                                stopOpacity={0.28}
                              />
                              <stop
                                offset="100%"
                                stopColor={primaryColor}
                                stopOpacity={0.03}
                              />
                            </linearGradient>
                          </defs>
                          <Area
                            type="stepAfter"
                            dataKey="brushBalanceBtc"
                            stroke={primaryColor}
                            strokeWidth={1.35}
                            fill={`url(#${brushGradientId})`}
                            fillOpacity={1}
                            dot={false}
                            connectNulls
                            isAnimationActive={false}
                          />
                        </AreaChart>
                      </Brush>
                    </AreaChart>
                  </ChartContainer>
                )}
              </div>
              <div className="pointer-events-none flex items-center justify-center">
                <span className="rotate-90 whitespace-nowrap text-[10px] font-semibold text-muted-foreground">
                  BTC Price ({fiatCurrency})
                </span>
              </div>
            </div>
            {plottedData.length > 3 && (
              <p className="pt-1 text-center text-[10px] text-muted-foreground">
                Drag the timeline window or handles to adjust the timeframe; double-click to reset
              </p>
            )}
          </div>
          {expanded && (
            <div className="grid content-start gap-3">
              <PortfolioInspector
                point={selectedPoint}
                previousPoint={previousPoint}
                hideSensitive={hideSensitive}
                priceEur={fiatRate}
                fiatCurrency={fiatCurrency}
                chartCurrency={currency}
              />
            </div>
          )}
        </div>
      </div>
    );
  };

  return (
    <Dialog>
      {renderChartCard()}
      <DialogContent
        showCloseButton={false}
        className="max-w-[calc(100vw-1rem)] p-0 sm:max-w-[min(1500px,calc(100vw-1.5rem))]"
      >
        <DialogTitle className="sr-only">
          Expanded BTC activity chart
        </DialogTitle>
        {renderChartCard(true)}
      </DialogContent>
    </Dialog>
  );
};
