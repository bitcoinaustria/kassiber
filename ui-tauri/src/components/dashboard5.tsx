import { Link } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  ChevronLeft,
  ChevronRight,
  CheckCircle2,
  CircleDollarSign,
  ClipboardList,
  CreditCard,
  Filter,
  FileText,
  Maximize2,
  MoreHorizontal,
  PieChartIcon,
  Plus,
  RefreshCw,
  Settings,
  ShieldAlert,
  WalletCards,
  Users,
  X,
} from "lucide-react";
import * as React from "react";
import {
  Area,
  AreaChart,
  Brush,
  CartesianGrid,
  Cell,
  ComposedChart,
  Line,
  Pie,
  PieChart,
  ReferenceLine,
  Scatter,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";

import { Button } from "@/components/ui/button";
import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { Checkbox } from "@/components/ui/checkbox";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
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
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { formatBtc, useCurrency, type Currency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  MOCK_OVERVIEW,
  type OverviewSnapshot,
  type PortfolioPoint,
  type Tx as OverviewTx,
} from "@/mocks/seed";

type StatItem = {
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

type HoldingsItem = {
  name: string;
  value: number;
  percent: number;
  color: string;
};

type BalanceDriverItem = {
  key: "incoming" | "outgoing" | "swap" | "fees";
  label: string;
  valueBtc: number;
  count: number;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  toneClassName: string;
};

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
const MAX_ACTIVITY_MARKER_MIN_BTC = 0.01;
const ACTIVITY_MARKER_MIN_STEP_BTC = 0.00005;
const INCOMING_MARKER_MIN_PARAM = "incomingMinBtc";
const OUTGOING_MARKER_MIN_PARAM = "outgoingMinBtc";
const LEGACY_INCOMING_MARKER_MIN_PARAM = "incomingMin";
const LEGACY_OUTGOING_MARKER_MIN_PARAM = "outgoingMin";

const defaultTreasurySeriesVisibility: TreasurySeriesVisibility = {
  primary: true,
  price: true,
  basis: true,
  events: true,
};

type TreasuryChartPoint = PortfolioChartPoint & {
  bitcoinPriceEur: number;
  avgCostEur: number | null;
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

type TransactionStatus = "confirmed" | "pending" | "review" | "failed";
type OverviewTransactionFlow = "incoming" | "outgoing" | "transfer" | "swap";
type OverviewHealthTone = "good" | "warning" | "alert" | "neutral";
type OverviewHref =
  | "/connections"
  | "/journals"
  | "/quarantine"
  | "/reports"
  | "/transactions";

type Transaction = {
  id: string;
  txid: string;
  explorerId?: string;
  counterparty: string;
  counterpartyInitials: string;
  paymentMethod?: "On-chain" | "Lightning" | "Liquid" | "Other";
  tags: string[];
  status: TransactionStatus;
  flow?: OverviewTransactionFlow;
  amount: number;
  amountBtc?: number;
  date: string;
};

type OverviewHealthItem = {
  key: string;
  title: string;
  value: string;
  detail: string;
  href: OverviewHref;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

type OverviewReadiness = {
  title: string;
  detail: string;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

const currencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
});

const numberFormatter = new Intl.NumberFormat("en-US");

const compactCurrencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
  notation: "compact",
  maximumFractionDigits: 0,
});

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

function btcFromEur(eur: number, priceEur: number) {
  return priceEur ? eur / priceEur : 0;
}

function formatDisplayMoney(eur: number, priceEur: number, currency: Currency) {
  if (currency === "btc") return formatBtc(btcFromEur(eur, priceEur));
  return currencyFormatter.format(eur);
}

function formatSignedDisplayMoney(
  eur: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") {
    return formatBtc(btcFromEur(eur, priceEur), { sign: true });
  }
  const prefix = eur >= 0 ? "+ " : "− ";
  return `${prefix}${currencyFormatter.format(Math.abs(eur))}`;
}

function formatCompactDisplayMoney(
  eur: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") {
    return formatBtc(btcFromEur(eur, priceEur), { precision: 3 });
  }
  return compactCurrencyFormatter.format(eur);
}

function formatPortfolioMoney(
  amount: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") return formatBtc(amount);
  return formatDisplayMoney(amount, priceEur, currency);
}

function formatDriverValue(btc: number, priceEur: number, currency: Currency) {
  if (currency === "btc") {
    return formatBtc(btc, { precision: btc > 0 && btc < 0.001 ? 8 : 3 });
  }
  return formatCompactDisplayMoney(btc * priceEur, priceEur, currency);
}

function formatDetailedPortfolioMoney(
  amount: number,
  priceEur: number,
  currency: Currency,
) {
  if (currency === "btc") {
    return formatBtc(amount, { precision: Math.abs(amount) < 0.01 ? 8 : 4 });
  }
  return formatDisplayMoney(amount, priceEur, currency);
}

function donutCenterValueClass(value: string) {
  const length = value.replace(/\s+/g, "").length;
  if (length <= 7) return "text-sm sm:text-base";
  if (length <= 9) return "text-xs sm:text-sm";
  if (length <= 11) return "text-[11px] sm:text-xs";
  return "text-[10px] sm:text-[11px]";
}

function transactionBtc(tx: Transaction, priceEur: number) {
  return tx.amountBtc ?? btcFromEur(tx.amount, priceEur);
}

function satToBtc(sats: number | undefined) {
  return (sats ?? 0) / 100_000_000;
}

/**
 * Custom hook for hover highlight interaction.
 * Provides stable callback to prevent unnecessary re-renders in chart components.
 */
function useHoverHighlight<T extends string | number>() {
  const [active, setActive] = React.useState<T | null>(null);

  const handleHover = React.useCallback((value: T | null) => {
    setActive(value);
  }, []);

  return { active, handleHover };
}

const mixBase = "var(--background)";

const palette = {
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

const portfolioChartColors = {
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

function useResolvedColorMode() {
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

const holdingsChartConfig = {
  onchain: { label: "On-chain BTC", color: palette.primary },
  lightning: { label: "Lightning", theme: palette.secondary },
  liquid: { label: "Liquid", theme: palette.tertiary },
  other: { label: "Other", theme: palette.quaternary },
} satisfies ChartConfig;

const statsData: StatItem[] = [
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

function latestPortfolioBalanceBtc(snapshot: OverviewSnapshot) {
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

function buildStatsData(
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
  const clamped = Math.min(Math.max(value, 0), MAX_ACTIVITY_MARKER_MIN_BTC);
  return (
    Math.round(clamped / ACTIVITY_MARKER_MIN_STEP_BTC) *
    ACTIVITY_MARKER_MIN_STEP_BTC
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

function formatActivityMarkerMinimum(value: number) {
  return `${serializeActivityMarkerMinimum(value)} BTC`;
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
    window.location.href = transactionDetailHref(transactionId);
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
      const valueEur = Math.abs(tx.eur);
      const priceEur =
        valueEur > 0 && btc > 0 ? valueEur / btc : tx.rate || snapshot.priceEur;
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
  const balanceBtc =
    event.tx.balanceBtc ?? anchor?.balanceBtc ?? latestPortfolioBalanceBtc(snapshot);
  const costBasisEur =
    event.tx.costBasisEur ?? anchor?.costBasisEur ?? snapshot.fiat.eurCostBasis;
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
  const chartDisplayData = plottedData.map((point) => {
    if (!point.isActivityEvent || visibleActivityMarkerIds.has(point.date)) {
      return point;
    }
    return {
      ...point,
      eventBalanceBtc: undefined,
      eventFlow: undefined,
      eventSize: 0,
      isActivityEvent: false,
    };
  });
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

function percentOf(value: number, total: number) {
  if (total <= 0) return 0;
  return Math.round((value / total) * 100);
}

type BalanceRail = "onchain" | "lightning" | "liquid" | "other";

function railForConnection(kind: string, label: string): BalanceRail {
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

function buildBalanceRailItems(snapshot: OverviewSnapshot) {
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

function buildHoldingsBySource(snapshot: OverviewSnapshot): HoldingsItem[] {
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

function buildBalanceDrivers(snapshot: OverviewSnapshot) {
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

function transactionsDriverSearch(driver: BalanceDriverItem["key"]) {
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

const transactionStatuses: TransactionStatus[] = [
  "confirmed",
  "pending",
  "review",
  "failed",
];

const transactionRecords: Transaction[] = [
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

const readinessToneStyles: Record<OverviewHealthTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};

const WelcomeSection = ({
  onAddConnection,
  onRefresh,
  onProcessJournals,
  isRefreshing,
  isProcessingJournals,
  snapshot,
}: {
  onAddConnection: () => void;
  onRefresh: () => void;
  onProcessJournals: () => void;
  isRefreshing: boolean;
  isProcessingJournals: boolean;
  snapshot: OverviewSnapshot;
}) => {
  const readiness = buildOverviewReadiness(snapshot);
  const ReadinessIcon = readiness.icon;
  const needsJournals = Boolean(snapshot.status?.needsJournals);
  const readinessClassName = cn(
    "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
    readinessToneStyles[readiness.tone],
  );

  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-center sm:justify-between sm:gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {needsJournals ? (
          <button
            type="button"
            className={cn(
              readinessClassName,
              "transition-colors hover:bg-amber-500/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60",
            )}
            onClick={onProcessJournals}
            disabled={isProcessingJournals}
          >
            <ReadinessIcon
              className={cn("size-4", isProcessingJournals && "animate-spin")}
              aria-hidden="true"
            />
            {isProcessingJournals ? "Reprocessing journals" : readiness.title}
          </button>
        ) : (
          <span className={readinessClassName}>
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readiness.title}
          </span>
        )}
        <span className="min-w-0 truncate text-xs text-muted-foreground sm:text-sm">
          {readiness.detail}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2 sm:gap-3">
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-2"
          aria-label="Refresh wallets and journals"
          onClick={onRefresh}
          disabled={isRefreshing}
        >
          <RefreshCw
            className={cn("size-4", isRefreshing && "animate-spin")}
            aria-hidden="true"
          />
          <span className="hidden sm:inline">
            {isRefreshing ? "Refreshing" : "Refresh"}
          </span>
        </Button>
        <Button
          size="sm"
          className="h-8 gap-2"
          aria-label="Add connection"
          onClick={onAddConnection}
        >
          <Plus className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">Add connection</span>
        </Button>
      </div>
    </div>
  );
};

const StatsCards = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const stats = buildStatsData(snapshot, currency);
  return (
    <div className="rounded-xl border bg-card">
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4 lg:divide-x">
        {stats.map((stat) => {
          const formatter =
            stat.format === "currency" ? currencyFormatter : numberFormatter;
          const hasComparison = stat.previousValue > 0;
          const statusText = hasComparison
            ? `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`
            : stat.value === 0
              ? "Clear"
              : stat.title === "Portfolio value"
                ? "Estimate"
                : stat.title === "Transactions"
                  ? "Loaded"
                  : stat.title === "Connections"
                    ? "Configured"
                    : "Open";
          const isBitcoinPortfolio =
            currency === "btc" && stat.title === "Portfolio value";

          return (
            <div
              key={stat.title}
              className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/60 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100 sm:p-4"
            >
              <Link
                to={stat.href}
                className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={`Open ${isBitcoinPortfolio ? "Bitcoin balance" : stat.title}`}
              />
              <div className="pointer-events-none relative z-20 space-y-2">
                <div className="text-muted-foreground">
                  <span className="text-xs font-medium">
                    {isBitcoinPortfolio ? "Bitcoin balance" : stat.title}
                  </span>
                </div>
                <p
                  className={cn(
                    "text-xl font-semibold tracking-tight",
                    blurClass(hideSensitive),
                  )}
                >
                  {isBitcoinPortfolio ? (
                    <span>
                      {formatBtc(latestPortfolioBalanceBtc(snapshot), {
                        precision: 3,
                      })}
                    </span>
                  ) : stat.format === "currency" ? (
                    <span>
                      {formatCompactDisplayMoney(
                        stat.value,
                        snapshot.priceEur,
                        currency,
                      )}
                    </span>
                  ) : (
                    formatter.format(stat.value)
                  )}
                </p>
                <div className="flex flex-wrap items-center gap-2 text-[10px] sm:text-xs xl:flex-nowrap">
                  <span
                    className={cn(
                      "font-medium",
                      stat.isPositive
                        ? "text-emerald-600 dark:text-emerald-400"
                        : "text-red-600 dark:text-red-400",
                      blurClass(hideSensitive),
                    )}
                  >
                    {statusText}
                    {hasComparison && (
                      <span className="hidden sm:inline">
                        (
                        {stat.format === "currency"
                          ? formatCompactDisplayMoney(
                              Math.abs(stat.value - stat.previousValue),
                              snapshot.priceEur,
                              currency,
                            )
                          : formatter.format(
                              Math.abs(stat.value - stat.previousValue),
                            )}
                        )
                      </span>
                    )}
                  </span>
                  <span className="hidden items-center gap-2 text-muted-foreground sm:inline-flex">
                    <span className="size-1 rounded-full bg-muted-foreground" />
                    <span className="xl:whitespace-nowrap">
                      {stat.comparisonLabel}
                    </span>
                  </span>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const BalanceDriversCard = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const { rows, maxValueBtc, netBtc, transactionCount } =
    buildBalanceDrivers(snapshot);
  const netEur = netBtc * snapshot.priceEur;
  const netTone =
    netBtc > 0
      ? "text-emerald-700 dark:text-emerald-300"
      : netBtc < 0
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";

  return (
    <div className="flex flex-col gap-3 rounded-xl border bg-card p-3 sm:p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Balance drivers"
          >
            <ArrowLeftRight className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium">Balance Drivers</span>
            <p
              className={cn(
                "text-[10px] text-muted-foreground sm:text-xs",
                blurClass(hideSensitive),
              )}
            >
              Latest {transactionCount.toLocaleString("en-US")} transactions
            </p>
          </div>
        </div>
        <CurrencyToggleText
          className={cn(
            "text-right text-sm font-semibold tabular-nums",
            netTone,
            blurClass(hideSensitive),
          )}
        >
          {formatSignedDisplayMoney(netEur, snapshot.priceEur, currency)}
        </CurrencyToggleText>
      </div>

      <div className="space-y-2.5">
        {rows.map((item) => {
          const Icon = item.icon;
          const width =
            maxValueBtc > 0 ? Math.max((item.valueBtc / maxValueBtc) * 100, 4) : 0;
          return (
            <Link
              key={item.key}
              to="/transactions"
              search={transactionsDriverSearch(item.key)}
              className="-mx-1 grid gap-1.5 rounded-md px-1 py-1 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label={`View ${item.label.toLowerCase()} transactions`}
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  <Icon className={cn("size-3.5 shrink-0", item.toneClassName)} />
                  <span className="truncate text-xs text-muted-foreground">
                    {item.label}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {item.count}
                  </span>
                </div>
                <span
                  className={cn(
                    "shrink-0 text-xs font-medium tabular-nums",
                    item.toneClassName,
                    blurClass(hideSensitive),
                  )}
                >
                  {formatDriverValue(item.valueBtc, snapshot.priceEur, currency)}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className={cn(
                    "h-full rounded-full",
                    item.key === "incoming"
                      ? "bg-emerald-500"
                      : item.key === "outgoing"
                        ? "bg-red-500"
                        : item.key === "swap"
                          ? "bg-sky-500"
                          : "bg-muted-foreground/60",
                  )}
                  style={{ width: `${width}%` }}
                />
              </div>
            </Link>
          );
        })}
      </div>
    </div>
  );
};

const HoldingsBySourceChart = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const isBitcoinMode = currency === "btc";
  const { active: activeSlice, handleHover: setHoveredSlice } =
    useHoverHighlight<number>();
  const holdingsData = buildHoldingsBySource(snapshot);
  const { items: balanceRailItems } = buildBalanceRailItems(snapshot);
  const unrealizedPercent = snapshot.fiat.eurCostBasis
    ? (snapshot.fiat.eurUnrealized / snapshot.fiat.eurCostBasis) * 100
    : 0;
  const totalHoldings = holdingsData.reduce(
    (acc, item) => acc + item.value,
    0,
  );
  const totalHoldingsLabel = formatCompactDisplayMoney(
    totalHoldings,
    snapshot.priceEur,
    currency,
  );
  const singleHolding = holdingsData.length === 1 ? holdingsData[0] : null;

  return (
    <div className="flex flex-1 flex-col gap-3 rounded-xl border bg-card p-3 sm:p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Holdings by source"
          >
            <PieChartIcon className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div className="min-w-0">
            <span className="text-sm font-medium">
              Holdings by Source
            </span>
            {isBitcoinMode ? (
              <p className="text-[10px] text-muted-foreground sm:text-xs">
                BTC allocation
              </p>
            ) : (
              <p className="flex items-center gap-1 text-[10px] text-muted-foreground sm:text-xs">
                <ArrowUpRight
                  className={cn(
                    "size-3",
                    unrealizedPercent >= 0
                      ? "text-emerald-600"
                      : "text-red-600",
                  )}
                  aria-hidden="true"
                />
                <span
                  className={cn(
                    unrealizedPercent >= 0
                      ? "text-emerald-600"
                      : "text-red-600",
                    blurClass(hideSensitive),
                  )}
                >
                  {unrealizedPercent >= 0 ? "+" : ""}
                  {unrealizedPercent.toFixed(1)}%
                </span>
                <span>vs cost basis</span>
              </p>
            )}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="size-7 sm:size-8"
          aria-label="More options"
        >
          <MoreHorizontal className="size-4 text-muted-foreground" />
        </Button>
      </div>

      {balanceRailItems.length > 1 ? (
        <div className="flex flex-wrap gap-1.5">
          {balanceRailItems.map((item) => (
            <span
              key={item.key}
              className="inline-flex items-center gap-1 rounded-md border bg-background/55 px-1.5 py-0.5 text-[10px] text-muted-foreground"
            >
              <span
                className="size-1.5 rounded-full"
                style={{ backgroundColor: item.color }}
              />
              {item.label}
              <span
                className={cn("tabular-nums", blurClass(hideSensitive))}
              >
                {item.percent}%
              </span>
            </span>
          ))}
        </div>
      ) : null}

      {singleHolding ? (
        <div className="flex flex-1 items-center rounded-md bg-muted/25 px-3 py-3">
          <div
            className="flex min-w-0 flex-1 items-start gap-2"
            onMouseEnter={() => setHoveredSlice(0)}
            onMouseLeave={() => setHoveredSlice(null)}
          >
            <span
              className="mt-1 size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: singleHolding.color }}
            />
            <div className="min-w-0">
              <p className="break-words text-sm font-medium leading-5">
                {singleHolding.name}
              </p>
              <p className="text-[10px] text-muted-foreground sm:text-xs">
                Only active source
              </p>
            </div>
          </div>
          <div className="ml-3 shrink-0 text-right">
            <p
              className={cn(
                "text-sm font-semibold tabular-nums",
                blurClass(hideSensitive),
              )}
            >
              {formatCompactDisplayMoney(
                singleHolding.value,
                snapshot.priceEur,
                currency,
              )}
            </p>
            <p
              className={cn(
                "text-[10px] text-muted-foreground tabular-nums sm:text-xs",
                blurClass(hideSensitive),
              )}
            >
              {singleHolding.percent}%
            </p>
          </div>
        </div>
      ) : (
      <div className="grid flex-1 items-center gap-3 sm:grid-cols-[minmax(104px,0.85fr)_minmax(0,1.15fr)]">
        <div className="relative mx-auto size-[116px] shrink-0 sm:size-[128px] xl:size-[136px]">
          <ChartContainer
            config={holdingsChartConfig}
            className="h-full w-full"
          >
            <PieChart>
              <Pie
                data={holdingsData}
                cx="50%"
                cy="50%"
                innerRadius="55%"
                outerRadius="90%"
                paddingAngle={2}
                dataKey="value"
                strokeWidth={0}
                onMouseEnter={(_: unknown, index: number) =>
                  setHoveredSlice(index)
                }
                onMouseLeave={() => setHoveredSlice(null)}
              >
                {holdingsData.map((entry) => (
                  <Cell key={entry.name} fill={entry.color} />
                ))}
              </Pie>
            </PieChart>
          </ChartContainer>
          <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
            <span
              className={cn(
                "max-w-[96px] whitespace-nowrap text-center leading-tight font-semibold tabular-nums sm:max-w-[116px]",
                donutCenterValueClass(totalHoldingsLabel),
                blurClass(hideSensitive),
              )}
            >
              {totalHoldingsLabel}
            </span>
            <span className="text-[8px] text-muted-foreground sm:text-[10px]">
              Total
            </span>
          </div>
        </div>

        <div className="flex min-w-0 flex-col gap-2 sm:gap-3">
          {holdingsData.map((item, index) => (
            <div
              key={item.name}
              className={cn(
                "flex items-start justify-between gap-2 transition-opacity duration-200 motion-reduce:transition-none",
                activeSlice !== null && activeSlice !== index && "opacity-50",
              )}
              onMouseEnter={() => setHoveredSlice(index)}
              onMouseLeave={() => setHoveredSlice(null)}
            >
              <div className="flex min-w-0 flex-1 items-start gap-2">
                <div
                  className="mt-1 size-2 shrink-0 rounded-full sm:size-2.5"
                  style={{ backgroundColor: item.color }}
                />
                <span className="min-w-0 break-words text-[10px] leading-4 text-muted-foreground sm:text-xs">
                  {item.name}
                </span>
              </div>
              <div className="flex shrink-0 flex-wrap justify-end gap-x-1.5 text-[10px] sm:text-xs">
                <span
                  className={cn(
                    "font-medium tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatCompactDisplayMoney(
                    item.value,
                    snapshot.priceEur,
                    currency,
                  )}
                </span>
                <span
                  className={cn(
                    "text-muted-foreground tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {item.percent}%
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
      )}
    </div>
  );
};

const SideChartsSection = ({
  className,
  snapshot,
  hideSensitive,
  currency,
}: {
  className?: string;
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  return (
    <div
      className={cn(
        "grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-1",
        className,
      )}
    >
      <BalanceDriversCard
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
      />
      <HoldingsBySourceChart
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
      />
    </div>
  );
};

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
  hideSensitive: boolean;
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
  hideSensitive,
}: ChartControlsSheetProps) {
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

            <div className="rounded-md border p-3">
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
                <span
                  className={cn("font-medium tabular-nums", blurClass(hideSensitive))}
                >
                  {formatActivityMarkerMinimum(incomingMarkerMinimumBtc)}
                </span>
              </div>
              <input
                aria-label="Minimum incoming payment dot size in BTC"
                className="mt-3 h-2 w-full cursor-pointer"
                min={0}
                max={MAX_ACTIVITY_MARKER_MIN_BTC}
                step={ACTIVITY_MARKER_MIN_STEP_BTC}
                type="range"
                value={incomingMarkerMinimumBtc}
                style={{ accentColor: activityFlowColors.incoming }}
                onChange={(event) =>
                  onIncomingMarkerMinimumChange(Number(event.currentTarget.value))
                }
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
                <span
                  className={cn(
                    "font-medium tabular-nums text-red-500 dark:text-red-400",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatActivityMarkerMinimum(outgoingMarkerMinimumBtc)}
                </span>
              </div>
              <input
                aria-label="Minimum outgoing activity dot size in BTC"
                className="mt-3 h-2 w-full cursor-pointer"
                min={0}
                max={MAX_ACTIVITY_MARKER_MIN_BTC}
                step={ACTIVITY_MARKER_MIN_STEP_BTC}
                type="range"
                value={outgoingMarkerMinimumBtc}
                style={{ accentColor: activityFlowColors.outgoing }}
                onChange={(event) =>
                  onOutgoingMarkerMinimumChange(Number(event.currentTarget.value))
                }
              />
            </div>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}

const RevenueFlowChart = ({
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

  const renderChartCard = (expanded = false) => {
    const plottedData = expanded ? expandedChartData : chartData;
    const markerView = expanded ? expandedMarkerView : compactMarkerView;
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
    const visibleLatestReserve = snapshot.fiat.eurBalance;
    const visibleCostBasis = snapshot.fiat.eurCostBasis;
    const latestBalanceBtc = latestPortfolioBalanceBtc(snapshot);
    const visibleAvgCost =
      latestBalanceBtc > 0 ? visibleCostBasis / latestBalanceBtc : (latestPoint?.avgCostEur ?? 0);
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
    const chartStats = buildTreasuryChartStats(plottedData);
    const statPeriodLabel = periodShortLabels[period];
    const selectedPoint = expanded
      ? (plottedData.find((point) => point.date === expandedPointDate) ??
        plottedData.at(-1) ??
        null)
      : null;
    const selectedPointIndex = selectedPoint
      ? plottedData.findIndex((point) => point.date === selectedPoint.date)
      : -1;
    const previousPoint =
      selectedPointIndex > 0 ? plottedData[selectedPointIndex - 1] : null;
    const handleExpandedChartMove = (state: PortfolioChartMouseState) => {
      if (!expanded) return;
      const point = state.activePayload?.find((item) => item.payload)?.payload;
      if (point) setExpandedPointDate(point.date);
    };
    const balancePoints = plottedData.filter((point) => !point.isActivityEvent);
    const xAxisTicks = portfolioAxisTicks(
      balancePoints.length ? balancePoints : plottedData,
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
                </span>{" "}
                net
              </span>
              <span>
                Received{" "}
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {formatBtc(receivedBtc, { precision: 4 })}
                </span>
              </span>
              <span>
                Spent{" "}
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {formatBtc(spentBtc, { precision: 4 })}
                </span>
              </span>
              {swapBtc > 0 && (
                <span>
                  Swapped{" "}
                  <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                    {formatBtc(swapBtc, { precision: 4 })}
                  </span>
                </span>
              )}
              {feeBtc > 0 && (
                <span>
                  Fees{" "}
                  <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                    {formatBtc(feeBtc, { precision: 8 })}
                  </span>
                </span>
              )}
              <span>
                Avg basis{" "}
                <span className={cn("font-semibold text-foreground", blurClass(hideSensitive))}>
                  {formatEurPrice(visibleAvgCost)}
                </span>
              </span>
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

        <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
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
              <ChartContainer
                config={chartConfig}
                className="h-full min-h-0 w-full overflow-visible [&_.recharts-tooltip-wrapper]:!z-[100]"
              >
                <ComposedChart
                  data={chartDisplayData}
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
                    left: expanded ? 8 : 4,
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
                    tick={{ fontSize: 10 }}
                    tickMargin={4}
                    tickFormatter={(value) =>
                      hideSensitive ? "" : formatBtcAxis(Number(value))
                    }
                    width={48}
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
                    allowEscapeViewBox={{ x: true, y: true }}
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
                      dataKey="balanceBtc"
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
                      isAnimationActive={false}
                    />
                  )}
                  {seriesVisible.price && (
                    <Line
                      yAxisId="price"
                      type="linear"
                      dataKey="bitcoinPriceEur"
                      name={legendItems[3]?.label}
                      stroke="#94a3b8"
                      strokeWidth={activeSeries === "price" ? 2.4 : 1.6}
                      strokeDasharray="3 5"
                      strokeOpacity={
                        activeSeries === null || activeSeries === "price" ? 0.72 : 0.2
                      }
                      dot={false}
                      activeDot={expanded ? { r: 3 } : { r: 2 }}
                      isAnimationActive={false}
                    />
                  )}
                  {seriesVisible.basis && (
                    <Line
                      yAxisId="price"
                      type="stepAfter"
                      dataKey="avgCostEur"
                      name={legendItems[2]?.label}
                      connectNulls={false}
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
                  {plottedData.length > 3 && (
                    <Brush
                      className="text-muted-foreground"
                      dataKey="date"
                      fill="rgba(12, 10, 8, 0.78)"
                      height={expanded ? 60 : 74}
                      padding={{ top: 8, right: 1, bottom: 8, left: 1 }}
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
                          type="monotone"
                          dataKey="bitcoinPriceEur"
                          stroke={primaryColor}
                          strokeWidth={1.35}
                          fill={`url(#${brushGradientId})`}
                          fillOpacity={1}
                          dot={false}
                          isAnimationActive={false}
                        />
                      </AreaChart>
                    </Brush>
                  )}
                </ComposedChart>
              </ChartContainer>
              <div className="pointer-events-none flex items-center justify-center">
                <span className="rotate-90 whitespace-nowrap text-[10px] font-semibold text-muted-foreground">
                  BTC Price (EUR)
                </span>
              </div>
            </div>
            {plottedData.length > 3 && (
              <p className="pt-1 text-center text-[10px] text-muted-foreground">
                Drag the handles or selection area to reframe the time period
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

const statusStyles: Record<TransactionStatus, string> = {
  confirmed:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  pending:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  review:
    "bg-blue-50 text-blue-700 ring-1 ring-inset ring-blue-700/10 dark:bg-blue-900/30 dark:text-blue-400 dark:ring-blue-400/20",
  failed:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/10 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
};

const statusLabels: Record<TransactionStatus, string> = {
  confirmed: "Confirmed",
  pending: "Pending",
  review: "Review",
  failed: "Failed",
};

const overviewFlowLabels: Record<OverviewTransactionFlow, string> = {
  incoming: "Incoming",
  outgoing: "Outgoing",
  transfer: "Transfer",
  swap: "Swap",
};

const overviewFlowStyles: Record<OverviewTransactionFlow, string> = {
  incoming:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  outgoing:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  transfer:
    "bg-zinc-50 text-zinc-700 ring-1 ring-inset ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
  swap: "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/25 dark:text-sky-300 dark:ring-sky-400/20",
};

function flowForOverviewTx(tx: OverviewTx): OverviewTransactionFlow {
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

function toDashboardTransaction(tx: OverviewTx, index: number): Transaction {
  const amount = tx.eur || (tx.amountSat / 100_000_000) * tx.rate;
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

function initials(value: string) {
  return value
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("");
}

function transactionDetailHref(transactionId: string) {
  const params = new URLSearchParams();
  if (typeof window !== "undefined") {
    const currentParams = new URLSearchParams(window.location.search);
    const period = currentParams.get("period");
    if (period) params.set("period", period);
  }
  params.set("tx", transactionId);
  return `/transactions?${params.toString()}`;
}

function buildOverviewReadiness(snapshot: OverviewSnapshot): OverviewReadiness {
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

function buildOverviewHealthItems(snapshot: OverviewSnapshot): OverviewHealthItem[] {
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

function buildPrimaryOverviewAction(snapshot: OverviewSnapshot) {
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

const RecentTransactionsTable = ({
  className,
  transactions,
  hideSensitive,
  currency,
  priceEur,
}: {
  className?: string;
  transactions: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  priceEur: number;
}) => {
  const [statusFilter, setStatusFilter] = React.useState<
    TransactionStatus | "all"
  >("all");
  const [currentPage, setCurrentPage] = React.useState(1);
  const [isHydrated, setIsHydrated] = React.useState(false);
  const pageSize = 6;

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const nextStatus = params.get("status");
    if (
      nextStatus &&
      (nextStatus === "all" ||
        transactionStatuses.includes(nextStatus as TransactionStatus))
    ) {
      setStatusFilter(nextStatus as TransactionStatus | "all");
    }
    const nextPage = Number(params.get("page"));
    if (!Number.isNaN(nextPage) && nextPage > 0) {
      setCurrentPage(nextPage);
    }
    setIsHydrated(true);
  }, []);

  const filteredTransactions = React.useMemo(() => {
    if (statusFilter === "all") return transactions;
    return transactions.filter((t) => t.status === statusFilter);
  }, [statusFilter, transactions]);

  const totalPages = Math.max(
    1,
    Math.ceil(filteredTransactions.length / pageSize),
  );

  const paginatedTransactions = React.useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    return filteredTransactions.slice(startIndex, startIndex + pageSize);
  }, [filteredTransactions, currentPage, pageSize]);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [statusFilter]);

  React.useEffect(() => {
    if (!isHydrated || typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    if (statusFilter !== "all") {
      params.set("status", statusFilter);
    } else {
      params.delete("status");
    }
    if (currentPage > 1) {
      params.set("page", String(currentPage));
    } else {
      params.delete("page");
    }
    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [statusFilter, currentPage, isHydrated]);

  const goToPage = (page: number) => {
    setCurrentPage(Math.max(1, Math.min(page, totalPages)));
  };

  const startRow = filteredTransactions.length
    ? (currentPage - 1) * pageSize + 1
    : 0;
  const endRow = Math.min(currentPage * pageSize, filteredTransactions.length);

  return (
    <>
      <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-3 pt-3 sm:px-4">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">
            Recent Transactions
          </span>
          <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {filteredTransactions.length}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <Button asChild variant="ghost" size="sm" className="h-8 sm:h-9">
            <Link to="/transactions">Show all</Link>
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1.5 sm:h-9 sm:gap-2"
              >
                <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Filter</span>
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[180px]">
              <DropdownMenuLabel>Filter by Status</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={statusFilter === "all"}
                onCheckedChange={() => setStatusFilter("all")}
              >
                All Statuses
              </DropdownMenuCheckboxItem>
              {transactionStatuses.map((status) => (
                <DropdownMenuCheckboxItem
                  key={status}
                  checked={statusFilter === status}
                  onCheckedChange={() => setStatusFilter(status)}
                >
                  {statusLabels[status]}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <div className="px-3 pt-2.5 pb-3 sm:px-4">
        {paginatedTransactions.length === 0 ? (
          <div className="flex h-24 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
            No transactions found.
          </div>
        ) : (
          <div className="divide-y rounded-lg border bg-background/50">
            {paginatedTransactions.map((t) => {
              const flow = t.flow ?? "incoming";
              const FlowIcon =
                flow === "incoming"
                  ? ArrowDownRight
                  : flow === "outgoing"
                    ? ArrowUpRight
                    : ArrowLeftRight;
              const amountBtc = transactionBtc(t, priceEur);
              const primaryAmount =
                currency === "btc"
                  ? formatBtc(amountBtc, { sign: true })
                  : formatSignedDisplayMoney(t.amount, priceEur, currency);
              const secondaryAmount =
                currency === "btc"
                  ? currencyFormatter.format(Math.abs(t.amount))
                  : formatBtc(amountBtc);
              const amountTone =
                flow === "incoming"
                  ? "text-emerald-700 dark:text-emerald-300"
                  : flow === "outgoing"
                    ? "text-red-700 dark:text-red-300"
                    : "text-muted-foreground";
              const primaryTag = t.tags[0] ?? overviewFlowLabels[flow];
              const extraTags = Math.max(0, t.tags.length - 1);
              return (
                <a
                  key={t.id}
                  href={transactionDetailHref(t.id)}
                  className="group flex min-w-0 items-center gap-3 px-3 py-2.5 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <span
                    className={cn(
                      "flex size-8 shrink-0 items-center justify-center rounded-md",
                      overviewFlowStyles[flow],
                    )}
                    aria-hidden="true"
                  >
                    <FlowIcon className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span
                      className={cn(
                        "block truncate text-sm font-medium text-foreground",
                        blurClass(hideSensitive),
                      )}
                    >
                      {t.counterparty}
                    </span>
                    <span className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium",
                          overviewFlowStyles[flow],
                        )}
                      >
                        {primaryTag}
                      </span>
                      {extraTags > 0 && (
                        <span className="text-[10px] text-muted-foreground">
                          +{extraTags}
                        </span>
                      )}
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium",
                          statusStyles[t.status],
                        )}
                      >
                        {statusLabels[t.status]}
                      </span>
                      <span className="truncate text-[10px] text-muted-foreground">
                        {t.date}
                      </span>
                    </span>
                    <span
                      className={cn(
                        "mt-1 hidden truncate font-mono text-[10px] text-muted-foreground sm:block",
                        blurClass(hideSensitive),
                      )}
                    >
                      {overviewFlowLabels[flow]} · {t.txid}
                    </span>
                  </span>
                  <span className="ml-auto flex shrink-0 flex-col items-end gap-0.5 pl-2 text-right">
                    <CurrencyToggleText
                      className={cn(
                        "text-sm font-semibold tabular-nums",
                        amountTone,
                        blurClass(hideSensitive),
                      )}
                    >
                      {primaryAmount}
                    </CurrencyToggleText>
                    <span
                      className={cn(
                        "text-[10px] text-muted-foreground tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {secondaryAmount}
                    </span>
                  </span>
                </a>
              );
            })}
          </div>
        )}
      </div>

      <div className="flex items-center justify-between border-t px-3 py-2.5 text-[10px] text-muted-foreground sm:px-4 sm:text-xs">
        <span>
          {startRow}-{endRow} of {filteredTransactions.length}
        </span>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon"
            className="size-7"
            onClick={() => goToPage(currentPage - 1)}
            disabled={currentPage === 1}
            aria-label="Go to previous page"
          >
            <ChevronLeft className="size-3.5" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-7"
            onClick={() => goToPage(currentPage + 1)}
            disabled={currentPage === totalPages}
            aria-label="Go to next page"
          >
            <ChevronRight className="size-3.5" />
          </Button>
        </div>
      </div>
      </div>
    </>
  );
};

const healthToneStyles: Record<OverviewHealthTone, string> = {
  good: "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

const BooksHealthPanel = ({
  className,
  snapshot,
  onProcessJournals,
  isProcessingJournals,
}: {
  className?: string;
  snapshot: OverviewSnapshot;
  onProcessJournals: () => void;
  isProcessingJournals: boolean;
}) => {
  const healthItems = buildOverviewHealthItems(snapshot);
  const primaryAction = buildPrimaryOverviewAction(snapshot);
  const PrimaryIcon = primaryAction?.icon;
  const needsJournals = Boolean(snapshot.status?.needsJournals);

  return (
    <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-3 pt-3 sm:px-4">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0 sm:size-8"
            aria-label="Books health"
          >
            <CheckCircle2 className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium">
              Books Health
            </span>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              What needs attention before reports
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-2.5 px-3 pt-2.5 pb-3 sm:px-4">
        {primaryAction && PrimaryIcon ? (
          <Link
            to={primaryAction.href}
            className={cn(
              "group flex items-start gap-3 rounded-lg p-2.5 ring-1 ring-inset transition-colors hover:bg-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              healthToneStyles[primaryAction.tone],
            )}
          >
            <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-background/70">
              <PrimaryIcon className="size-4" aria-hidden="true" />
            </span>
            <span className="min-w-0">
              <span className="block text-sm font-semibold">
                {primaryAction.title}
              </span>
              <span className="mt-0.5 block text-xs leading-5 opacity-80">
                {primaryAction.detail}
              </span>
            </span>
          </Link>
        ) : null}

        <div className="divide-y rounded-lg border bg-background/50">
          {healthItems.map((item) => {
            const ItemIcon = item.icon;
            const isJournalRefresh = item.key === "journals" && needsJournals;
            const content = (
              <>
                <span
                  className={cn(
                    "flex size-8 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
                    healthToneStyles[item.tone],
                  )}
                >
                  <ItemIcon
                    className={cn(
                      "size-4",
                      isJournalRefresh && isProcessingJournals && "animate-spin",
                    )}
                    aria-hidden="true"
                  />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs font-medium text-muted-foreground">
                    {item.title}
                  </span>
                  <span className="mt-0.5 block truncate text-sm font-semibold text-foreground">
                    {isJournalRefresh && isProcessingJournals
                      ? "Reprocessing"
                      : item.value}
                  </span>
                </span>
                <span className="hidden max-w-[140px] text-right text-[10px] leading-4 text-muted-foreground sm:block">
                  {item.detail}
                </span>
              </>
            );
            const className =
              "group flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

            return isJournalRefresh ? (
              <button
                key={item.key}
                type="button"
                className={className}
                onClick={onProcessJournals}
                disabled={isProcessingJournals}
              >
                {content}
              </button>
            ) : (
              <Link
                key={item.key}
                to={item.href}
                className={className}
              >
                {content}
              </Link>
            );
          })}
        </div>

        <Button asChild variant="ghost" size="sm" className="h-8 w-full">
          <Link to="/reports">
            <FileText className="size-4" aria-hidden="true" />
            Reports
          </Link>
        </Button>
      </div>
    </div>
  );
};

const Dashboard5 = ({
  className,
  snapshot = MOCK_OVERVIEW,
}: {
  className?: string;
  snapshot?: OverviewSnapshot;
}) => {
  const [addConnectionOpen, setAddConnectionOpen] = React.useState(false);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({ notifyStart: true });
  const transactions = React.useMemo(
    () =>
      snapshot.txs.length
        ? snapshot.txs.map(toDashboardTransaction)
        : transactionRecords,
    [snapshot.txs],
  );
  const refreshOverviewState = React.useCallback(() => {
    if (isSyncing || isProcessingJournals) return;
    syncAll({ onTrustedSuccess: runJournalProcessing });
  }, [isProcessingJournals, isSyncing, runJournalProcessing, syncAll]);
  const isRefreshingOverview = isSyncing || isProcessingJournals;

  return (
    <div
      className={cn(screenShellClassName, className)}
    >
      <WelcomeSection
        snapshot={snapshot}
        onRefresh={refreshOverviewState}
        onProcessJournals={runJournalProcessing}
        isRefreshing={isRefreshingOverview}
        isProcessingJournals={isProcessingJournals}
        onAddConnection={() => setAddConnectionOpen(true)}
      />
      <AddConnectionDialog
        open={addConnectionOpen}
        onOpenChange={setAddConnectionOpen}
      />
      <StatsCards
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
      />
      <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="grid min-w-0 gap-3">
          <RevenueFlowChart
            snapshot={snapshot}
            hideSensitive={hideSensitive}
            currency={currency}
          />
          <RecentTransactionsTable
            className="min-w-0"
            transactions={transactions}
            hideSensitive={hideSensitive}
            currency={currency}
            priceEur={snapshot.priceEur}
          />
        </div>
        <div className="grid min-w-0 gap-3">
          <SideChartsSection
            snapshot={snapshot}
            hideSensitive={hideSensitive}
            currency={currency}
          />
          <BooksHealthPanel
            snapshot={snapshot}
            onProcessJournals={runJournalProcessing}
            isProcessingJournals={isProcessingJournals}
          />
        </div>
      </div>
    </div>
  );
};

export { Dashboard5 };
