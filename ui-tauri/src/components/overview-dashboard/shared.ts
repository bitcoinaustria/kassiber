import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  CheckCircle2,
  CircleDollarSign,
  ClipboardList,
  CreditCard,
  FileText,
  Plus,
  RefreshCw,
  ShieldAlert,
  Users,
  WalletCards,
} from "lucide-react";
import * as React from "react";

import { type ChartConfig } from "@/components/ui/chart";
import {
  formatBtc,
  MISSING_FIAT_LABEL,
  type Currency,
} from "@/lib/currency";
import { useUiStore } from "@/store/ui";
import {
  type OverviewSnapshot,
  type PortfolioPoint,
  type Tx as OverviewTx,
} from "@/mocks/seed";

export type StatItem = {
  title: string;
  previousValue: number;
  value: number;
  changePercent: number;
  isPositive: boolean;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  format: "currency" | "number";
  comparisonLabel: string;
  href: OverviewHref;
};

export type HoldingsItem = {
  name: string;
  value: number;
  percent: number;
  color: string;
};

export type BalanceDriverItem = {
  key: "incoming" | "outgoing" | "swap" | "fees";
  label: string;
  valueBtc: number;
  count: number;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  toneClassName: string;
};

export type PortfolioChartPoint = {
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

export type PortfolioChartMetric = "value" | "btc" | "basis" | "unrealized";
export type ActivityFlow = "incoming" | "outgoing" | "swap" | "transfer" | "fee";
export type TreasuryChartSeriesKey = "primary" | "price" | "basis" | "events";
export type TreasurySeriesVisibility = Record<TreasuryChartSeriesKey, boolean>;
export type TreasuryLegendItem = {
  key: TreasuryChartSeriesKey;
  label: string;
  color: string;
  dashed: boolean;
};

export const DEFAULT_INCOMING_MARKER_MIN_BTC = 0.0025;
export const DEFAULT_OUTGOING_MARKER_MIN_BTC = 0;
export const ACTIVITY_MARKER_SLIDER_MAX_BTC = 1;
export const ACTIVITY_MARKER_INPUT_STEP_BTC = 0.00000001;
export const ACTIVITY_MARKER_SLIDER_MARKS = [0, 0.0025, 0.01, 0.1, 0.5, 1] as const;
export const INCOMING_MARKER_MIN_PARAM = "incomingMinBtc";
export const OUTGOING_MARKER_MIN_PARAM = "outgoingMinBtc";
export const LEGACY_INCOMING_MARKER_MIN_PARAM = "incomingMin";
export const LEGACY_OUTGOING_MARKER_MIN_PARAM = "outgoingMin";
export const TREASURY_BRUSH_MIN_WINDOW_MS = (7 * 24 * 60 * 60 * 1000) / 3;
export const TREASURY_BRUSH_MIN_INDEX_SPAN = 2;

export const defaultTreasurySeriesVisibility: TreasurySeriesVisibility = {
  primary: true,
  price: true,
  basis: true,
  events: true,
};

export type TreasuryChartPoint = PortfolioChartPoint & {
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

export type PortfolioChartMouseState = {
  activePayload?: Array<{ payload?: TreasuryChartPoint }>;
};

export type TreasuryBrushRange = {
  startIndex: number;
  endIndex: number;
};

export type TreasuryBrushChange = {
  startIndex?: number;
  endIndex?: number;
};

export type ActivityMarkerView = {
  activityPoints: TreasuryChartPoint[];
  chartDisplayData: TreasuryChartPoint[];
  visibleActivityMarkers: TreasuryChartPoint[];
};

export type TreasuryActivityEvent = {
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

export type TransactionStatus = "confirmed" | "pending" | "review" | "failed";
export type OverviewTransactionFlow = "incoming" | "outgoing" | "transfer" | "swap";
export type OverviewHealthTone = "good" | "warning" | "alert" | "neutral";
export type OverviewHref =
  | "/connections"
  | "/journals"
  | "/quarantine"
  | "/reports"
  | "/transactions";

export type Transaction = {
  id: string;
  txid: string;
  explorerId?: string;
  counterparty: string;
  counterpartyInitials: string;
  paymentMethod?: "On-chain" | "Lightning" | "Liquid" | "Other";
  tags: string[];
  status: TransactionStatus;
  flow?: OverviewTransactionFlow;
  amount: number | null;
  amountBtc?: number;
  date: string;
};

export type OverviewHealthItem = {
  key: string;
  title: string;
  value: string;
  detail: string;
  href: OverviewHref;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

export type OverviewReadiness = {
  title: string;
  detail: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

export const currencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
});

export const numberFormatter = new Intl.NumberFormat("en-US");

export const compactCurrencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
  notation: "compact",
  maximumFractionDigits: 0,
});

export const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function btcFromEur(eur: number, priceEur: number) {
  return priceEur ? eur / priceEur : 0;
}

export function formatDisplayMoney(
  eur: number | null,
  priceEur: number,
  currency: Currency,
) {
  if (eur === null) return MISSING_FIAT_LABEL;
  if (currency === "btc") return formatBtc(btcFromEur(eur, priceEur));
  return currencyFormatter.format(eur);
}

export function formatSignedDisplayMoney(
  eur: number | null,
  priceEur: number,
  currency: Currency,
) {
  if (eur === null) return MISSING_FIAT_LABEL;
  if (currency === "btc") {
    return formatBtc(btcFromEur(eur, priceEur), { sign: true });
  }
  const prefix = eur >= 0 ? "+ " : "− ";
  return `${prefix}${currencyFormatter.format(Math.abs(eur))}`;
}

export function formatCompactDisplayMoney(
  eur: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") {
    return formatBtc(btcFromEur(eur, priceEur), { precision: 3 });
  }
  return compactCurrencyFormatter.format(eur);
}

export function formatPortfolioMoney(
  amount: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") return formatBtc(amount);
  return formatDisplayMoney(amount, priceEur, currency);
}

export function formatDriverValue(btc: number, priceEur: number, currency: Currency) {
  if (currency === "btc") {
    return formatBtc(btc, { precision: btc > 0 && btc < 0.001 ? 8 : 3 });
  }
  return formatCompactDisplayMoney(btc * priceEur, priceEur, currency);
}

export function formatDetailedPortfolioMoney(
  amount: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") {
    return formatBtc(amount, { precision: Math.abs(amount) < 0.01 ? 8 : 4 });
  }
  return formatDisplayMoney(amount, priceEur, currency);
}

export function donutCenterValueClass(value: string) {
  const length = value.replace(/\s+/g, "").length;
  if (length <= 7) return "text-sm sm:text-base";
  if (length <= 9) return "text-xs sm:text-sm";
  if (length <= 11) return "text-[11px] sm:text-xs";
  return "text-[10px] sm:text-[11px]";
}

export function transactionBtc(tx: Transaction, priceEur: number) {
  return tx.amountBtc ?? btcFromEur(tx.amount ?? 0, priceEur);
}

export function satToBtc(sats: number | undefined) {
  return (sats ?? 0) / 100_000_000;
}

/**
 * Custom hook for hover highlight interaction.
 * Provides stable callback to prevent unnecessary re-renders in chart components.
 */
export function useHoverHighlight<T extends string | number>() {
  const [active, setActive] = React.useState<T | null>(null);

  const handleHover = React.useCallback((value: T | null) => {
    setActive(value);
  }, []);

  return { active, handleHover };
}

export const mixBase = "var(--background)";

export const palette = {
  primary: "var(--primary)",
  risk: {
    main: "var(--color-accent)",
    soft: `color-mix(in oklch, var(--color-accent) 16%, transparent)`,
    light: `color-mix(in oklch, var(--color-accent) 70%, ${mixBase})`,
  },
  secondary: {
    light: `color-mix(in oklch, var(--primary) 75%, ${mixBase})`,
    dark: `color-mix(in oklch, var(--primary) 85%, ${mixBase})`,
  },
  tertiary: {
    light: `color-mix(in oklch, var(--primary) 55%, ${mixBase})`,
    dark: `color-mix(in oklch, var(--primary) 65%, ${mixBase})`,
  },
  quaternary: {
    light: `color-mix(in oklch, var(--primary) 40%, ${mixBase})`,
    dark: `color-mix(in oklch, var(--primary) 45%, ${mixBase})`,
  },
};

export const portfolioChartColors = {
  light: {
    value: "#f7931a",
    costBasis: "#2fae79",
    focus: "#2f2f33",
    risk: "#e3000f",
    riskSoft: "rgba(227, 0, 15, 0.16)",
  },
  dark: {
    value: "#f6a21a",
    costBasis: "#50c695",
    focus: "#e8e8ec",
    risk: "#ff3341",
    riskSoft: "rgba(255, 51, 65, 0.18)",
  },
} as const;

export function useResolvedColorMode() {
  const theme = useUiStore((state) => state.theme);
  const [systemDark, setSystemDark] = React.useState(false);

  React.useEffect(() => {
    if (typeof window === "undefined") return undefined;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => setSystemDark(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  return theme === "dark" || (theme === "system" && systemDark)
    ? "dark"
    : "light";
}

export const holdingsChartConfig = {
  onchain: { label: "On-chain BTC", color: palette.primary },
  lightning: { label: "Lightning", theme: palette.secondary },
  liquid: { label: "Liquid", theme: palette.tertiary },
  other: { label: "Other", theme: palette.quaternary },
} satisfies ChartConfig;

export const statsData: StatItem[] = [
  {
    title: "Portfolio value",
    previousValue: 198502,
    value: 312842.77,
    changePercent: 27.86,
    isPositive: true,
    icon: CircleDollarSign,
    format: "currency",
    comparisonLabel: "vs Last Month",
    href: "/reports",
  },
  {
    title: "Transactions",
    previousValue: 184,
    value: 218,
    changePercent: 18.4,
    isPositive: true,
    icon: ClipboardList,
    format: "number",
    comparisonLabel: "vs Last Month",
    href: "/transactions",
  },
  {
    title: "Reviewed events",
    previousValue: 412,
    value: 497,
    changePercent: 20.8,
    isPositive: true,
    icon: Users,
    format: "number",
    comparisonLabel: "vs Last Month",
    href: "/connections",
  },
  {
    title: "Open review",
    previousValue: 98,
    value: 84,
    changePercent: 13.73,
    isPositive: false,
    icon: CreditCard,
    format: "currency",
    comparisonLabel: "vs Last Month",
    href: "/quarantine",
  },
];

export function latestPortfolioBalanceBtc(snapshot: OverviewSnapshot) {
  if (snapshot.portfolioSeries?.length) {
    const latest = [...snapshot.portfolioSeries].sort((a, b) =>
      a.date.localeCompare(b.date),
    )[snapshot.portfolioSeries.length - 1];
    if (latest) return latest.balanceBtc;
  }
  const latestBalance = snapshot.balanceSeries[snapshot.balanceSeries.length - 1];
  if (typeof latestBalance === "number") return latestBalance;
  return btcFromEur(snapshot.fiat.eurBalance, snapshot.priceEur);
}

export function buildStatsData(
  snapshot: OverviewSnapshot,
  currency: Currency,
): StatItem[] {
  const isBitcoinMode = currency === "btc";
  const transactionCount = snapshot.status?.transactionCount ?? snapshot.txs.length;
  return [
    {
      ...statsData[0],
      value: snapshot.fiat.eurBalance,
      previousValue: isBitcoinMode ? 0 : snapshot.fiat.eurCostBasis,
      changePercent: !isBitcoinMode && snapshot.fiat.eurCostBasis
        ? (snapshot.fiat.eurUnrealized / snapshot.fiat.eurCostBasis) * 100
        : 0,
      isPositive: snapshot.fiat.eurUnrealized >= 0,
      comparisonLabel: isBitcoinMode
        ? "BTC balance"
        : snapshot.fiat.eurCostBasis
          ? "vs cost basis"
          : "from loaded rows",
    },
    {
      ...statsData[1],
      value: transactionCount,
      previousValue: 0,
      changePercent: 0,
      isPositive: true,
      comparisonLabel: "loaded rows",
    },
    {
      ...statsData[2],
      title: "Connections",
      value: snapshot.connections.length,
      previousValue: 0,
      changePercent: 0,
      isPositive: true,
      comparisonLabel: "configured",
    },
    {
      ...statsData[3],
      title: "Open review",
      value: snapshot.status?.quarantines ?? 0,
      previousValue: 0,
      changePercent: 0,
      isPositive: (snapshot.status?.quarantines ?? 0) === 0,
      format: "number",
      comparisonLabel: "journal quarantine",
    },
  ];
}

export const fullYearData = [
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

export const fiveYearData = [
  { month: "2022", thisYear: 210000, prevYear: 178000 },
  { month: "2023", thisYear: 248000, prevYear: 205000 },
  { month: "2024", thisYear: 287000, prevYear: 244000 },
  { month: "2025", thisYear: 319000, prevYear: 276000 },
  { month: "2026", thisYear: 337000, prevYear: 291000 },
];

export type TimePeriod = "30days" | "3months" | "ytd" | "1year" | "5years" | "all";

export const periodLabels: Record<TimePeriod, string> = {
  "30days": "30 Days",
  "3months": "3 Months",
  ytd: "YTD",
  "1year": "1 Year",
  "5years": "5 Years",
  all: "All Time",
};

export const periodShortLabels: Record<TimePeriod, string> = {
  "30days": "30D",
  "3months": "3M",
  ytd: "YTD",
  "1year": "1Y",
  "5years": "5Y",
  all: "All",
};

export const periodKeys: TimePeriod[] = [
  "30days",
  "3months",
  "ytd",
  "1year",
  "5years",
  "all",
];

export function normalizeTimePeriodParam(value: string | null): TimePeriod | null {
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

export function initialTimePeriodFromUrl(): TimePeriod {
  if (typeof window === "undefined") return "ytd";
  const params = new URLSearchParams(window.location.search);
  return normalizeTimePeriodParam(params.get("period")) ?? "ytd";
}

export function clampActivityMarkerMinimum(value: number) {
  if (!Number.isFinite(value)) return 0;
  const clamped = Math.max(value, 0);
  return (
    Math.round(clamped / ACTIVITY_MARKER_INPUT_STEP_BTC) *
    ACTIVITY_MARKER_INPUT_STEP_BTC
  );
}

export function initialActivityMarkerMinimumFromUrl(
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

export function serializeActivityMarkerMinimum(value: number) {
  return clampActivityMarkerMinimum(value)
    .toFixed(8)
    .replace(/0+$/, "")
    .replace(/\.$/, "");
}

export function formatEditableActivityMarkerMinimum(value: number) {
  const serialized = serializeActivityMarkerMinimum(value);
  const [, fraction = ""] = serialized.split(".");
  const precision = Math.min(8, Math.max(1, fraction.length + 1));
  return clampActivityMarkerMinimum(value).toFixed(precision);
}

export function fallbackPortfolioData(
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

export function getDataForPeriod(
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

export function buildDatedPortfolioPoints(
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

export function parseSeriesDate(value: string | undefined) {
  const parsed = value ? new Date(`${value.slice(0, 10)}T00:00:00Z`) : null;
  return parsed && !Number.isNaN(parsed.valueOf()) ? parsed : new Date();
}

export function isPointInPeriod(
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

export function buildPortfolioChartPoint(
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

export function chartCurrencyForMetric(
  metric: PortfolioChartMetric,
  currency: Currency,
): Currency {
  if (metric === "btc") return "btc";
  if (metric === "value") return currency;
  return "eur";
}

export function parseIsoDayDate(value: string | undefined) {
  if (!value || !/^\d{4}-\d{2}-\d{2}/.test(value)) return null;
  const parsed = new Date(`${value.slice(0, 10)}T00:00:00Z`);
  return Number.isNaN(parsed.valueOf()) ? null : parsed;
}

export function parseOverviewTxDate(value: string | undefined) {
  if (!value) return null;
  const normalized = value.includes("T") ? value : `${value.replace(" ", "T")}Z`;
  const parsed = new Date(normalized);
  return Number.isNaN(parsed.valueOf()) ? null : parsed;
}

export function treasurySortTime(value: string | undefined) {
  if (!value) return null;
  const key = value.split("#")[0] ?? value;
  const parsed = key.includes("T") ? new Date(key) : parseIsoDayDate(key);
  return parsed && !Number.isNaN(parsed.valueOf()) ? parsed.valueOf() : null;
}

export function fullTreasuryBrushRange(dataLength: number): TreasuryBrushRange {
  return {
    startIndex: 0,
    endIndex: Math.max(0, dataLength - 1),
  };
}

export function sameTreasuryBrushRange(
  a: TreasuryBrushRange | null,
  b: TreasuryBrushRange,
) {
  return a?.startIndex === b.startIndex && a.endIndex === b.endIndex;
}

export function clampTreasuryBrushIndex(index: number | undefined, dataLength: number) {
  if (!Number.isFinite(index)) return 0;
  return Math.max(0, Math.min(dataLength - 1, Math.round(index ?? 0)));
}

export function treasuryBrushTime(point: TreasuryChartPoint | undefined) {
  if (!point) return null;
  if (Number.isFinite(point.sortTimeMs) && point.sortTimeMs > 0) {
    return point.sortTimeMs;
  }
  return treasurySortTime(point.date);
}

export function findTreasuryBrushIndexAtOrAfter(
  data: TreasuryChartPoint[],
  targetTime: number,
) {
  const index = data.findIndex((point) => {
    const time = treasuryBrushTime(point);
    return time !== null && time >= targetTime;
  });
  return index === -1 ? data.length - 1 : index;
}

export function findTreasuryBrushIndexAtOrBefore(
  data: TreasuryChartPoint[],
  targetTime: number,
) {
  for (let index = data.length - 1; index >= 0; index -= 1) {
    const time = treasuryBrushTime(data[index]);
    if (time !== null && time <= targetTime) return index;
  }
  return 0;
}

export function treasuryBrushMinIndexSpan(data: TreasuryChartPoint[]) {
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

export function normalizeTreasuryBrushRange(
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

export function rawTreasuryBrushRange(
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

export function formatTreasuryTick(value: string) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return value;
  return parsed.toLocaleDateString("en-US", {
    month: "short",
    year: "2-digit",
    timeZone: "UTC",
  });
}

export function formatTreasuryDetailDate(value: string) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return value;
  return parsed.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function formatEurPrice(eur: number) {
  if (Math.abs(eur) >= 100_000) return `${Math.round(eur).toLocaleString("en-US")} EUR`;
  return `${eur.toLocaleString("en-US", {
    maximumFractionDigits: 0,
  })} EUR`;
}

export function treasuryPrimaryValue(point: TreasuryChartPoint) {
  return point.balanceBtc;
}

export function formatBtcAxis(value: number) {
  const precision = Math.abs(value) >= 10 ? 0 : Math.abs(value) >= 1 ? 2 : 3;
  return formatBtc(value, { precision }).replace("₿ ", "₿");
}

export function activityMarkerSliderValue(value: number) {
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


export function compactEventId(value: string | undefined) {
  if (!value) return null;
  if (value.length <= 18) return value;
  return `${value.slice(0, 8)}…${value.slice(-6)}`;
}

export function statusForOverviewTx(tx: OverviewTx): TransactionStatus {
  if (tx.internal) return "pending";
  if (tx.conf > 0) return "confirmed";
  return tx.tag.toLowerCase().includes("review") ? "review" : "pending";
}

export function activityFlowForTx(tx: OverviewTx): ActivityFlow {
  if (tx.type === "Fee") return "fee";
  return flowForOverviewTx(tx);
}

export const activityFlowLabels: Record<ActivityFlow, string> = {
  incoming: "Received",
  outgoing: "Spent",
  swap: "Swap",
  transfer: "Transfer",
  fee: "Fee",
};

export const activityFlowColors: Record<ActivityFlow, string> = {
  incoming: "#34d399",
  outgoing: "#f87171",
  swap: "#38bdf8",
  transfer: "#f59e0b",
  fee: "#a1a1aa",
};

export const activityFlowKeys: ActivityFlow[] = [
  "incoming",
  "outgoing",
  "swap",
  "transfer",
  "fee",
];


export function activityTxs(snapshot: OverviewSnapshot): TreasuryActivityEvent[] {
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

export function isActivityInTreasuryPeriod(
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

export function activityDateKey(event: TreasuryActivityEvent) {
  return `${event.occurredAt.toISOString()}#${event.tx.id || event.sequence}`;
}

export function buildTreasuryBasePoint(
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

export function nearestTreasuryAnchor(
  points: TreasuryChartPoint[],
  event: TreasuryActivityEvent,
) {
  const eventTime = event.occurredAt.valueOf();
  const previous = [...points]
    .reverse()
    .find((point) => point.sortTimeMs <= eventTime);
  return previous ?? points[0] ?? null;
}

export function buildTreasuryActivityPoint(
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

export function enrichTreasuryChartData(
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

export function buildTreasuryChartStats(points: TreasuryChartPoint[]) {
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

export function activityMarkerView(
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

export function expandFallbackYearData(
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

export function samplePortfolioPoints(
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

export function formatPortfolioTick(value: string, period: TimePeriod) {
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

export function portfolioTickBucket(point: PortfolioChartPoint, period: TimePeriod) {
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

export function portfolioAxisTicks(
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

export function formatPortfolioDetailLabel(value: string) {
  const date = parseSeriesDate(value);
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function percentOf(value: number, total: number) {
  if (total <= 0) return 0;
  return Math.round((value / total) * 100);
}

export type BalanceRail = "onchain" | "lightning" | "liquid" | "other";

export function railForConnection(kind: string, label: string): BalanceRail {
  const kindKey = kind.toLowerCase();
  switch (kindKey) {
    case "xpub":
    case "address":
    case "descriptor":
    case "btcpay":
      return "onchain";
    case "core-ln":
    case "lnd":
    case "nwc":
    case "phoenix":
      return "lightning";
    case "cashu":
    case "kraken":
    case "bitstamp":
    case "coinbase":
    case "bitpanda":
    case "river":
    case "bullbitcoin":
    case "coinfinity":
    case "strike":
    case "csv":
    case "bip329":
      return "other";
  }
  const value = `${kind} ${label}`.toLowerCase();
  if (value.includes("liquid") || value.includes("lbtc")) return "liquid";
  if (
    value.includes("lightning") ||
    value.includes("phoenix") ||
    value.includes("nwc") ||
    value.includes("core-ln") ||
    value.includes("lnd")
  ) {
    return "lightning";
  }
  return "onchain";
}

export function buildBalanceRailItems(snapshot: OverviewSnapshot) {
  const byRail: Record<BalanceRail, number> = {
    onchain: 0,
    lightning: 0,
    liquid: 0,
    other: 0,
  };
  for (const connection of snapshot.connections) {
    if (connection.balance <= 0) continue;
    const rail = railForConnection(connection.kind, connection.label);
    byRail[rail] += connection.balance * snapshot.priceEur;
  }
  const total = Object.values(byRail).reduce((sum, value) => sum + value, 0);
  const items = [
    {
      key: "onchain",
      label: "On-chain",
      value: byRail.onchain,
      percent: percentOf(byRail.onchain, total),
      color: palette.primary,
    },
    {
      key: "lightning",
      label: "Lightning",
      value: byRail.lightning,
      percent: percentOf(byRail.lightning, total),
      color: palette.secondary.light,
    },
    {
      key: "liquid",
      label: "Liquid",
      value: byRail.liquid,
      percent: percentOf(byRail.liquid, total),
      color: palette.tertiary.light,
    },
    {
      key: "other",
      label: "Other",
      value: byRail.other,
      percent: percentOf(byRail.other, total),
      color: `color-mix(in oklch, var(--muted-foreground) 70%, ${mixBase})`,
    },
  ];
  return {
    total,
    items: total > 0 ? items.filter((item) => item.value > 0) : items,
  };
}

export function buildHoldingsBySource(snapshot: OverviewSnapshot): HoldingsItem[] {
  const rows = snapshot.connections
    .filter((connection) => connection.balance > 0)
    .map((connection) => ({
      name: connection.label,
      value: connection.balance * snapshot.priceEur,
    }))
    .sort((a, b) => b.value - a.value);
  const total = rows.reduce((sum, item) => sum + item.value, 0);
  const visibleRows =
    rows.length > 4
      ? [
          ...rows.slice(0, 3),
          {
            name: "Other sources",
            value: rows.slice(3).reduce((sum, item) => sum + item.value, 0),
          },
        ]
      : rows;
  const colors = [
    palette.primary,
    palette.secondary.light,
    palette.tertiary.light,
    `color-mix(in oklch, var(--muted-foreground) 70%, ${mixBase})`,
  ];
  return visibleRows.map((item, index) => ({
    name: item.name,
    value: item.value,
    percent: percentOf(item.value, total),
    color: colors[index] ?? colors[colors.length - 1],
  }));
}

export function buildBalanceDrivers(snapshot: OverviewSnapshot) {
  const totals = {
    incomingBtc: 0,
    outgoingBtc: 0,
    swapBtc: 0,
    feesBtc: 0,
    incomingCount: 0,
    outgoingCount: 0,
    swapCount: 0,
    feeCount: 0,
  };
  for (const tx of snapshot.txs.filter((row) => !row.excluded)) {
    const flow = flowForOverviewTx(tx);
    const amountBtc = satToBtc(Math.abs(tx.amountSat));
    const feeBtc = satToBtc(Math.abs(tx.feeSat ?? 0));
    if (flow === "incoming") {
      totals.incomingBtc += amountBtc;
      totals.incomingCount += 1;
    } else if (flow === "outgoing") {
      totals.outgoingBtc += amountBtc;
      totals.outgoingCount += 1;
    } else if (flow === "swap") {
      const pairedVolume = Math.max(
        amountBtc,
        satToBtc(Math.abs(tx.pair?.outAmountSat ?? 0)),
        satToBtc(Math.abs(tx.pair?.inAmountSat ?? 0)),
      );
      totals.swapBtc += pairedVolume;
      totals.swapCount += 1;
    }
    if (feeBtc > 0) {
      totals.feesBtc += feeBtc;
      totals.feeCount += 1;
    }
  }
  const netBtc = totals.incomingBtc - totals.outgoingBtc - totals.feesBtc;
  const items: BalanceDriverItem[] = [
    {
      key: "incoming",
      label: "Incoming",
      valueBtc: totals.incomingBtc,
      count: totals.incomingCount,
      icon: ArrowDownRight,
      toneClassName: "text-emerald-700 dark:text-emerald-300",
    },
    {
      key: "outgoing",
      label: "Outgoing",
      valueBtc: totals.outgoingBtc,
      count: totals.outgoingCount,
      icon: ArrowUpRight,
      toneClassName: "text-red-700 dark:text-red-300",
    },
    {
      key: "swap",
      label: "Swap volume",
      valueBtc: totals.swapBtc,
      count: totals.swapCount,
      icon: ArrowLeftRight,
      toneClassName: "text-sky-700 dark:text-sky-300",
    },
    {
      key: "fees",
      label: "Fees",
      valueBtc: totals.feesBtc,
      count: totals.feeCount,
      icon: CircleDollarSign,
      toneClassName: "text-muted-foreground",
    },
  ];
  return {
    rows: items,
    maxValueBtc: Math.max(...items.map((item) => item.valueBtc), 0),
    netBtc,
    transactionCount: snapshot.txs.filter((row) => !row.excluded).length,
  };
}

export function transactionsDriverSearch(driver: BalanceDriverItem["key"]) {
  const search: Record<string, string> = {};
  if (typeof window !== "undefined") {
    const currentParams = new URLSearchParams(window.location.search);
    const period = currentParams.get("period");
    if (period) search.period = period;
  }
  if (driver === "fees") {
    search.fees = "with-fees";
  } else {
    search.flow = driver;
  }
  return search;
}

export const transactionStatuses: TransactionStatus[] = [
  "confirmed",
  "pending",
  "review",
  "failed",
];

export const transactionRecords: Transaction[] = [
  {
    id: "1",
    txid: "TX-2026-001",
    counterparty: "Cold Storage",
    counterpartyInitials: "CS",
    tags: ["Invoice", "ACME GmbH"],
    status: "confirmed",
    amount: 2499.0,
    date: "Jan 28, 2026",
  },
  {
    id: "2",
    txid: "TX-2026-002",
    counterparty: "Home Node",
    counterpartyInitials: "HN",
    tags: ["Server rental", "Hetzner"],
    status: "review",
    amount: 1348.0,
    date: "Jan 27, 2026",
  },
  {
    id: "3",
    txid: "TX-2026-003",
    counterparty: "Multisig Vault",
    counterpartyInitials: "MV",
    tags: ["Internal transfer"],
    status: "pending",
    amount: 1198.0,
    date: "Jan 27, 2026",
  },
  {
    id: "4",
    txid: "TX-2026-004",
    counterparty: "Alby Hub",
    counterpartyInitials: "AH",
    tags: ["Lightning payment"],
    status: "confirmed",
    amount: 799.0,
    date: "Jan 26, 2026",
  },
  {
    id: "5",
    txid: "TX-2026-005",
    counterparty: "Cashu Wallet",
    counterpartyInitials: "CW",
    tags: ["Ecash spend"],
    status: "failed",
    amount: 599.0,
    date: "Jan 26, 2026",
  },
  {
    id: "6",
    txid: "TX-2026-006",
    counterparty: "BTCPay Server",
    counterpartyInitials: "BP",
    tags: ["Customer invoice", "Bitcoin Austria"],
    status: "confirmed",
    amount: 5498.0,
    date: "Jan 25, 2026",
  },
  {
    id: "7",
    txid: "TX-2026-007",
    counterparty: "Bitstamp",
    counterpartyInitials: "BS",
    tags: ["EUR off-ramp"],
    status: "confirmed",
    amount: 1199.0,
    date: "Jan 25, 2026",
  },
  {
    id: "8",
    txid: "TX-2026-008",
    counterparty: "Kraken",
    counterpartyInitials: "KR",
    tags: ["Withdrawal", "Self-custody"],
    status: "pending",
    amount: 878.0,
    date: "Jan 24, 2026",
  },
  {
    id: "9",
    txid: "TX-2026-009",
    counterparty: "Phoenix Wallet",
    counterpartyInitials: "PW",
    tags: ["Lightning sweep"],
    status: "confirmed",
    amount: 549.0,
    date: "Jan 24, 2026",
  },
  {
    id: "10",
    txid: "TX-2026-010",
    counterparty: "Voltage Cloud",
    counterpartyInitials: "VC",
    tags: ["Node hosting"],
    status: "confirmed",
    amount: 1648.0,
    date: "Jan 23, 2026",
  },
  {
    id: "11",
    txid: "TX-2026-011",
    counterparty: "Mullvad VPN",
    counterpartyInitials: "MU",
    tags: ["Subscription", "Privacy"],
    status: "confirmed",
    amount: 96.0,
    date: "Jan 23, 2026",
  },
  {
    id: "12",
    txid: "TX-2026-012",
    counterparty: "OpenSats",
    counterpartyInitials: "OS",
    tags: ["Donation"],
    status: "confirmed",
    amount: 250.0,
    date: "Jan 22, 2026",
  },
  {
    id: "13",
    txid: "TX-2026-013",
    counterparty: "Bitrefill",
    counterpartyInitials: "BR",
    tags: ["Gift card"],
    status: "confirmed",
    amount: 199.0,
    date: "Jan 22, 2026",
  },
  {
    id: "14",
    txid: "TX-2026-014",
    counterparty: "Hardware Wallet",
    counterpartyInitials: "HW",
    tags: ["Cold storage move"],
    status: "review",
    amount: 12498.0,
    date: "Jan 21, 2026",
  },
  {
    id: "15",
    txid: "TX-2026-015",
    counterparty: "River Financial",
    counterpartyInitials: "RF",
    tags: ["Recurring buy", "DCA"],
    status: "confirmed",
    amount: 648.0,
    date: "Jan 21, 2026",
  },
  {
    id: "16",
    txid: "TX-2026-016",
    counterparty: "Strike",
    counterpartyInitials: "SK",
    tags: ["Auto-buy"],
    status: "pending",
    amount: 249.0,
    date: "Jan 20, 2026",
  },
  {
    id: "17",
    txid: "TX-2026-017",
    counterparty: "Lightning Labs",
    counterpartyInitials: "LL",
    tags: ["Service payment"],
    status: "confirmed",
    amount: 399.0,
    date: "Jan 20, 2026",
  },
  {
    id: "18",
    txid: "TX-2026-018",
    counterparty: "Mobile Wallet",
    counterpartyInitials: "MW",
    tags: ["Tip jar"],
    status: "confirmed",
    amount: 42.0,
    date: "Jan 19, 2026",
  },
  {
    id: "19",
    txid: "TX-2026-019",
    counterparty: "Coinbase",
    counterpartyInitials: "CB",
    tags: ["Withdrawal"],
    status: "failed",
    amount: 448.0,
    date: "Jan 19, 2026",
  },
  {
    id: "20",
    txid: "TX-2026-020",
    counterparty: "Project Treasury",
    counterpartyInitials: "PT",
    tags: ["Reimbursement"],
    status: "review",
    amount: 1299.0,
    date: "Jan 18, 2026",
  },
];

export const readinessToneStyles: Record<OverviewHealthTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};

export const healthToneStyles: Record<OverviewHealthTone, string> = {
  good: "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

export const statusStyles: Record<TransactionStatus, string> = {
  confirmed:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  pending:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  review:
    "bg-blue-50 text-blue-700 ring-1 ring-inset ring-blue-700/10 dark:bg-blue-900/30 dark:text-blue-400 dark:ring-blue-400/20",
  failed:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/10 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
};

export const statusLabels: Record<TransactionStatus, string> = {
  confirmed: "Confirmed",
  pending: "Pending",
  review: "Review",
  failed: "Failed",
};

export const overviewFlowLabels: Record<OverviewTransactionFlow, string> = {
  incoming: "Incoming",
  outgoing: "Outgoing",
  transfer: "Transfer",
  swap: "Swap",
};

export const overviewFlowStyles: Record<OverviewTransactionFlow, string> = {
  incoming:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  outgoing:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  transfer:
    "bg-zinc-50 text-zinc-700 ring-1 ring-inset ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
  swap: "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/25 dark:text-sky-300 dark:ring-sky-400/20",
};

export function flowForOverviewTx(tx: OverviewTx): OverviewTransactionFlow {
  if (
    tx.internal ||
    tx.type === "Transfer" ||
    tx.type === "Consolidation" ||
    tx.type === "Rebalance"
  ) {
    return "transfer";
  }
  if (tx.type === "Swap" || tx.type === "Mint" || tx.type === "Melt") {
    return "swap";
  }
  return tx.amountSat >= 0 ? "incoming" : "outgoing";
}

export function toDashboardTransaction(tx: OverviewTx, index: number): Transaction {
  const amount =
    tx.eur !== null
      ? tx.eur
      : tx.rate !== null
        ? (tx.amountSat / 100_000_000) * tx.rate
        : null;
  const account = tx.account || tx.counter || "Unassigned";
  const accountLower = account.toLowerCase();
  const paymentMethod = accountLower.includes("liquid")
    ? "Liquid"
    : accountLower.includes("lightning") ||
        accountLower.includes("ln") ||
        accountLower.includes("cln") ||
        accountLower.includes("phoenix")
      ? "Lightning"
      : accountLower.includes("on-chain") ||
          accountLower.includes("xpub") ||
          accountLower.includes("cold") ||
          accountLower.includes("vault") ||
          accountLower.includes("multisig")
        ? "On-chain"
        : "Other";
  const status: TransactionStatus = tx.internal
    ? "pending"
    : tx.conf > 0
      ? "confirmed"
      : tx.tag.toLowerCase().includes("review")
        ? "review"
        : "pending";
  return {
    id: tx.id,
    txid: tx.externalId || tx.id || `TX-${index + 1}`,
    explorerId: tx.explorerId || undefined,
    counterparty: account,
    counterpartyInitials: initials(account || "TX"),
    paymentMethod,
    tags: tx.tag
      ? tx.tag
          .split(",")
          .map((tag) => tag.trim())
          .filter(Boolean)
      : [tx.type],
    status,
    flow: flowForOverviewTx(tx),
    amount,
    amountBtc: tx.amountSat / 100_000_000,
    date: tx.date,
  };
}

export function initials(value: string) {
  return value
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
}

export function transactionDetailHref(transactionId: string) {
  const params = new URLSearchParams();
  if (typeof window !== "undefined") {
    const currentParams = new URLSearchParams(window.location.search);
    const period = currentParams.get("period");
    if (period) params.set("period", period);
  }
  params.set("tx", transactionId);
  return `/transactions?${params.toString()}`;
}

export function buildOverviewReadiness(snapshot: OverviewSnapshot): OverviewReadiness {
  const status = snapshot.status;
  const needsJournals = Boolean(status?.needsJournals);
  const quarantines = status?.quarantines ?? 0;
  const totalConnections = snapshot.connections.length;
  const syncedConnections = snapshot.connections.filter(
    (connection) => connection.status === "synced",
  ).length;
  const syncingConnections = snapshot.connections.filter(
    (connection) => connection.status === "syncing",
  ).length;
  const erroredConnections = snapshot.connections.filter(
    (connection) => connection.status === "error",
  ).length;
  const sourceDetail = totalConnections
    ? `${syncedConnections}/${totalConnections} source${
        totalConnections === 1 ? "" : "s"
      } current`
    : "No sources connected";

  if (!snapshot.txs.length && !totalConnections) {
    return {
      title: "Connect a source",
      detail: "Add a watch-only source or import rows to populate this book.",
      icon: Plus,
      tone: "neutral",
    };
  }

  if (erroredConnections) {
    return {
      title: "Source attention",
      detail: `${erroredConnections} source${
        erroredConnections === 1 ? "" : "s"
      } needs attention`,
      icon: WalletCards,
      tone: "alert",
    };
  }

  if (needsJournals) {
    return {
      title: "Reprocess journals",
      detail: "Reports need a fresh journal state",
      icon: RefreshCw,
      tone: "warning",
    };
  }

  if (quarantines > 0) {
    return {
      title: "Review queue open",
      detail: `${quarantines} item${
        quarantines === 1 ? "" : "s"
      } before reports`,
      icon: ShieldAlert,
      tone: "alert",
    };
  }

  if (syncingConnections) {
    return {
      title: "Sync in progress",
      detail: sourceDetail,
      icon: RefreshCw,
      tone: "warning",
    };
  }

  return {
    title: "Ready for reports",
    detail: sourceDetail,
    icon: CheckCircle2,
    tone: "good",
  };
}

export function buildOverviewHealthItems(snapshot: OverviewSnapshot): OverviewHealthItem[] {
  const status = snapshot.status;
  const needsJournals = Boolean(status?.needsJournals);
  const quarantines = status?.quarantines ?? 0;
  const totalConnections = snapshot.connections.length;
  const syncingConnections = snapshot.connections.filter(
    (connection) => connection.status === "syncing",
  ).length;
  const erroredConnections = snapshot.connections.filter(
    (connection) => connection.status === "error",
  ).length;
  const syncedConnections = snapshot.connections.filter(
    (connection) => connection.status === "synced",
  ).length;

  return [
    {
      key: "journals",
      title: "Journal state",
      value: needsJournals ? "Reprocess" : "Current",
      detail: needsJournals
        ? "Reports should wait for a fresh journal run."
        : "Reports are ready from the current journal state.",
      href: "/journals",
      icon: needsJournals ? RefreshCw : CheckCircle2,
      tone: needsJournals ? "warning" : "good",
    },
    {
      key: "review",
      title: "Review queue",
      value: quarantines ? `${quarantines} open` : "Clear",
      detail: quarantines
        ? "Resolve quarantined rows before tax reporting."
        : "No quarantined transactions in this book.",
      href: "/quarantine",
      icon: quarantines ? ShieldAlert : CheckCircle2,
      tone: quarantines ? "alert" : "good",
    },
    {
      key: "connections",
      title: "Connections",
      value: erroredConnections
        ? `${erroredConnections} issue${erroredConnections === 1 ? "" : "s"}`
        : syncingConnections
          ? `${syncingConnections} refreshing`
          : totalConnections
            ? `${syncedConnections}/${totalConnections} current`
            : "None yet",
      detail: totalConnections
        ? `${totalConnections} configured source${totalConnections === 1 ? "" : "s"}`
        : "Add a watch-only source, exchange, or import source.",
      href: "/connections",
      icon: WalletCards,
      tone: erroredConnections
        ? "alert"
        : syncingConnections
          ? "warning"
          : totalConnections
            ? "good"
            : "neutral",
    },
  ];
}

export function buildPrimaryOverviewAction(snapshot: OverviewSnapshot) {
  const status = snapshot.status;
  if (status?.needsJournals) {
    return null;
  }
  if ((status?.quarantines ?? 0) > 0) {
    return {
      title: "Review quarantines",
      detail: "Clear missing prices or unsupported semantics first.",
      href: "/quarantine",
      icon: ShieldAlert,
      tone: "alert" as const,
    };
  }
  if (snapshot.connections.some((connection) => connection.status === "error")) {
    return {
      title: "Check connections",
      detail: "One or more sources need attention before the next sync.",
      href: "/connections",
      icon: WalletCards,
      tone: "alert" as const,
    };
  }
  return {
    title: snapshot.txs.length ? "Open reports" : "Add a connection",
    detail: snapshot.txs.length
      ? "Move from overview into the report package for the current book."
      : "Connect a source or import rows to start the book.",
    href: snapshot.txs.length ? "/reports" : "/connections",
    icon: snapshot.txs.length ? FileText : Plus,
    tone: "good" as const,
  };
}
