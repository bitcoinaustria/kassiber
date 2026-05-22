import { useNavigate } from "@tanstack/react-router";
import { Maximize2, RefreshCw, Settings, X } from "lucide-react";
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
import { Checkbox } from "@/components/ui/checkbox";
import { type ChartConfig, ChartContainer } from "@/components/ui/chart";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { formatBtc, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import {
  type OverviewSnapshot,
  type PortfolioPoint,
  type Tx as OverviewTx,
} from "@/mocks/seed";

import {
  blurClass,
  btcFromEur,
  flowForOverviewTx,
  formatDetailedPortfolioMoney,
  formatPortfolioMoney,
  latestPortfolioBalanceBtc,
  portfolioChartColors,
  satToBtc,
  statusLabels,
  transactionDetailHref,
  type TransactionStatus,
  useHoverHighlight,
  useResolvedColorMode,
} from "./shared";

type PortfolioChartPoint = {
  date: string;
  month: string;
  detailLabel: string;
  thisYear: number;
  prevYear?: number;
  balanceBtc: number;
  valueEur: number;
  costBasisEur: number;
  unrealizedEur: number;
};

type PortfolioChartMetric = "value" | "btc" | "basis" | "unrealized";
type ActivityFlow = "incoming" | "outgoing" | "swap" | "transfer" | "fee";
type TreasuryChartSeriesKey = "primary" | "price" | "basis" | "events";
type TreasurySeriesVisibility = Record<TreasuryChartSeriesKey, boolean>;
type TreasuryLegendItem = {
  key: TreasuryChartSeriesKey;
  label: string;
  color: string;
  dashed: boolean;
};

const DEFAULT_INCOMING_MARKER_MIN_BTC = 0.0025;
const DEFAULT_OUTGOING_MARKER_MIN_BTC = 0;
const ACTIVITY_MARKER_SLIDER_MAX_BTC = 1;
const ACTIVITY_MARKER_INPUT_STEP_BTC = 0.00000001;
const ACTIVITY_MARKER_SLIDER_MARKS = [0, 0.0025, 0.01, 0.1, 0.5, 1] as const;
const INCOMING_MARKER_MIN_PARAM = "incomingMinBtc";
const OUTGOING_MARKER_MIN_PARAM = "outgoingMinBtc";
const LEGACY_INCOMING_MARKER_MIN_PARAM = "incomingMin";
const LEGACY_OUTGOING_MARKER_MIN_PARAM = "outgoingMin";
const TREASURY_BRUSH_MIN_WINDOW_MS = (7 * 24 * 60 * 60 * 1000) / 3;
const TREASURY_BRUSH_MIN_INDEX_SPAN = 2;

const defaultTreasurySeriesVisibility: TreasurySeriesVisibility = {
  primary: true,
  price: true,
  basis: true,
  events: true,
};

type TreasuryChartPoint = PortfolioChartPoint & {
  bitcoinPriceEur: number;
  avgCostEur: number | null;
  lineBalanceBtc?: number;
  lineBitcoinPriceEur?: number;
  lineAvgCostEur?: number | null;
  brushBalanceBtc: number;
  reserveValueEur: number;
  activityBtc: number;
  activityCount: number;
  activityValueEur: number;
  eventPriceEur?: number;
  eventBalanceBtc?: number;
  eventSize: number;
  eventFlow?: ActivityFlow;
  eventSignedBtc?: number;
  eventFeeBtc?: number;
  eventFiatValueEur?: number;
  eventType?: OverviewTx["type"];
  eventAccount?: string;
  eventCounter?: string;
  eventTag?: string;
  eventStatus?: TransactionStatus;
  eventConfirmations?: number;
  eventId?: string;
  eventTransactionId?: string;
  sortTimeMs: number;
  isActivityEvent?: boolean;
};

type PortfolioChartMouseState = {
  activePayload?: Array<{ payload?: TreasuryChartPoint }>;
};

type TreasuryBrushRange = {
  startIndex: number;
  endIndex: number;
};

type TreasuryBrushChange = {
  startIndex?: number;
  endIndex?: number;
};

type ActivityScatterDotProps = {
  cx?: number;
  cy?: number;
  size?: number;
  payload?: TreasuryChartPoint;
  activeSeries: TreasuryChartSeriesKey | null;
};

type ActivityMarkerView = {
  activityPoints: TreasuryChartPoint[];
  chartDisplayData: TreasuryChartPoint[];
  visibleActivityMarkers: TreasuryChartPoint[];
};

type TreasuryActivityEvent = {
  tx: OverviewSnapshot["txs"][number];
  btc: number;
  signedBtc: number;
  feeBtc: number;
  occurredAt: Date;
  priceEur: number;
  valueEur: number;
  flow: ActivityFlow;
  volumeBtc: number;
  sequence: number;
};

const fullYearData = [
  { month: "Jan", thisYear: 42000, prevYear: 38000 },
  { month: "Feb", thisYear: 38000, prevYear: 45000 },
  { month: "Mar", thisYear: 52000, prevYear: 41000 },
  { month: "Apr", thisYear: 45000, prevYear: 48000 },
  { month: "May", thisYear: 58000, prevYear: 44000 },
  { month: "Jun", thisYear: 41000, prevYear: 52000 },
  { month: "Jul", thisYear: 55000, prevYear: 47000 },
  { month: "Aug", thisYear: 48000, prevYear: 53000 },
  { month: "Sep", thisYear: 62000, prevYear: 49000 },
  { month: "Oct", thisYear: 54000, prevYear: 58000 },
  { month: "Nov", thisYear: 67000, prevYear: 52000 },
  { month: "Dec", thisYear: 71000, prevYear: 61000 },
];

const fiveYearData = [
  { month: "2022", thisYear: 210000, prevYear: 178000 },
  { month: "2023", thisYear: 248000, prevYear: 205000 },
  { month: "2024", thisYear: 287000, prevYear: 244000 },
  { month: "2025", thisYear: 319000, prevYear: 276000 },
  { month: "2026", thisYear: 337000, prevYear: 291000 },
];

type TimePeriod = "30days" | "3months" | "ytd" | "1year" | "5years" | "all";

const periodLabels: Record<TimePeriod, string> = {
  "30days": "30 Days",
  "3months": "3 Months",
  ytd: "YTD",
  "1year": "1 Year",
  "5years": "5 Years",
  all: "All Time",
};

const periodShortLabels: Record<TimePeriod, string> = {
  "30days": "30D",
  "3months": "3M",
  ytd: "YTD",
  "1year": "1Y",
  "5years": "5Y",
  all: "All",
};

const periodKeys: TimePeriod[] = [
  "30days",
  "3months",
  "ytd",
  "1year",
  "5years",
  "all",
];

function normalizeTimePeriodParam(value: string | null): TimePeriod | null {
  if (!value) return null;
  const normalized = value.toLowerCase().replace(/[\s_-]/g, "");
  if (normalized === "30days" || normalized === "30day" || normalized === "30d") {
    return "30days";
  }
  if (
    normalized === "3months" ||
    normalized === "3month" ||
    normalized === "3mos" ||
    normalized === "3mo" ||
    normalized === "3m"
  ) {
    return "3months";
  }
  if (normalized === "ytd") return "ytd";
  if (
    normalized === "1year" ||
    normalized === "1years" ||
    normalized === "1yr" ||
    normalized === "1y"
  ) {
    return "1year";
  }
  if (
    normalized === "5years" ||
    normalized === "5year" ||
    normalized === "5yrs" ||
    normalized === "5yr" ||
    normalized === "5y"
  ) {
    return "5years";
  }
  if (normalized === "all" || normalized === "max") return "all";
  return null;
}

function initialTimePeriodFromUrl(): TimePeriod {
  if (typeof window === "undefined") return "ytd";
  const params = new URLSearchParams(window.location.search);
  return normalizeTimePeriodParam(params.get("period")) ?? "ytd";
}

function clampActivityMarkerMinimum(value: number) {
  if (!Number.isFinite(value)) return 0;
  const clamped = Math.max(value, 0);
  return (
    Math.round(clamped / ACTIVITY_MARKER_INPUT_STEP_BTC) *
    ACTIVITY_MARKER_INPUT_STEP_BTC
  );
}

function initialActivityMarkerMinimumFromUrl(
  param: string,
  fallback: number,
  legacyParam?: string,
) {
  if (typeof window === "undefined") return fallback;
  const params = new URLSearchParams(window.location.search);
  const rawValue = params.get(param);
  if (rawValue !== null) {
    const parsed = Number(rawValue);
    if (!Number.isFinite(parsed)) return fallback;
    return clampActivityMarkerMinimum(parsed);
  }
  const legacyValue = legacyParam ? params.get(legacyParam) : null;
  if (legacyValue === null) return fallback;
  const parsed = Number(legacyValue);
  if (!Number.isFinite(parsed)) return fallback;
  if (parsed <= 0) return fallback;
  return clampActivityMarkerMinimum(parsed);
}

function serializeActivityMarkerMinimum(value: number) {
  return clampActivityMarkerMinimum(value)
    .toFixed(8)
    .replace(/0+$/, "")
    .replace(/\.$/, "");
}

function formatEditableActivityMarkerMinimum(value: number) {
  const serialized = serializeActivityMarkerMinimum(value);
  const [, fraction = ""] = serialized.split(".");
  const precision = Math.min(8, Math.max(1, fraction.length + 1));
  return clampActivityMarkerMinimum(value).toFixed(precision);
}

function fallbackPortfolioData(
  data: Array<{ month: string; thisYear: number; prevYear: number }>,
  snapshot: OverviewSnapshot,
  metric: PortfolioChartMetric,
  currency: Currency,
  { densify }: { densify: boolean },
): PortfolioChartPoint[] {
  const scoped = densify
    ? expandFallbackYearData(data, snapshot.priceEur)
    : data;
  return scoped.map((point, index) => {
    const valueEur = point.thisYear;
    const costBasisEur = point.prevYear;
    const balanceBtc = btcFromEur(valueEur, snapshot.priceEur);
    return buildPortfolioChartPoint(
      {
        date: `fallback-${index}`,
        label: point.month,
        balanceBtc,
        valueEur,
        costBasisEur,
      },
      point.month,
      point.month,
      metric,
      currency,
    );
  });
}

function getDataForPeriod(
  period: TimePeriod,
  snapshot: OverviewSnapshot,
  metric: PortfolioChartMetric,
  currency: Currency,
  density: "compact" | "detailed",
): PortfolioChartPoint[] {
  const fallback = fallbackPortfolioData(
    period === "5years" ? fiveYearData : fullYearData,
    snapshot,
    metric,
    currency,
    { densify: period !== "5years" },
  );
  if (snapshot.portfolioSeries?.length) {
    const points = buildDatedPortfolioPoints(
      snapshot.portfolioSeries,
      period,
      metric,
      currency,
      density,
    );
    if (points.length) return points;
  }
  if (!snapshot.balanceSeries.some((value) => value !== 0)) {
    if (period === "30days") return fallback.slice(-4);
    if (period === "3months") return fallback.slice(-3);
    if (period === "ytd") return fallback.slice(0, 6);
    return fallback;
  }
  const labels =
    period === "5years"
      ? ["2022", "2023", "2024", "2025", "2026"]
      : [
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
        ];
  const points = snapshot.balanceSeries.map((btc, index) => {
    const isLatestPoint = index === snapshot.balanceSeries.length - 1;
    const value = isLatestPoint
      ? snapshot.fiat.eurBalance
      : btc * snapshot.priceEur;
    const basisShare =
      snapshot.fiat.eurBalance > 0
        ? value / snapshot.fiat.eurBalance
        : index / Math.max(1, snapshot.balanceSeries.length - 1);
    const label = labels[index % labels.length];
    return buildPortfolioChartPoint(
      {
        date: `series-${index}`,
        label,
        balanceBtc: btc,
        valueEur: value,
        costBasisEur:
          snapshot.fiat.eurCostBasis * Math.max(0, Math.min(1, basisShare)),
      },
      label,
      label,
      metric,
      currency,
    );
  });
  if (period === "30days") return points.slice(-4);
  if (period === "3months") return points.slice(-3);
  if (period === "ytd") {
    return points.slice(0, Math.max(1, new Date().getMonth() + 1));
  }
  if (period === "5years") {
    return points.filter((_, index) => index % 3 === 0).slice(-5);
  }
  return points;
}

function buildDatedPortfolioPoints(
  series: PortfolioPoint[],
  period: TimePeriod,
  metric: PortfolioChartMetric,
  currency: Currency,
  density: "compact" | "detailed",
): PortfolioChartPoint[] {
  const sorted = [...series].sort((a, b) => a.date.localeCompare(b.date));
  const latestDate = parseSeriesDate(sorted[sorted.length - 1]?.date);
  const filtered =
    period === "all"
      ? sorted
      : sorted.filter((point) =>
          isPointInPeriod(point.date, latestDate, period),
        );
  const scoped = filtered.length ? filtered : sorted.slice(-1);
  return samplePortfolioPoints(scoped, period, density).map((point) =>
    buildPortfolioChartPoint(
      point,
      formatPortfolioTick(point.date, period),
      formatPortfolioDetailLabel(point.date),
      metric,
      currency,
    ),
  );
}

function parseSeriesDate(value: string | undefined) {
  const parsed = value ? new Date(`${value.slice(0, 10)}T00:00:00Z`) : null;
  return parsed && !Number.isNaN(parsed.valueOf()) ? parsed : new Date();
}

function isPointInPeriod(
  value: string,
  latestDate: Date,
  period: TimePeriod,
) {
  const pointDate = parseSeriesDate(value);
  if (period === "ytd") {
    return pointDate.getUTCFullYear() === latestDate.getUTCFullYear();
  }
  const start = new Date(latestDate);
  if (period === "30days") {
    start.setUTCDate(start.getUTCDate() - 30);
  } else if (period === "3months") {
    start.setUTCMonth(start.getUTCMonth() - 3);
  } else if (period === "1year") {
    start.setUTCFullYear(start.getUTCFullYear() - 1);
  } else if (period === "5years") {
    start.setUTCFullYear(start.getUTCFullYear() - 5);
  } else {
    return true;
  }
  return pointDate >= start && pointDate <= latestDate;
}

function buildPortfolioChartPoint(
  point: Pick<
    PortfolioPoint,
    "date" | "label" | "balanceBtc" | "valueEur" | "costBasisEur"
  >,
  month: string,
  detailLabel: string,
  metric: PortfolioChartMetric,
  currency: Currency,
): PortfolioChartPoint {
  const unrealizedEur = point.valueEur - point.costBasisEur;
  const chartCurrency = chartCurrencyForMetric(metric, currency);
  const thisYear =
    metric === "btc"
      ? point.balanceBtc
      : metric === "basis"
        ? point.costBasisEur
        : metric === "unrealized"
          ? unrealizedEur
          : chartCurrency === "btc"
            ? point.balanceBtc
            : point.valueEur;
  const prevYear =
    metric === "value" && chartCurrency === "eur"
      ? point.costBasisEur
      : undefined;
  return {
    date: point.date,
    month,
    detailLabel,
    thisYear,
    prevYear,
    balanceBtc: point.balanceBtc,
    valueEur: point.valueEur,
    costBasisEur: point.costBasisEur,
    unrealizedEur,
  };
}

function chartCurrencyForMetric(
  metric: PortfolioChartMetric,
  currency: Currency,
): Currency {
  if (metric === "btc") return "btc";
  if (metric === "value") return currency;
  return "eur";
}

function parseIsoDayDate(value: string | undefined) {
  if (!value || !/^\d{4}-\d{2}-\d{2}/.test(value)) return null;
  const parsed = new Date(`${value.slice(0, 10)}T00:00:00Z`);
  return Number.isNaN(parsed.valueOf()) ? null : parsed;
}

function parseOverviewTxDate(value: string | undefined) {
  if (!value) return null;
  const normalized = value.includes("T") ? value : `${value.replace(" ", "T")}Z`;
  const parsed = new Date(normalized);
  return Number.isNaN(parsed.valueOf()) ? null : parsed;
}

function treasurySortTime(value: string | undefined) {
  if (!value) return null;
  const key = value.split("#")[0] ?? value;
  const parsed = key.includes("T") ? new Date(key) : parseIsoDayDate(key);
  return parsed && !Number.isNaN(parsed.valueOf()) ? parsed.valueOf() : null;
}

function fullTreasuryBrushRange(dataLength: number): TreasuryBrushRange {
  return {
    startIndex: 0,
    endIndex: Math.max(0, dataLength - 1),
  };
}

function sameTreasuryBrushRange(
  a: TreasuryBrushRange | null,
  b: TreasuryBrushRange,
) {
  return a?.startIndex === b.startIndex && a.endIndex === b.endIndex;
}

function clampTreasuryBrushIndex(index: number | undefined, dataLength: number) {
  if (!Number.isFinite(index)) return 0;
  return Math.max(0, Math.min(dataLength - 1, Math.round(index ?? 0)));
}

function treasuryBrushTime(point: TreasuryChartPoint | undefined) {
  if (!point) return null;
  if (Number.isFinite(point.sortTimeMs) && point.sortTimeMs > 0) {
    return point.sortTimeMs;
  }
  return treasurySortTime(point.date);
}

function findTreasuryBrushIndexAtOrAfter(
  data: TreasuryChartPoint[],
  targetTime: number,
) {
  const index = data.findIndex((point) => {
    const time = treasuryBrushTime(point);
    return time !== null && time >= targetTime;
  });
  return index === -1 ? data.length - 1 : index;
}

function findTreasuryBrushIndexAtOrBefore(
  data: TreasuryChartPoint[],
  targetTime: number,
) {
  for (let index = data.length - 1; index >= 0; index -= 1) {
    const time = treasuryBrushTime(data[index]);
    if (time !== null && time <= targetTime) return index;
  }
  return 0;
}

function treasuryBrushMinIndexSpan(data: TreasuryChartPoint[]) {
  if (data.length <= 1) return 0;
  const firstTime = treasuryBrushTime(data[0]);
  const lastTime = treasuryBrushTime(data[data.length - 1]);
  if (
    firstTime === null ||
    lastTime === null ||
    lastTime <= firstTime
  ) {
    return 1;
  }
  const proportionalSpan = Math.ceil(
    ((data.length - 1) * TREASURY_BRUSH_MIN_WINDOW_MS) /
      (lastTime - firstTime),
  );
  return Math.max(
    1,
    Math.min(
      data.length - 1,
      Math.max(proportionalSpan, TREASURY_BRUSH_MIN_INDEX_SPAN),
    ),
  );
}

function normalizeTreasuryBrushRange(
  data: TreasuryChartPoint[],
  next: TreasuryBrushChange | TreasuryBrushRange | null,
  previous: TreasuryBrushRange | null,
): TreasuryBrushRange {
  if (data.length <= 1) return fullTreasuryBrushRange(data.length);

  const fullRange = fullTreasuryBrushRange(data.length);
  const requestedStart = clampTreasuryBrushIndex(
    next?.startIndex ?? previous?.startIndex ?? fullRange.startIndex,
    data.length,
  );
  const requestedEnd = clampTreasuryBrushIndex(
    next?.endIndex ?? previous?.endIndex ?? fullRange.endIndex,
    data.length,
  );
  const startIndex = Math.min(requestedStart, requestedEnd);
  const endIndex = Math.max(requestedStart, requestedEnd);
  const startTime = treasuryBrushTime(data[startIndex]);
  const endTime = treasuryBrushTime(data[endIndex]);
  const firstTime = treasuryBrushTime(data[0]);
  const lastTime = treasuryBrushTime(data[data.length - 1]);

  if (
    startTime === null ||
    endTime === null ||
    firstTime === null ||
    lastTime === null
  ) {
    return { startIndex, endIndex };
  }

  if (lastTime - firstTime <= TREASURY_BRUSH_MIN_WINDOW_MS) {
    return fullRange;
  }

  const minIndexSpan = treasuryBrushMinIndexSpan(data);
  if (
    endTime - startTime >= TREASURY_BRUSH_MIN_WINDOW_MS &&
    endIndex - startIndex >= minIndexSpan
  ) {
    return { startIndex, endIndex };
  }

  const startChanged = previous ? startIndex !== previous.startIndex : false;
  const endChanged = previous ? endIndex !== previous.endIndex : false;
  const keepEndFixed =
    startChanged && !endChanged
      ? true
      : endChanged && !startChanged
        ? false
        : endIndex >= data.length - 1 || startIndex > data.length / 2;

  if (keepEndFixed) {
    return {
      startIndex: Math.min(
        findTreasuryBrushIndexAtOrBefore(
          data,
          endTime - TREASURY_BRUSH_MIN_WINDOW_MS,
        ),
        Math.max(0, endIndex - minIndexSpan),
      ),
      endIndex,
    };
  }

  return {
    startIndex,
    endIndex: Math.max(
      findTreasuryBrushIndexAtOrAfter(
        data,
        startTime + TREASURY_BRUSH_MIN_WINDOW_MS,
      ),
      Math.min(data.length - 1, startIndex + minIndexSpan),
    ),
  };
}

function rawTreasuryBrushRange(
  dataLength: number,
  next: TreasuryBrushChange | TreasuryBrushRange | null,
  previous: TreasuryBrushRange,
): TreasuryBrushRange {
  if (dataLength <= 1) return fullTreasuryBrushRange(dataLength);
  const requestedStart = clampTreasuryBrushIndex(
    next?.startIndex ?? previous.startIndex,
    dataLength,
  );
  const requestedEnd = clampTreasuryBrushIndex(
    next?.endIndex ?? previous.endIndex,
    dataLength,
  );
  return {
    startIndex: Math.min(requestedStart, requestedEnd),
    endIndex: Math.max(requestedStart, requestedEnd),
  };
}

function formatTreasuryTick(value: string) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return value;
  return parsed.toLocaleDateString("en-US", {
    month: "short",
    year: "2-digit",
    timeZone: "UTC",
  });
}

function formatTreasuryDetailDate(value: string) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return value;
  return parsed.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

function formatEurPrice(eur: number) {
  if (Math.abs(eur) >= 100_000) return `${Math.round(eur).toLocaleString("en-US")} EUR`;
  return `${eur.toLocaleString("en-US", {
    maximumFractionDigits: 0,
  })} EUR`;
}

function treasuryPrimaryValue(point: TreasuryChartPoint) {
  return point.balanceBtc;
}

function formatBtcAxis(value: number) {
  const precision = Math.abs(value) >= 10 ? 0 : Math.abs(value) >= 1 ? 2 : 3;
  return formatBtc(value, { precision }).replace("₿ ", "₿");
}

function activityMarkerSliderValue(value: number) {
  const clamped = Math.min(
    clampActivityMarkerMinimum(value),
    ACTIVITY_MARKER_SLIDER_MAX_BTC,
  );
  let closestIndex = 0;
  let closestDistance = Number.POSITIVE_INFINITY;
  ACTIVITY_MARKER_SLIDER_MARKS.forEach((mark, index) => {
    const distance = Math.abs(mark - clamped);
    if (distance < closestDistance) {
      closestDistance = distance;
      closestIndex = index;
    }
  });
  return closestIndex;
}

function ActivityMarkerSlider({
  id,
  label,
  value,
  color,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  color: string;
  onChange: (value: number) => void;
}) {
  const marksId = `${id}-marks`;
  return (
    <div className="mt-3 space-y-2">
      <input
        aria-label={label}
        className="h-2 w-full cursor-pointer"
        list={marksId}
        min={0}
        max={ACTIVITY_MARKER_SLIDER_MARKS.length - 1}
        step={1}
        type="range"
        value={activityMarkerSliderValue(value)}
        style={{ accentColor: color }}
        onChange={(event) =>
          onChange(ACTIVITY_MARKER_SLIDER_MARKS[Number(event.currentTarget.value)] ?? 0)
        }
      />
      <datalist id={marksId}>
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark, index) => (
          <option key={mark} value={index} label={serializeActivityMarkerMinimum(mark)} />
        ))}
      </datalist>
      <div className="flex justify-between text-[10px] text-muted-foreground">
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark) => (
          <span key={mark} className="tabular-nums">
            {serializeActivityMarkerMinimum(mark)}
          </span>
        ))}
      </div>
    </div>
  );
}

function compactEventId(value: string | undefined) {
  if (!value) return null;
  if (value.length <= 18) return value;
  return `${value.slice(0, 8)}…${value.slice(-6)}`;
}

function statusForOverviewTx(tx: OverviewTx): TransactionStatus {
  if (tx.internal) return "pending";
  if (tx.conf > 0) return "confirmed";
  return tx.tag.toLowerCase().includes("review") ? "review" : "pending";
}

function activityFlowForTx(tx: OverviewTx): ActivityFlow {
  if (tx.type === "Fee") return "fee";
  return flowForOverviewTx(tx);
}

const activityFlowLabels: Record<ActivityFlow, string> = {
  incoming: "Received",
  outgoing: "Spent",
  swap: "Swap",
  transfer: "Transfer",
  fee: "Fee",
};

const activityFlowColors: Record<ActivityFlow, string> = {
  incoming: "#34d399",
  outgoing: "#f87171",
  swap: "#38bdf8",
  transfer: "#f59e0b",
  fee: "#a1a1aa",
};

const activityFlowKeys: ActivityFlow[] = [
  "incoming",
  "outgoing",
  "swap",
  "transfer",
  "fee",
];

function ActivityScatterDot({
  cx,
  cy,
  size,
  payload,
  activeSeries,
}: ActivityScatterDotProps) {
  const navigate = useNavigate();
  if (
    typeof cx !== "number" ||
    typeof cy !== "number" ||
    !payload?.eventFlow
  ) {
    return null;
  }

  const normalizedSize = typeof size === "number" ? size : 80;
  const radius = Math.max(3, Math.sqrt(normalizedSize / Math.PI));
  const transactionId = payload.eventTransactionId ?? payload.eventId;
  const openTransactionDetail = () => {
    if (!transactionId) return;
    void navigate({ to: transactionDetailHref(transactionId) });
  };
  const handleClick = (event: React.MouseEvent<SVGCircleElement>) => {
    if (!transactionId) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetail();
  };
  const handleKeyDown = (event: React.KeyboardEvent<SVGCircleElement>) => {
    if (!transactionId || (event.key !== "Enter" && event.key !== " ")) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetail();
  };

  return (
    <circle
      className="recharts-scatter-symbol"
      cx={cx}
      cy={cy}
      r={radius}
      aria-label={
        transactionId
          ? `Open ${activityFlowLabels[payload.eventFlow]} transaction`
          : undefined
      }
      fill={activityFlowColors[payload.eventFlow]}
      fillOpacity={
        activeSeries === null || activeSeries === "events" ? 0.92 : 0.28
      }
      focusable={transactionId ? true : false}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      onMouseDown={(event) => event.preventDefault()}
      role={transactionId ? "button" : undefined}
      stroke="var(--background)"
      strokeWidth={2.5}
      style={{
        cursor: transactionId ? "pointer" : "default",
      }}
      tabIndex={transactionId ? 0 : -1}
    />
  );
}

function activityTxs(snapshot: OverviewSnapshot): TreasuryActivityEvent[] {
  const txs = snapshot.activityTxs?.length ? snapshot.activityTxs : snapshot.txs;
  return txs
    .flatMap((tx, sequence) => {
      const occurredAt = parseOverviewTxDate(tx.occurredAt ?? tx.date);
      if (tx.excluded || !occurredAt) return [];
      const signedBtc = satToBtc(tx.amountSat);
      const btc = Math.abs(signedBtc);
      const feeBtc = satToBtc(Math.abs(tx.feeSat ?? 0));
      const flow = activityFlowForTx(tx);
      const pairedVolume = Math.max(
        btc,
        satToBtc(Math.abs(tx.pair?.outAmountSat ?? 0)),
        satToBtc(Math.abs(tx.pair?.inAmountSat ?? 0)),
      );
      const volumeBtc =
        flow === "fee" ? Math.max(btc, feeBtc) : flow === "swap" ? pairedVolume : btc;
      if (volumeBtc <= 0 && feeBtc <= 0) return [];
      const valueEur = Math.abs(tx.eur ?? 0);
      const priceEur =
        valueEur > 0 && btc > 0 ? valueEur / btc : tx.rate ?? snapshot.priceEur;
      return [
        {
          tx,
          btc,
          signedBtc,
          feeBtc,
          occurredAt,
          priceEur,
          valueEur,
          flow,
          volumeBtc,
          sequence,
        },
      ];
    })
    .sort((a, b) => {
      const timeDelta = a.occurredAt.valueOf() - b.occurredAt.valueOf();
      return timeDelta || a.sequence - b.sequence;
    });
}

function isActivityInTreasuryPeriod(
  event: TreasuryActivityEvent,
  latestDate: Date,
  period: TimePeriod,
) {
  if (period === "all") return true;
  if (period === "ytd") {
    return event.occurredAt.getUTCFullYear() === latestDate.getUTCFullYear();
  }
  const start = new Date(latestDate);
  if (period === "30days") {
    start.setUTCDate(start.getUTCDate() - 30);
  } else if (period === "3months") {
    start.setUTCMonth(start.getUTCMonth() - 3);
  } else if (period === "1year") {
    start.setUTCFullYear(start.getUTCFullYear() - 1);
  } else if (period === "5years") {
    start.setUTCFullYear(start.getUTCFullYear() - 5);
  }
  return event.occurredAt >= start && event.occurredAt <= latestDate;
}

function activityDateKey(event: TreasuryActivityEvent) {
  return `${event.occurredAt.toISOString()}#${event.tx.id || event.sequence}`;
}

function buildTreasuryBasePoint(
  point: PortfolioChartPoint,
  snapshot: OverviewSnapshot,
): TreasuryChartPoint {
  const bitcoinPriceEur =
    point.balanceBtc > 0 ? point.valueEur / point.balanceBtc : snapshot.priceEur;
  const avgCostEur =
    point.balanceBtc > 0 && point.costBasisEur > 0
      ? point.costBasisEur / point.balanceBtc
      : null;
  return {
    ...point,
    bitcoinPriceEur,
    avgCostEur,
    lineBalanceBtc: point.balanceBtc,
    lineBitcoinPriceEur: bitcoinPriceEur,
    lineAvgCostEur: avgCostEur,
    brushBalanceBtc: point.balanceBtc,
    reserveValueEur: point.valueEur,
    activityBtc: 0,
    activityCount: 0,
    activityValueEur: 0,
    eventPriceEur: undefined,
    eventBalanceBtc: undefined,
    eventSize: 0,
    sortTimeMs: treasurySortTime(point.date) ?? 0,
    month: point.month || formatTreasuryTick(point.date),
    detailLabel: point.detailLabel || formatTreasuryDetailDate(point.date),
  };
}

function nearestTreasuryAnchor(
  points: TreasuryChartPoint[],
  event: TreasuryActivityEvent,
) {
  const eventTime = event.occurredAt.valueOf();
  const previous = [...points]
    .reverse()
    .find((point) => point.sortTimeMs <= eventTime);
  return previous ?? points[0] ?? null;
}

function buildTreasuryActivityPoint(
  event: TreasuryActivityEvent,
  anchor: TreasuryChartPoint | null,
  snapshot: OverviewSnapshot,
): TreasuryChartPoint {
  const balanceBtc = anchor?.balanceBtc ?? latestPortfolioBalanceBtc(snapshot);
  const costBasisEur = anchor?.costBasisEur ?? snapshot.fiat.eurCostBasis;
  const valueEur =
    balanceBtc > 0
      ? balanceBtc * event.priceEur
      : anchor?.valueEur ?? snapshot.fiat.eurBalance;
  const avgCostEur =
    balanceBtc > 0 && costBasisEur > 0 ? costBasisEur / balanceBtc : null;
  const date = activityDateKey(event);
  return {
    date,
    month: formatTreasuryTick(date),
    detailLabel: formatTreasuryDetailDate(date),
    thisYear: valueEur,
    prevYear: costBasisEur,
    balanceBtc,
    valueEur,
    costBasisEur,
    unrealizedEur: valueEur - costBasisEur,
    bitcoinPriceEur: event.priceEur,
    avgCostEur,
    lineBalanceBtc: balanceBtc,
    lineBitcoinPriceEur: event.priceEur,
    lineAvgCostEur: avgCostEur,
    brushBalanceBtc: balanceBtc,
    reserveValueEur: valueEur,
    activityBtc: event.volumeBtc,
    activityCount: 1,
    activityValueEur: event.valueEur,
    eventPriceEur: event.priceEur,
    eventBalanceBtc: balanceBtc,
    eventSize: Math.max(event.volumeBtc, event.feeBtc),
    eventFlow: event.flow,
    eventSignedBtc: event.signedBtc,
    eventFeeBtc: event.feeBtc,
    eventFiatValueEur: event.valueEur,
    eventType: event.tx.type,
    eventAccount: event.tx.account,
    eventCounter: event.tx.counter,
    eventTag: event.tx.tag,
    eventStatus: statusForOverviewTx(event.tx),
    eventConfirmations: event.tx.conf,
    eventId: event.tx.explorerId ?? event.tx.externalId ?? event.tx.id,
    eventTransactionId: event.tx.id,
    sortTimeMs: event.occurredAt.valueOf() + event.sequence / 1000,
    isActivityEvent: true,
  };
}

function enrichTreasuryChartData(
  points: PortfolioChartPoint[],
  snapshot: OverviewSnapshot,
  period: TimePeriod,
): TreasuryChartPoint[] {
  const basePoints = points.map((point) => buildTreasuryBasePoint(point, snapshot));
  const events = activityTxs(snapshot);
  const candidateTimes = [
    ...basePoints.map((point) => point.sortTimeMs).filter((time) => time > 0),
    ...events.map((event) => event.occurredAt.valueOf()),
  ];
  const latestTime = candidateTimes.length ? Math.max(...candidateTimes) : Date.now();
  const latestDate = new Date(latestTime);
  const eventPoints = events
    .filter((event) => isActivityInTreasuryPeriod(event, latestDate, period))
    .map((event) =>
      buildTreasuryActivityPoint(
        event,
        nearestTreasuryAnchor(basePoints, event),
        snapshot,
      ),
    );

  return [...basePoints, ...eventPoints].sort((a, b) => {
    const timeDelta = a.sortTimeMs - b.sortTimeMs;
    if (timeDelta !== 0) return timeDelta;
    if (a.isActivityEvent === b.isActivityEvent) return a.date.localeCompare(b.date);
    return a.isActivityEvent ? -1 : 1;
  });
}

function buildTreasuryChartStats(points: TreasuryChartPoint[]) {
  if (!points.length) return null;
  const firstPoint = points[0];
  const lastPoint = points[points.length - 1] ?? firstPoint;
  const first = treasuryPrimaryValue(firstPoint);
  const last = treasuryPrimaryValue(lastPoint);
  const delta = last - first;
  const highPoint = points.reduce((highest, point) =>
    treasuryPrimaryValue(point) > treasuryPrimaryValue(highest)
      ? point
      : highest,
  );
  const lowPoint = points.reduce((lowest, point) =>
    treasuryPrimaryValue(point) < treasuryPrimaryValue(lowest)
      ? point
      : lowest,
  );
  return {
    first,
    last,
    delta,
    pct: first !== 0 ? (delta / Math.abs(first)) * 100 : null,
    highPoint,
    lowPoint,
  };
}

function activityMarkerView(
  plottedData: TreasuryChartPoint[],
  showEvents: boolean,
  markerMinimumForPoint: (point: TreasuryChartPoint) => number,
): ActivityMarkerView {
  const activityPoints = plottedData.filter((point) => point.isActivityEvent);
  const visibleActivityMarkers = activityPoints.filter(
    (point) =>
      showEvents &&
      (point.eventSize || point.activityBtc) >= markerMinimumForPoint(point),
  );
  const visibleActivityMarkerIds = new Set(
    visibleActivityMarkers.map((point) => point.date),
  );
  const chartDisplayData = plottedData.filter(
    (point) => !point.isActivityEvent || visibleActivityMarkerIds.has(point.date),
  );
  return {
    activityPoints,
    chartDisplayData,
    visibleActivityMarkers,
  };
}

function expandFallbackYearData(
  data: Array<{ month: string; thisYear: number; prevYear: number }>,
  priceEur: number,
) {
  return data.flatMap((point, index) => {
    if (index === 0) return [point];
    const previous = data[index - 1];
    const swing = Math.sin(index * 2.43) * 0.06;
    const midValue =
      previous.thisYear + (point.thisYear - previous.thisYear) * 0.52;
    const midBasis =
      previous.prevYear + (point.prevYear - previous.prevYear) * 0.52;
    return [
      {
        month: `${point.month} · 1`,
        thisYear: Math.max(0, midValue * (1 + swing)),
        prevYear: Math.max(0, midBasis),
      },
      point,
    ];
  }).map((point) => ({
    ...point,
    thisYear: Math.round(point.thisYear * 100) / 100,
    prevYear: Math.round(point.prevYear * 100) / 100,
    balanceBtc: btcFromEur(point.thisYear, priceEur),
  }));
}

function samplePortfolioPoints(
  points: PortfolioPoint[],
  period: TimePeriod,
  density: "compact" | "detailed",
) {
  const maxPoints =
    density === "detailed"
      ? period === "all" || period === "5years"
        ? 720
        : 420
      : period === "30days"
        ? 90
        : period === "3months"
          ? 120
          : 180;
  if (points.length <= maxPoints) return points;
  const step = Math.ceil((points.length - 2) / Math.max(1, maxPoints - 2));
  return points.filter(
    (_, index) =>
      index === 0 || index === points.length - 1 || index % step === 0,
  );
}

function formatPortfolioTick(value: string, period: TimePeriod) {
  const date = parseSeriesDate(value);
  if (period === "5years") {
    return date.toLocaleDateString("en-US", {
      month: "short",
      year: "2-digit",
      timeZone: "UTC",
    });
  }
  if (period === "30days" || period === "3months") {
    return date.toLocaleDateString("en-US", {
      month: "short",
      day: "numeric",
      timeZone: "UTC",
    });
  }
  return date.toLocaleDateString("en-US", {
    month: "short",
    year: "2-digit",
    timeZone: "UTC",
  });
}

function portfolioTickBucket(point: PortfolioChartPoint, period: TimePeriod) {
  if (point.date.startsWith("fallback-") || point.date.startsWith("series-")) {
    return point.month;
  }
  if (period === "30days" || period === "3months") return point.month;
  if (period === "5years" || period === "all") {
    const date = parseSeriesDate(point.date);
    return `${date.getUTCFullYear()}-${Math.floor(date.getUTCMonth() / 3)}`;
  }
  const date = parseSeriesDate(point.date);
  return `${date.getUTCFullYear()}-${date.getUTCMonth()}`;
}

function portfolioAxisTicks(
  points: PortfolioChartPoint[],
  period: TimePeriod,
  expanded: boolean,
) {
  const maxTicks = expanded ? 10 : 8;
  const ticks: string[] = [];
  const seenBuckets = new Set<string>();
  for (const point of points) {
    const bucket = portfolioTickBucket(point, period);
    if (seenBuckets.has(bucket)) continue;
    seenBuckets.add(bucket);
    ticks.push(point.date);
  }
  if (ticks.length <= maxTicks) return ticks;
  const step = Math.ceil((ticks.length - 1) / Math.max(1, maxTicks - 1));
  return ticks.filter(
    (_, index) => index === 0 || index === ticks.length - 1 || index % step === 0,
  );
}

function formatPortfolioDetailLabel(value: string) {
  const date = parseSeriesDate(value);
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}


interface TreasuryTooltipPayload {
  dataKey?: string | number;
  value?: number | string;
  payload?: TreasuryChartPoint;
}

interface TreasuryTooltipProps {
  active?: boolean;
  payload?: TreasuryTooltipPayload[];
  label?: string | number;
  hideSensitive: boolean;
  priceEur: number;
}

function TreasuryTooltip({
  active,
  payload,
  label,
  hideSensitive,
  priceEur,
}: TreasuryTooltipProps) {
  if (!active || !payload?.length) return null;

  const point =
    payload.find((p) => p.payload?.isActivityEvent)?.payload ??
    payload.find((p) => p.payload)?.payload;
  if (!point) return null;

  const unrealizedPct = point.costBasisEur
    ? (point.unrealizedEur / Math.abs(point.costBasisEur)) * 100
    : 0;
  const eventFlow = point.eventFlow;
  const hasEvent = point.isActivityEvent && eventFlow !== undefined;
  const eventTone =
    eventFlow === "incoming" || eventFlow === "swap"
      ? "good"
      : eventFlow === "outgoing" || eventFlow === "fee"
        ? "bad"
        : "neutral";
  const eventAmount =
    eventFlow === "swap"
      ? `${formatBtc(point.activityBtc, { precision: 8 })} volume`
      : eventFlow === "fee"
        ? formatBtc(-(point.eventFeeBtc || point.activityBtc), {
            precision: 8,
            sign: true,
          })
        : formatBtc(point.eventSignedBtc ?? 0, {
            precision: 8,
            sign: true,
          });
  const eventId = compactEventId(point.eventId);

  if (hasEvent) {
    return (
      <div className="min-w-[280px] max-w-[320px] rounded-lg border border-border bg-popover p-3 text-xs shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-1.5">
              <span
                className="size-2.5 rounded-full"
                style={{ backgroundColor: activityFlowColors[eventFlow] }}
                aria-hidden="true"
              />
              <span className="font-semibold text-foreground">
                {activityFlowLabels[eventFlow]}
              </span>
              {point.eventType && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {point.eventType}
                </span>
              )}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              {point.detailLabel ?? label}
            </p>
          </div>
          <span
            className={cn(
              "shrink-0 text-right font-semibold tabular-nums",
              eventTone === "good" && "text-emerald-500",
              eventTone === "bad" && "text-[var(--color-accent)]",
              blurClass(hideSensitive),
            )}
          >
            {eventAmount}
          </span>
        </div>

        <div className="mt-3 space-y-1.5">
          {point.eventAccount && (
            <TooltipMetricRow
              label="Source"
              value={point.eventAccount}
              hidden={false}
            />
          )}
          {point.eventCounter && (
            <TooltipMetricRow
              label="Counterparty"
              value={point.eventCounter}
              hidden={false}
            />
          )}
          <TooltipMetricRow
            label="Fiat value"
            value={formatPortfolioMoney(point.eventFiatValueEur ?? 0, priceEur, "eur")}
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="BTC price"
            value={formatEurPrice(point.bitcoinPriceEur)}
            hidden={hideSensitive}
          />
          {(point.eventFeeBtc ?? 0) > 0 && (
            <TooltipMetricRow
              label="Fee"
              value={formatBtc(point.eventFeeBtc ?? 0, { precision: 8 })}
              hidden={hideSensitive}
            />
          )}
          <TooltipMetricRow
            label="Position after"
            value={formatBtc(point.balanceBtc, { precision: 8 })}
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="Avg basis after"
            value={
              point.avgCostEur === null ? "—" : formatEurPrice(point.avgCostEur)
            }
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="Status"
            value={
              point.eventStatus === "confirmed"
                ? `${point.eventConfirmations?.toLocaleString("en-US") ?? 0} confirmations`
                : point.eventStatus
                  ? statusLabels[point.eventStatus]
                  : "Unknown"
            }
            hidden={false}
          />
          {(point.eventTag || eventId) && (
            <div className="flex flex-wrap gap-1.5 pt-1">
              {point.eventTag && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {point.eventTag}
                </span>
              )}
              {eventId && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                  {eventId}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="min-w-[220px] rounded-lg border border-border bg-popover p-2.5 text-xs shadow-lg">
      <p className="mb-2 font-medium text-foreground">
        {point.detailLabel ?? label}
      </p>
      <div className="space-y-1.5">
        <TooltipMetricRow
          label="BTC balance"
          value={formatBtc(point.balanceBtc, { precision: 8 })}
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="BTC price"
          value={formatEurPrice(point.bitcoinPriceEur)}
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="Avg basis"
          value={
            point.avgCostEur === null ? "—" : formatEurPrice(point.avgCostEur)
          }
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="Unrealized"
          value={`${point.unrealizedEur >= 0 ? "+ " : "− "}${formatPortfolioMoney(
            Math.abs(point.unrealizedEur),
            priceEur,
            "eur",
          )} (${unrealizedPct >= 0 ? "+" : "−"}${Math.abs(unrealizedPct).toFixed(
            1,
          )}%)`}
          tone={point.unrealizedEur >= 0 ? "good" : "bad"}
          hidden={hideSensitive}
        />
      </div>
    </div>
  );
}

function TooltipMetricRow({
  label,
  value,
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
  tone?: "good" | "bad" | "neutral";
  hidden: boolean;
}) {
  return (
    <div className="flex items-baseline justify-between gap-4">
      <span className="text-muted-foreground">{label}</span>
      <span
        className={cn(
          "font-medium tabular-nums",
          tone === "good" && "text-emerald-500",
          tone === "bad" && "text-[var(--color-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </span>
    </div>
  );
}

function ChartStat({
  label,
  value,
  detail,
  prefix = "",
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
  detail?: string;
  prefix?: string;
  tone?: "good" | "bad" | "neutral";
  hidden: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md bg-background/70 px-2.5 py-2">
      <div className="text-[10px] font-medium text-muted-foreground">
        {label}
      </div>
      <div
        className={cn(
          "mt-0.5 truncate text-sm font-semibold tabular-nums",
          tone === "good" && "text-emerald-600 dark:text-emerald-400",
          tone === "bad" && "text-[var(--color-accent)]",
          blurClass(hidden),
        )}
      >
        {prefix}
        {value}
      </div>
      {detail && (
        <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
          {detail}
        </div>
      )}
    </div>
  );
}

function PortfolioInspector({
  point,
  previousPoint,
  hideSensitive,
  priceEur,
  chartCurrency,
}: {
  point: PortfolioChartPoint | null;
  previousPoint: PortfolioChartPoint | null;
  hideSensitive: boolean;
  priceEur: number;
  chartCurrency: Currency;
}) {
  const isBtc = chartCurrency === "btc";
  const priorValueEur = previousPoint?.valueEur ?? point?.valueEur ?? 0;
  const pointValueEur = point?.valueEur ?? 0;
  const eurDelta = pointValueEur - priorValueEur;
  const priorBtc = previousPoint?.balanceBtc ?? point?.balanceBtc ?? 0;
  const btcDelta =
    point && previousPoint ? point.balanceBtc - previousPoint.balanceBtc : 0;
  const primaryDelta = isBtc ? btcDelta : eurDelta;
  const primaryPrior = isBtc ? priorBtc : priorValueEur;
  const deltaPct = previousPoint && primaryPrior
    ? (primaryDelta / Math.abs(primaryPrior)) * 100
    : null;
  const secondaryDelta = isBtc ? eurDelta : btcDelta;
  const secondaryLabel = isBtc
    ? `${secondaryDelta >= 0 ? "+" : "−"}${formatPortfolioMoney(
        Math.abs(secondaryDelta),
        priceEur,
        "eur",
      )}`
    : `${secondaryDelta >= 0 ? "+" : "−"}${formatBtc(
        Math.abs(secondaryDelta),
        { precision: 8 },
      )}`;

  return (
    <aside className="flex min-h-0 flex-col gap-3 rounded-lg border bg-background/65 p-3 xl:max-h-[min(64vh,620px)] xl:overflow-y-auto">
      <div>
        <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          Selected point
        </p>
        <p className="mt-1 text-sm font-semibold">
          {point?.detailLabel ?? "No point selected"}
        </p>
      </div>

      <div className="grid gap-2">
        <InspectorMetric
          label="Value"
          value={formatPortfolioMoney(point?.valueEur ?? 0, priceEur, "eur")}
          hidden={hideSensitive}
        />
        <InspectorMetric
          label="BTC"
          value={formatBtc(point?.balanceBtc ?? 0, { precision: 8 })}
          hidden={hideSensitive}
        />
        <InspectorMetric
          label="Cost basis"
          value={formatPortfolioMoney(point?.costBasisEur ?? 0, priceEur, "eur")}
          hidden={hideSensitive}
        />
        <InspectorMetric
          label="Unrealized"
          value={`${(point?.unrealizedEur ?? 0) >= 0 ? "+ " : "− "}${formatPortfolioMoney(
            Math.abs(point?.unrealizedEur ?? 0),
            priceEur,
            "eur",
          )}`}
          tone={(point?.unrealizedEur ?? 0) >= 0 ? "good" : "bad"}
          hidden={hideSensitive}
        />
      </div>

      <div className="rounded-md border bg-muted/20 p-2.5">
        <p className="text-[10px] font-medium text-muted-foreground">
          Since previous point
        </p>
        <div
          className={cn(
            "mt-1 text-sm font-semibold tabular-nums",
            primaryDelta >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-[var(--color-accent)]",
            blurClass(hideSensitive),
          )}
        >
          {primaryDelta >= 0 ? "+ " : "− "}
          {formatDetailedPortfolioMoney(
            Math.abs(primaryDelta),
            priceEur,
            chartCurrency,
          )}
        </div>
        <p className="mt-1 text-[10px] text-muted-foreground">
          {deltaPct === null
            ? "Start of selected range"
            : `${deltaPct >= 0 ? "+" : "−"}${Math.abs(deltaPct).toFixed(1)}%`}{" "}
          · {secondaryLabel}
        </p>
      </div>
    </aside>
  );
}

function InspectorMetric({
  label,
  value,
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
  tone?: "good" | "bad" | "neutral";
  hidden: boolean;
}) {
  return (
    <div className="rounded-md bg-muted/25 px-2.5 py-2">
      <p className="text-[10px] font-medium text-muted-foreground">{label}</p>
      <p
        className={cn(
          "mt-0.5 truncate text-sm font-semibold tabular-nums",
          tone === "good" && "text-emerald-600 dark:text-emerald-400",
          tone === "bad" && "text-[var(--color-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </p>
    </div>
  );
}

type ChartControlsSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  period: TimePeriod;
  onPeriodChange: (period: TimePeriod) => void;
  primaryColor: string;
  legendItems: TreasuryLegendItem[];
  seriesVisible: TreasurySeriesVisibility;
  onToggleSeries: (key: TreasuryChartSeriesKey) => void;
  activeSeries: TreasuryChartSeriesKey | null;
  onHoverSeries: (key: TreasuryChartSeriesKey | null) => void;
  markerCount: number;
  visibleMarkerCount: number;
  incomingMarkerCount: number;
  visibleIncomingMarkerCount: number;
  outgoingMarkerCount: number;
  visibleOutgoingMarkerCount: number;
  incomingMarkerMinimumBtc: number;
  onIncomingMarkerMinimumChange: (value: number) => void;
  outgoingMarkerMinimumBtc: number;
  onOutgoingMarkerMinimumChange: (value: number) => void;
  onResetMarkerMinimums: () => void;
  hideSensitive: boolean;
};

type ActivityMarkerValueEditorProps = {
  value: number;
  onChange: (value: number) => void;
  className?: string;
  hidden: boolean;
};

function ActivityFlowKey() {
  return (
    <div className="rounded-md border p-3">
      <p className="text-xs font-medium text-muted-foreground">
        Activity flows
      </p>
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
        {activityFlowKeys.map((flow) => (
          <div key={flow} className="flex min-w-0 items-center gap-2">
            <span
              className="size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: activityFlowColors[flow] }}
              aria-hidden="true"
            />
            <span className="truncate">{activityFlowLabels[flow]}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function ActivityLegendSwatch({ muted = false }: { muted?: boolean }) {
  return (
    <span
      className={cn(
        "flex w-11 shrink-0 items-center gap-0.5",
        muted && "opacity-40",
      )}
      aria-hidden="true"
    >
      {activityFlowKeys.map((flow) => (
        <span
          key={flow}
          className="size-1.5 rounded-full"
          style={{ backgroundColor: activityFlowColors[flow] }}
        />
      ))}
    </span>
  );
}

function ChartControlsSheet({
  open,
  onOpenChange,
  period,
  onPeriodChange,
  primaryColor,
  legendItems,
  seriesVisible,
  onToggleSeries,
  activeSeries,
  onHoverSeries,
  markerCount,
  visibleMarkerCount,
  incomingMarkerCount,
  visibleIncomingMarkerCount,
  outgoingMarkerCount,
  visibleOutgoingMarkerCount,
  incomingMarkerMinimumBtc,
  onIncomingMarkerMinimumChange,
  outgoingMarkerMinimumBtc,
  onOutgoingMarkerMinimumChange,
  onResetMarkerMinimums,
  hideSensitive,
}: ChartControlsSheetProps) {
  const markerMinimumsAtDefault =
    incomingMarkerMinimumBtc === DEFAULT_INCOMING_MARKER_MIN_BTC &&
    outgoingMarkerMinimumBtc === DEFAULT_OUTGOING_MARKER_MIN_BTC;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        className="w-[min(100vw,420px)] overflow-hidden p-0 sm:max-w-none"
        showCloseButton={false}
      >
        <SheetHeader className="border-b p-0">
          <div className="flex items-start justify-between gap-4 px-4 py-4 sm:px-6">
            <div className="min-w-0">
              <SheetTitle className="truncate text-xl sm:text-2xl">
                Chart controls
              </SheetTitle>
              <SheetDescription className="mt-1 truncate">
                Time range, chart series, and BTC dot minimums
              </SheetDescription>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className="text-[10px] text-muted-foreground">
                  {visibleMarkerCount.toLocaleString("en-US")} of{" "}
                  {markerCount.toLocaleString("en-US")} activity dots visible
                </span>
              </div>
            </div>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label="Close chart controls"
              onClick={() => onOpenChange(false)}
            >
              <X className="size-4" aria-hidden="true" />
            </Button>
          </div>
        </SheetHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="space-y-5 p-4 sm:p-6">
            <div className="rounded-md border p-3">
              <p className="text-xs font-medium text-muted-foreground">
                Time Range
              </p>
              <div className="mt-3 grid grid-cols-2 gap-2">
                {periodKeys.map((key) => (
                  <button
                    key={key}
                    type="button"
                    aria-pressed={period === key}
                    className={cn(
                      "rounded-md border px-2.5 py-2 text-left text-sm transition-colors",
                      period === key
                        ? "text-foreground"
                        : "border-transparent bg-muted/20 text-muted-foreground hover:bg-muted/45 hover:text-foreground",
                    )}
                    style={
                      period === key
                        ? {
                            backgroundColor: `${primaryColor}24`,
                            borderColor: primaryColor,
                          }
                        : undefined
                    }
                    onClick={() => onPeriodChange(key)}
                  >
                    {periodLabels[key]}
                  </button>
                ))}
              </div>
            </div>

            <ActivityFlowKey />

            <div className="rounded-md border p-3">
              <p className="text-xs font-medium text-muted-foreground">
                Series
              </p>
              <div className="mt-3 space-y-1">
                {legendItems.map((item) => (
                  <label
                    key={item.key}
                    className={cn(
                      "flex cursor-pointer items-center gap-2 rounded-md px-2 py-2 text-sm transition-colors hover:bg-muted/35",
                      !seriesVisible[item.key] && "text-muted-foreground",
                      activeSeries !== null &&
                        activeSeries !== item.key &&
                        "opacity-55",
                    )}
                    onMouseEnter={() => onHoverSeries(item.key)}
                    onMouseLeave={() => onHoverSeries(null)}
                  >
                    <Checkbox
                      checked={seriesVisible[item.key]}
                      onCheckedChange={() => onToggleSeries(item.key)}
                      aria-label={`Show ${item.label}`}
                      className="data-[state=checked]:border-[var(--chart-control-accent)] data-[state=checked]:bg-[var(--chart-control-accent)] data-[state=checked]:text-background"
                      style={
                        {
                          "--chart-control-accent": item.color,
                        } as React.CSSProperties
                      }
                    />
                    {item.key === "events" ? (
                      <ActivityLegendSwatch muted={!seriesVisible.events} />
                    ) : (
                      <span
                        className={cn(
                          "h-0.5 w-6 shrink-0 rounded-full",
                          item.dashed && "border-t border-dashed bg-transparent",
                        )}
                        style={{
                          backgroundColor: item.dashed ? "transparent" : item.color,
                          borderColor: item.color,
                        }}
                      />
                    )}
                    <span className="truncate">{item.label}</span>
                  </label>
                ))}
              </div>
            </div>

            <div className="space-y-3 rounded-md border p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-xs font-medium text-muted-foreground">
                    Marker size
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Minimum BTC size for activity dots
                  </p>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="shrink-0 gap-2"
                  onClick={onResetMarkerMinimums}
                  disabled={markerMinimumsAtDefault}
                >
                  <RefreshCw className="size-3.5" aria-hidden="true" />
                  Reset
                </Button>
              </div>
              <div className="flex items-center justify-between gap-3 text-sm">
                <div>
                  <p className="text-xs font-medium text-muted-foreground">
                    Incoming payments
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Minimum size in BTC ·{" "}
                    {visibleIncomingMarkerCount.toLocaleString("en-US")} of{" "}
                    {incomingMarkerCount.toLocaleString("en-US")} dots shown
                  </p>
                </div>
                <ActivityMarkerValueEditor
                  value={incomingMarkerMinimumBtc}
                  onChange={onIncomingMarkerMinimumChange}
                  hidden={hideSensitive}
                />
              </div>
              <ActivityMarkerSlider
                id="incoming-marker-minimum"
                label="Minimum incoming payment dot size in BTC"
                value={incomingMarkerMinimumBtc}
                color={activityFlowColors.incoming}
                onChange={onIncomingMarkerMinimumChange}
              />
            </div>

            <div className="rounded-md border border-red-500/20 bg-red-500/5 p-3">
              <div className="flex items-center justify-between gap-3 text-sm">
                <div>
                  <p className="text-xs font-medium text-red-500 dark:text-red-400">
                    Outgoing activity
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Minimum size in BTC ·{" "}
                    {visibleOutgoingMarkerCount.toLocaleString("en-US")} of{" "}
                    {outgoingMarkerCount.toLocaleString("en-US")} dots shown
                  </p>
                </div>
                <ActivityMarkerValueEditor
                  value={outgoingMarkerMinimumBtc}
                  onChange={onOutgoingMarkerMinimumChange}
                  className="text-red-500 dark:text-red-400"
                  hidden={hideSensitive}
                />
              </div>
              <ActivityMarkerSlider
                id="outgoing-marker-minimum"
                label="Minimum outgoing activity dot size in BTC"
                value={outgoingMarkerMinimumBtc}
                color={activityFlowColors.outgoing}
                onChange={onOutgoingMarkerMinimumChange}
              />
            </div>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}

function ActivityMarkerValueEditor({
  value,
  onChange,
  className,
  hidden,
}: ActivityMarkerValueEditorProps) {
  const formattedValue = formatEditableActivityMarkerMinimum(value);
  const [draft, setDraft] = React.useState(formattedValue);
  const [editing, setEditing] = React.useState(false);

  React.useEffect(() => {
    if (!editing) setDraft(formattedValue);
  }, [editing, formattedValue]);

  const commitDraft = React.useCallback(
    (rawValue: string) => {
      const parsed = Number(rawValue);
      if (!rawValue.trim() || !Number.isFinite(parsed)) {
        setDraft(formatEditableActivityMarkerMinimum(value));
        return;
      }
      const nextValue = clampActivityMarkerMinimum(parsed);
      onChange(nextValue);
      setDraft(formatEditableActivityMarkerMinimum(nextValue));
    },
    [onChange, value],
  );

  return (
    <label
      className={cn(
        "group inline-flex h-8 items-center rounded-md border border-transparent bg-transparent transition-colors hover:border-border hover:bg-background focus-within:border-ring focus-within:bg-background focus-within:ring-2 focus-within:ring-ring/20",
        className,
        hidden && blurClass(true),
      )}
      title="Click to enter a custom BTC minimum"
    >
      <input
        aria-label="Custom marker minimum in BTC"
        className="h-full w-[10ch] rounded-l-md bg-transparent px-2 text-right font-medium tabular-nums outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        min={0}
        step={ACTIVITY_MARKER_INPUT_STEP_BTC}
        type="number"
        value={editing ? draft : formattedValue}
        onBlur={(event) => {
          commitDraft(event.currentTarget.value);
          setEditing(false);
        }}
        onChange={(event) => {
          const nextDraft = event.currentTarget.value;
          setDraft(nextDraft);
          const parsed = Number(nextDraft);
          if (nextDraft.trim() && Number.isFinite(parsed)) {
            onChange(clampActivityMarkerMinimum(parsed));
          }
        }}
        onFocus={() => setEditing(true)}
      />
      <span className="pr-2 text-xs">BTC</span>
    </label>
  );
}

export const RevenueFlowChart = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
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
      ),
    [activityMarkerMinimumForPoint, chartData, seriesVisible.events],
  );
  const expandedMarkerView = React.useMemo(
    () =>
      activityMarkerView(
        expandedChartData,
        seriesVisible.events,
        activityMarkerMinimumForPoint,
      ),
    [activityMarkerMinimumForPoint, expandedChartData, seriesVisible.events],
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
    const handleChartDoubleClick = (event: React.MouseEvent) => {
      if (plottedData.length <= 3) return;
      resetBrushRange();
      event.preventDefault();
    };
    const handleChartClick = (event: React.MouseEvent) => {
      if (event.detail >= 2) handleChartDoubleClick(event);
    };
    const handleChartMouseDown = (event: React.MouseEvent) => {
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
                  {Math.abs(gainPct).toFixed(2)}% unrealized
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
                  className="min-h-0 flex-1 w-full overflow-visible [&_.recharts-tooltip-wrapper]:!z-[100]"
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
                        : formatEurPrice(Number(value))
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
                        priceEur={snapshot.priceEur}
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
                      dataKey="eventBalanceBtc"
                      name="Activity"
                      fill="transparent"
                      shape={(props) => (
                        <ActivityScatterDot
                          {...props}
                          activeSeries={activeSeries}
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
                  BTC Price (EUR)
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
                priceEur={snapshot.priceEur}
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
