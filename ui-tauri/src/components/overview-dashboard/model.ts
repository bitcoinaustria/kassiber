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
  formatCompactFiatAmount,
  formatFiatAmount,
  fiatNumberFormatter,
  MISSING_FIAT_LABEL,
  type Currency,
} from "@/lib/currency";
import { formatShortDate } from "@/lib/date";
import { currentUiLocale } from "@/lib/localeFormat";
import { useUiStore } from "@/store/ui";
import {
  type OverviewSnapshot,
  type PortfolioPoint,
  type Tx as OverviewTx,
} from "@/mocks/seed";

// Translator type for the `overview` namespace. Helpers that produce
// user-facing copy take this (optionally) so the daemon/model layer stays
// locale-free while the visible strings resolve through i18next at the call
// site. When omitted, helpers fall back to English so non-UI callers (tests,
// not-yet-migrated components) keep working.
export type OverviewTranslate = (key: string, options?: Record<string, unknown>) => string;

// Maps an overview i18n key (+ optional interpolation params) to an English
// string, used as the fallback when no translator is supplied.
const OVERVIEW_EN_FALLBACK: Record<string, string> = {
  "marketRate.noSource": "No source",
  "marketRate.manual": "Manual",
  "marketRate.notSynced": "Not synced",
  "marketRate.justNow": "just now",
  "marketRate.fetchRates": "Fetch rates",
  "marketRate.noRate": "No {{currency}} rate",
  "marketRate.perBtc": "{{value}} / BTC",
  "marketRate.synced": "Synced {{date}}",
  "marketRate.minutesAgo": "{{count}}m ago",
  "marketRate.hoursAgo": "{{count}}h ago",
  "marketRate.daysAgo": "{{count}}d ago",
  "recentTx.unassigned": "Unassigned",
};

function fallbackTranslate(key: string, options?: Record<string, unknown>) {
  const template = OVERVIEW_EN_FALLBACK[key] ?? key;
  if (!options) return template;
  return template.replace(/\{\{(\w+)\}\}/g, (_match, name: string) =>
    options[name] === undefined ? "" : String(options[name]),
  );
}

function resolveTranslate(t?: OverviewTranslate): OverviewTranslate {
  return t ?? fallbackTranslate;
}

export type StatId =
  | "portfolioValue"
  | "transactions"
  | "reviewedEvents"
  | "openReview"
  | "connections";

export type StatItem = {
  // Stable slug for React keys and equality checks; never a translated string.
  id: StatId;
  // i18n key in the `overview` namespace for the visible title.
  titleKey: string;
  previousValue: number;
  value: number;
  changePercent: number;
  isPositive: boolean;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  format: "currency" | "number";
  // i18n key in the `overview` namespace for the comparison caption.
  comparisonLabelKey: string;
  href: OverviewHref;
};

export type HoldingsItem = {
  // Connection label (data) for real sources; for the synthetic aggregate row
  // this is empty and `nameKey` carries the i18n key instead.
  name: string;
  // i18n key in the `overview` namespace for synthetic rows (e.g. "Other
  // sources"); undefined for real connection rows that display `name`.
  nameKey?: string;
  value: number;
  percent: number;
  color: string;
};

export type BalanceDriverItem = {
  key: "incoming" | "outgoing" | "swap" | "fees";
  // i18n key in the `overview` namespace for the visible driver label.
  labelKey: string;
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
  priceEur?: number;
  unrealizedEur: number;
};

export type PortfolioChartMetric = "value" | "btc" | "basis" | "unrealized";
export type ActivityFlow = "incoming" | "outgoing" | "movement" | "fee";
export type TreasuryChartSeriesKey =
  | "primary"
  | "portfolioValue"
  | "price"
  | "basis"
  | "events";
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
export const Y_SCALE_PARAM = "scale";
export const Y_AUTO_FIT_PARAM = "fit";
export const ACTIVITY_MARKER_GROUPING_PARAM = "groupEvents";
export const TREASURY_BRUSH_MIN_WINDOW_MS = (7 * 24 * 60 * 60 * 1000) / 3;
export const TREASURY_BRUSH_MIN_INDEX_SPAN = 2;

export const defaultTreasurySeriesVisibility: TreasurySeriesVisibility = {
  primary: true,
  portfolioValue: false,
  price: true,
  basis: true,
  events: true,
};

export type TreasuryChartPoint = PortfolioChartPoint & {
  bitcoinPriceEur: number;
  avgCostEur: number | null;
  lineBalanceBtc?: number;
  linePortfolioValueEur?: number;
  lineBitcoinPriceEur?: number;
  lineAvgCostEur?: number | null;
  brushBalanceBtc: number;
  reserveValueEur: number;
  activityBtc: number;
  activityCount: number;
  activityValueEur: number;
  eventPriceEur?: number;
  eventBalanceBtc?: number;
  markerBalanceBtc?: number;
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
  markerCount?: number;
  markerGroupedPoints?: TreasuryChartPoint[];
  markerMixedFlows?: boolean;
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

export type ActivityScatterDotProps = {
  cx?: number;
  cy?: number;
  size?: number;
  payload?: TreasuryChartPoint;
  activeSeries: TreasuryChartSeriesKey | null;
  // Resolved per theme once in the chart — dots render per marker, so they
  // take the palette as a prop instead of subscribing to the theme store.
  flowColors: Record<ActivityFlow, string>;
  onOpenTransactionDetail?: (transactionId: string) => void;
  onHoverActivityPoint?: (point: TreasuryChartPoint | null) => void;
};

export type ActivityMarkerView = {
  activityPoints: TreasuryChartPoint[];
  chartDisplayData: TreasuryChartPoint[];
  visibleActivityMarkers: TreasuryChartPoint[];
};

export type ActivityMarkerClusterOptions = {
  maxVisibleMarkers?: number;
};

export type TreasuryActivityEvent = {
  tx: OverviewSnapshot["txs"][number];
  btc: number;
  signedBtc: number;
  feeBtc: number;
  postBalanceBtc?: number;
  postCostBasisEur?: number;
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
  profileId?: string;
  scopeLabel?: string;
  counterparty: string;
  counterpartyInitials: string;
  paymentMethod?: "On-chain" | "Lightning" | "Liquid" | "Other";
  tags: string[];
  status: TransactionStatus;
  flow?: OverviewTransactionFlow;
  amount: number | null;
  amountBtc?: number;
  fiatCurrency?: string | null;
  date: string;
};

// i18n key plus optional interpolation params, resolved via `t()` at the call
// site. Lets the locale-free model layer describe copy without rendering it.
export type OverviewCopy = {
  key: string;
  params?: Record<string, unknown>;
};

export type OverviewHealthItem = {
  key: string;
  title: OverviewCopy;
  value: OverviewCopy;
  detail: OverviewCopy;
  href: OverviewHref;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

export type OverviewReadiness = {
  title: OverviewCopy;
  detail: OverviewCopy;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  tone: OverviewHealthTone;
};

export const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function btcFromEur(eur: number, priceEur: number) {
  return priceEur ? eur / priceEur : 0;
}

export function formatDisplayMoney(
  fiatValue: number | null,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (fiatValue === null) return MISSING_FIAT_LABEL;
  if (currency === "btc") return formatBtc(btcFromEur(fiatValue, fiatRate));
  return formatFiatAmount(fiatValue, fiatCurrency);
}

export function formatSignedDisplayMoney(
  fiatValue: number | null,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (fiatValue === null) return MISSING_FIAT_LABEL;
  if (currency === "btc") {
    return formatBtc(btcFromEur(fiatValue, fiatRate), { sign: true });
  }
  const prefix = fiatValue >= 0 ? "+ " : "− ";
  return `${prefix}${formatFiatAmount(Math.abs(fiatValue), fiatCurrency)}`;
}

export function formatCompactDisplayMoney(
  fiatValue: number,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (currency === "btc") {
    return formatBtc(btcFromEur(fiatValue, fiatRate), { precision: 3 });
  }
  return formatCompactFiatAmount(fiatValue, fiatCurrency);
}

export function formatPortfolioMoney(
  amount: number,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (currency === "btc") return formatBtc(amount);
  return formatDisplayMoney(amount, fiatRate, currency, fiatCurrency);
}

export function formatDriverValue(
  btc: number,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (currency === "btc") {
    return formatBtc(btc, { precision: btc > 0 && btc < 0.001 ? 8 : 3 });
  }
  return formatCompactDisplayMoney(btc * fiatRate, fiatRate, currency, fiatCurrency);
}

export function formatDetailedPortfolioMoney(
  amount: number,
  fiatRate: number,
  currency: Currency,
  fiatCurrency = "EUR",
) {
  if (currency === "btc") {
    return formatBtc(amount, { precision: Math.abs(amount) < 0.01 ? 8 : 4 });
  }
  return formatDisplayMoney(amount, fiatRate, currency, fiatCurrency);
}

export function activeMarketFiatCurrency(snapshot: OverviewSnapshot) {
  return (
    snapshot.marketRate?.fiatCurrency ??
    snapshot.fiat.fiatCurrency ??
    "EUR"
  ).toUpperCase();
}

export function activeMarketFiatRate(snapshot: OverviewSnapshot) {
  const rate = snapshot.marketRate?.rate;
  if (typeof rate === "number" && Number.isFinite(rate) && rate > 0) {
    return rate;
  }
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  if (fiatCurrency === "USD") return snapshot.priceUsd;
  return snapshot.priceEur;
}

export function formatMarketRateValue(
  snapshot: OverviewSnapshot,
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const rate = snapshot.marketRate?.rate;
  if (typeof rate !== "number" || !Number.isFinite(rate) || rate <= 0) {
    return t("marketRate.noRate", { currency: fiatCurrency });
  }
  return t("marketRate.perBtc", { value: formatFiatAmount(rate, fiatCurrency) });
}

// Provider/product names stay as-is (proper nouns); `manual` is a UI word and
// resolves through the translator at the call site.
const MARKET_RATE_SOURCE_LABELS: Record<string, string> = {
  "coinbase-exchange": "Coinbase Exchange",
  "kraken-csv": "Kraken CSV",
  coingecko: "CoinGecko",
};

export function formatMarketRateSource(
  source: string | null | undefined,
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  if (!source) return t("marketRate.noSource");
  const normalized = source.trim().toLowerCase();
  if (normalized === "manual") return t("marketRate.manual");
  return MARKET_RATE_SOURCE_LABELS[normalized] ?? source;
}

export function marketRateSyncLabel(
  snapshot: OverviewSnapshot,
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  const syncedAt = snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp;
  return syncedAt
    ? t("marketRate.synced", { date: formatShortDate(syncedAt) })
    : t("marketRate.notSynced");
}

export function formatRelativeMarketRateTime(
  value: string | null | undefined,
  nowMs = Date.now(),
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  if (!value) return null;
  const thenMs = Date.parse(value);
  if (!Number.isFinite(thenMs)) return null;
  const diffSec = Math.max(0, Math.floor((nowMs - thenMs) / 1000));
  if (diffSec < 60) return t("marketRate.justNow");
  if (diffSec < 3600) {
    return t("marketRate.minutesAgo", { count: Math.floor(diffSec / 60) });
  }
  if (diffSec < 86400) {
    return t("marketRate.hoursAgo", { count: Math.floor(diffSec / 3600) });
  }
  return t("marketRate.daysAgo", { count: Math.floor(diffSec / 86400) });
}

export function marketRateCompactLabel(
  snapshot: OverviewSnapshot,
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  const syncedAt = snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp;
  const source = snapshot.marketRate?.source;
  const sourceLabel = source
    ? formatMarketRateSource(source, t).replace(/\s+Exchange$/, "")
    : null;
  if (!syncedAt && !sourceLabel) return t("marketRate.fetchRates");
  const timeLabel = formatRelativeMarketRateTime(syncedAt, Date.now(), t);
  return [sourceLabel, timeLabel].filter(Boolean).join(" · ");
}

export function marketRateDetailLabel(
  snapshot: OverviewSnapshot,
  translate?: OverviewTranslate,
) {
  const t = resolveTranslate(translate);
  const pair = snapshot.marketRate?.pair;
  const source = snapshot.marketRate?.source;
  if (!pair && !source) return t("marketRate.fetchRates");
  if (!pair) return formatMarketRateSource(source, t);
  if (!source) return pair;
  return `${formatMarketRateSource(source, t)} · ${pair}`;
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
    main: "var(--kb-accent)",
    soft: `color-mix(in oklch, var(--kb-accent) 16%, transparent)`,
    light: `color-mix(in oklch, var(--kb-accent) 70%, ${mixBase})`,
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
    portfolioValue: "#0284c7",
    costBasis: "#2fae79",
    focus: "#2f2f33",
    risk: "#e3000f",
    riskSoft: "rgba(227, 0, 15, 0.16)",
    // slate-500: slate-400 only clears ~2.9:1 on a white card
    price: "#64748b",
  },
  dark: {
    value: "#f6a21a",
    portfolioValue: "#38bdf8",
    costBasis: "#50c695",
    focus: "#e8e8ec",
    risk: "#ff3341",
    riskSoft: "rgba(255, 51, 65, 0.18)",
    price: "#94a3b8",
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

// Recharts `ChartConfig` `label`s are not surfaced visibly here — the holdings
// donut renders a custom legend (translated at the call site) and has no
// recharts `<Tooltip>`. These stay as inert internal config values.
export const holdingsChartConfig = {
  onchain: { label: "On-chain BTC", color: palette.primary },
  lightning: { label: "Lightning", theme: palette.secondary },
  liquid: { label: "Liquid", theme: palette.tertiary },
  other: { label: "Other", theme: palette.quaternary },
} satisfies ChartConfig;

const statsData: StatItem[] = [
  {
    id: "portfolioValue",
    titleKey: "stats.title.portfolioValue",
    previousValue: 198502,
    value: 312842.77,
    changePercent: 27.86,
    isPositive: true,
    icon: CircleDollarSign,
    format: "currency",
    comparisonLabelKey: "stats.comparison.vsLastMonth",
    href: "/reports",
  },
  {
    id: "transactions",
    titleKey: "stats.title.transactions",
    previousValue: 184,
    value: 218,
    changePercent: 18.4,
    isPositive: true,
    icon: ClipboardList,
    format: "number",
    comparisonLabelKey: "stats.comparison.vsLastMonth",
    href: "/transactions",
  },
  {
    id: "reviewedEvents",
    titleKey: "stats.title.reviewedEvents",
    previousValue: 412,
    value: 497,
    changePercent: 20.8,
    isPositive: true,
    icon: Users,
    format: "number",
    comparisonLabelKey: "stats.comparison.vsLastMonth",
    href: "/connections",
  },
  {
    id: "openReview",
    titleKey: "stats.title.openReview",
    previousValue: 98,
    value: 84,
    changePercent: 13.73,
    isPositive: false,
    icon: CreditCard,
    format: "currency",
    comparisonLabelKey: "stats.comparison.vsLastMonth",
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
  const fiatBalance = btcFromEur(
    snapshot.fiat.eurBalance,
    activeMarketFiatRate(snapshot),
  );
  if (fiatBalance > 0) return fiatBalance;
  const latestBalance = snapshot.balanceSeries[snapshot.balanceSeries.length - 1];
  if (typeof latestBalance === "number") return latestBalance;
  return fiatBalance;
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
      comparisonLabelKey: isBitcoinMode
        ? "stats.comparison.bitcoinBalance"
        : snapshot.fiat.eurCostBasis
          ? "stats.comparison.vsCostBasis"
          : "stats.comparison.fromLoadedRows",
    },
    {
      ...statsData[1],
      value: transactionCount,
      previousValue: 0,
      changePercent: 0,
      isPositive: true,
      comparisonLabelKey: "stats.comparison.loadedRows",
    },
    {
      ...statsData[2],
      id: "connections",
      titleKey: "stats.title.connections",
      value: snapshot.connections.length,
      previousValue: 0,
      changePercent: 0,
      isPositive: true,
      comparisonLabelKey: "stats.comparison.configured",
    },
    {
      ...statsData[3],
      id: "openReview",
      titleKey: "stats.title.openReview",
      value: snapshot.status?.quarantines ?? 0,
      previousValue: 0,
      changePercent: 0,
      isPositive: (snapshot.status?.quarantines ?? 0) === 0,
      format: "number",
      comparisonLabelKey: "stats.comparison.journalQuarantine",
    },
  ];
}

export type TimePeriod =
  | "auto"
  | "30days"
  | "3months"
  | "6months"
  | "ytd"
  | "1year"
  | "5years"
  | "all";

export type ResolvedTimePeriod = Exclude<TimePeriod, "auto"> | "10years" | "15years";

// i18n keys in the `overview` namespace, resolved via `t()` at the call site.
export const periodLabelKeys = {
  auto: "period.auto",
  "30days": "period.30days",
  "3months": "period.3months",
  "6months": "period.6months",
  ytd: "period.ytd",
  "1year": "period.1year",
  "5years": "period.5years",
  all: "period.all",
} as const satisfies Record<TimePeriod, string>;

export const periodShortLabelKeys = {
  auto: "period.short.auto",
  "30days": "period.short.30days",
  "3months": "period.short.3months",
  "6months": "period.short.6months",
  ytd: "period.short.ytd",
  "1year": "period.short.1year",
  "5years": "period.short.5years",
  all: "period.short.all",
} as const satisfies Record<TimePeriod, string>;

export const periodKeys: TimePeriod[] = [
  "auto",
  "30days",
  "3months",
  "6months",
  "ytd",
  "1year",
  "5years",
  "all",
];

export function normalizeTimePeriodParam(value: string | null): TimePeriod | null {
  if (!value) return null;
  const normalized = value.toLowerCase().replace(/[\s_-]/g, "");
  if (normalized === "auto" || normalized === "automatic") return "auto";
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
  if (
    normalized === "6months" ||
    normalized === "6month" ||
    normalized === "6mos" ||
    normalized === "6mo" ||
    normalized === "6m"
  ) {
    return "6months";
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

export function initialTimePeriodFromUrl(fallback: TimePeriod = "auto"): TimePeriod {
  if (typeof window === "undefined") return fallback;
  const params = new URLSearchParams(window.location.search);
  return normalizeTimePeriodParam(params.get("period")) ?? fallback;
}

export function initialYScaleLogFromUrl(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  return params.get(Y_SCALE_PARAM)?.toLowerCase() === "log";
}

export function initialYAutoFitFromUrl(): boolean {
  if (typeof window === "undefined") return false;
  const params = new URLSearchParams(window.location.search);
  const value = params.get(Y_AUTO_FIT_PARAM)?.toLowerCase();
  return value === "auto" || value === "1";
}

export function initialActivityMarkerGroupingFromUrl(): boolean {
  if (typeof window === "undefined") return true;
  const params = new URLSearchParams(window.location.search);
  const value = params.get(ACTIVITY_MARKER_GROUPING_PARAM)?.toLowerCase();
  return value !== "0" && value !== "false" && value !== "off";
}

// A log scale has no place for zero: before the first funding the balance
// line sits at 0, which d3's log scale maps to -Infinity and breaks the SVG
// path. Null those values out instead — the lines already `connectNulls`, so
// the pre-funding stretch simply isn't drawn (TradingView behaves the same).
export function logSafeTreasuryPoints(
  points: TreasuryChartPoint[],
): TreasuryChartPoint[] {
  const positive = (value: number | null | undefined) =>
    typeof value === "number" && Number.isFinite(value) && value > 0
      ? value
      : undefined;
  return points.map((point) => ({
    ...point,
    lineBalanceBtc: positive(point.lineBalanceBtc),
    linePortfolioValueEur: positive(point.linePortfolioValueEur),
    lineBitcoinPriceEur: positive(point.lineBitcoinPriceEur),
    lineAvgCostEur: positive(point.lineAvgCostEur) ?? null,
  }));
}

export function logSafeActivityMarkers(
  points: TreasuryChartPoint[],
): TreasuryChartPoint[] {
  return points.filter((point) => (point.markerBalanceBtc ?? 0) > 0);
}

// Explicit domain for a log axis. Recharts' `auto` nices a log domain to full
// powers of ten, which pads a 58k–84k window out to 10k–100k and flattens the
// chart; a small multiplicative margin keeps the data filling the plot.
export function positiveLogDomain(
  values: Array<number | null | undefined>,
): [number, number] | null {
  let min = Number.POSITIVE_INFINITY;
  let max = 0;
  for (const value of values) {
    if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
      continue;
    }
    if (value < min) min = value;
    if (value > max) max = value;
  }
  if (!(max > 0)) return null;
  if (min === max) return [min * 0.9, max * 1.1];
  return [min * 0.96, max * 1.04];
}

// Snap `raw` onto a `grid`-sized step. Edge ticks round INWARD (ceil at the
// bottom of the domain, floor at the top) so the first and last labels never
// round themselves out of the domain and vanish — that left the plot's top
// and bottom stretches unlabeled. The 1e9 quantization kills float noise
// (39.2/0.1 === 392.0000000000001 must not ceil to 39.3).
function snapAxisTick(
  raw: number,
  grid: number,
  edge: "low" | "high" | "mid",
): number {
  const quotient = Math.round((raw / grid) * 1e9) / 1e9;
  const snapped =
    edge === "low"
      ? Math.ceil(quotient)
      : edge === "high"
        ? Math.floor(quotient)
        : Math.round(quotient);
  return snapped * grid;
}

// Evenly spaced ticks in log space, rounded to significant digits — the same
// visual rhythm TradingView's log price scale uses. Narrow domains (a balance
// that barely moves) need extra digits or the rounded ticks collapse into one.
export function logAxisTicks(
  [lo, hi]: [number, number],
  count = 5,
): number[] {
  if (!(lo > 0) || !(hi > lo) || count < 2) return [];
  const ratio = hi / lo;
  const significantDigits = ratio >= 1.5 ? 2 : ratio >= 1.05 ? 3 : 4;
  const logLo = Math.log10(lo);
  const logHi = Math.log10(hi);
  const ticks = new Set<number>();
  for (let index = 0; index < count; index += 1) {
    const raw = Math.pow(10, logLo + ((logHi - logLo) * index) / (count - 1));
    const grid = Math.pow(
      10,
      Math.floor(Math.log10(raw)) - (significantDigits - 1),
    );
    const edge = index === 0 ? "low" : index === count - 1 ? "high" : "mid";
    const rounded = snapAxisTick(raw, grid, edge);
    if (rounded >= lo && rounded <= hi) ticks.add(rounded);
  }
  return [...ticks].sort((a, b) => a - b);
}

// Evenly spaced ticks for a linear auto-fit domain. Recharts' own nice ticks
// start at the nearest round number INSIDE the domain, which can leave the
// bottom ~15% of a fitted axis unlabeled (exactly where the cost-basis line
// tends to sit). A half-magnitude grid keeps labels round while covering
// both edges.
export function linearAxisTicks(
  [lo, hi]: [number, number],
  count = 5,
): number[] {
  if (!Number.isFinite(lo) || !(hi > lo) || count < 2) return [];
  const step = (hi - lo) / (count - 1);
  const grid = Math.pow(10, Math.floor(Math.log10(step))) / 2;
  const ticks = new Set<number>();
  for (let index = 0; index < count; index += 1) {
    const raw = lo + step * index;
    const edge = index === 0 ? "low" : index === count - 1 ? "high" : "mid";
    const rounded = snapAxisTick(raw, grid, edge);
    if (rounded >= lo && rounded <= hi) ticks.add(rounded);
  }
  return [...ticks].sort((a, b) => a - b);
}

// Explicit domain for a linear auto-fit axis (TradingView's "Auto"): the
// visible data's extent plus a small margin, clamped at zero. Computing it
// client-side keeps the tick formatter's precision in sync with what the
// axis actually shows.
export function autoFitDomain(
  values: Array<number | null | undefined>,
): [number, number] | null {
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  for (const value of values) {
    if (typeof value !== "number" || !Number.isFinite(value)) continue;
    if (value < min) min = value;
    if (value > max) max = value;
  }
  if (!(max >= min)) return null;
  if (min === max) {
    const pad = Math.abs(min) * 0.05 || 1;
    return [Math.max(0, min - pad), max + pad];
  }
  const pad = (max - min) * 0.06;
  return [Math.max(0, min - pad), max + pad];
}

// Latest drawable value of a line series within the visible window, used for
// the TradingView-style last-value tag on the axis.
export function lastTreasuryLineValue(
  points: TreasuryChartPoint[],
  key:
    | "lineBalanceBtc"
    | "linePortfolioValueEur"
    | "lineBitcoinPriceEur"
    | "lineAvgCostEur",
): number | null {
  for (let index = points.length - 1; index >= 0; index -= 1) {
    const value = points[index][key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
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

// Whether the snapshot carries real treasury data worth plotting. Mirrors the
// fallback gate inside `getDataForPeriod`: with no portfolio series, an
// all-zero balance series, and no activity events, the chart would otherwise
// render synthetic demo points. Callers use this to show an empty state with a
// refresh prompt instead of misleading placeholder data.
export function hasTreasuryChartData(snapshot: OverviewSnapshot): boolean {
  if (snapshot.portfolioSeries?.length) return true;
  if (snapshot.balanceSeries.some((value) => value !== 0)) return true;
  return activityTxs(snapshot).length > 0;
}
export function getDataForPeriod(
  period: TimePeriod,
  snapshot: OverviewSnapshot,
  metric: PortfolioChartMetric,
  currency: Currency,
  density: "compact" | "detailed",
): PortfolioChartPoint[] {
  const resolvedPeriod = resolveAutoTimePeriod(snapshot, period);
  if (snapshot.portfolioSeries?.length) {
    const points = buildDatedPortfolioPoints(
      snapshot.portfolioSeries,
      resolvedPeriod,
      metric,
      currency,
      density,
    );
    if (points.length) return points;
  }
  if (!snapshot.balanceSeries.some((value) => value !== 0)) {
    // A book without history gets an honest empty chart, never invented
    // demo numbers. (Mock mode ships non-zero series via fixtures, so the
    // demo experience is unaffected.)
    return [];
  }
  const fiatRate = activeMarketFiatRate(snapshot);
  const labels =
    resolvedPeriod === "5years"
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
      : btc * fiatRate;
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
  if (resolvedPeriod === "30days") return points.slice(-4);
  if (resolvedPeriod === "3months") return points.slice(-3);
  if (resolvedPeriod === "6months") return points.slice(-6);
  if (resolvedPeriod === "ytd") {
    return points.slice(0, Math.max(1, new Date().getMonth() + 1));
  }
  if (resolvedPeriod === "5years") {
    return points.filter((_, index) => index % 3 === 0).slice(-5);
  }
  return points;
}

const AUTO_MIN_MEANINGFUL_EVENTS = 3;
const AUTO_MIN_ACTIVITY_VOLUME_BTC = 0.00001;
const AUTO_MIN_BALANCE_RANGE_BTC = 0.001;
const AUTO_MIN_BALANCE_RANGE_RATIO = 0.01;
const MS_PER_YEAR = 365.25 * 24 * 60 * 60 * 1000;

function autoCandidatePeriodsForHistory(
  oldestDate: Date,
  latestDate: Date,
): ResolvedTimePeriod[] {
  const historyYears =
    (latestDate.valueOf() - oldestDate.valueOf()) / MS_PER_YEAR;
  return [
    "ytd",
    "1year",
    "5years",
    ...(historyYears >= 8 ? (["10years"] as const) : []),
    ...(historyYears >= 12 ? (["15years"] as const) : []),
    "all",
  ];
}

function portfolioBalanceValuesForPeriod(
  snapshot: OverviewSnapshot,
  events: TreasuryActivityEvent[],
  latestDate: Date,
  period: ResolvedTimePeriod,
): number[] {
  const seriesValues = (snapshot.portfolioSeries ?? [])
    .filter((point) => isPointInPeriod(point.date, latestDate, period))
    .map((point) => point.balanceBtc)
    .filter((value) => Number.isFinite(value));
  const eventValues = events
    .filter((event) => isActivityInTreasuryPeriod(event, latestDate, period))
    .map((event) => event.postBalanceBtc)
    .filter((value): value is number => value !== undefined && Number.isFinite(value));
  return [...seriesValues, ...eventValues];
}

function hasMeaningfulBalanceRange(values: number[]): boolean {
  if (values.length < 2) return true;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const latest = Math.abs(values[values.length - 1] ?? max);
  const threshold = Math.max(
    AUTO_MIN_BALANCE_RANGE_BTC,
    latest * AUTO_MIN_BALANCE_RANGE_RATIO,
  );
  return max - min >= threshold;
}

export function resolveAutoTimePeriod(
  snapshot: OverviewSnapshot,
  period: TimePeriod,
): ResolvedTimePeriod {
  if (period !== "auto") return period;
  const events = activityTxs(snapshot);
  if (!events.length) return "ytd";
  const candidateTimes = [
    ...(snapshot.portfolioSeries ?? [])
      .map((point) => parseSeriesDate(point.date).valueOf())
      .filter((time) => Number.isFinite(time)),
    ...events.map((event) => event.occurredAt.valueOf()),
  ];
  const latestDate = new Date(
    candidateTimes.length ? Math.max(...candidateTimes) : Date.now(),
  );
  const oldestDate = new Date(
    candidateTimes.length ? Math.min(...candidateTimes) : latestDate.valueOf(),
  );
  const meaningfulEvents = events.filter(
    (event) => event.volumeBtc >= AUTO_MIN_ACTIVITY_VOLUME_BTC,
  );
  if (!meaningfulEvents.length) return "ytd";
  const targetEventCount = Math.min(
    AUTO_MIN_MEANINGFUL_EVENTS,
    meaningfulEvents.length,
  );
  return (
    autoCandidatePeriodsForHistory(oldestDate, latestDate).find(
      (candidate) => {
        const visibleEvents = meaningfulEvents.filter((event) =>
          isActivityInTreasuryPeriod(event, latestDate, candidate),
        );
        if (visibleEvents.length < targetEventCount) return false;
        return hasMeaningfulBalanceRange(
          portfolioBalanceValuesForPeriod(
            snapshot,
            events,
            latestDate,
            candidate,
          ),
        );
      },
    ) ?? "all"
  );
}

export function buildDatedPortfolioPoints(
  series: PortfolioPoint[],
  period: ResolvedTimePeriod,
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
  period: TimePeriod | ResolvedTimePeriod,
) {
  if (period === "auto") return true;
  const pointDate = parseSeriesDate(value);
  if (period === "ytd") {
    return pointDate.getUTCFullYear() === latestDate.getUTCFullYear();
  }
  const start = new Date(latestDate);
  if (period === "30days") {
    start.setUTCDate(start.getUTCDate() - 30);
  } else if (period === "3months") {
    start.setUTCMonth(start.getUTCMonth() - 3);
  } else if (period === "6months") {
    start.setUTCMonth(start.getUTCMonth() - 6);
  } else if (period === "1year") {
    start.setUTCFullYear(start.getUTCFullYear() - 1);
  } else if (period === "5years") {
    start.setUTCFullYear(start.getUTCFullYear() - 5);
  } else if (period === "10years") {
    start.setUTCFullYear(start.getUTCFullYear() - 10);
  } else if (period === "15years") {
    start.setUTCFullYear(start.getUTCFullYear() - 15);
  } else {
    return true;
  }
  return pointDate >= start && pointDate <= latestDate;
}

export function buildPortfolioChartPoint(
  point: Pick<
    PortfolioPoint,
    | "date"
    | "label"
    | "balanceBtc"
    | "valueEur"
    | "costBasisEur"
    | "priceEur"
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
    priceEur: point.priceEur,
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

export function endOfIsoDayDate(value: string | undefined) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return null;
  parsed.setUTCHours(23, 59, 59, 999);
  return parsed;
}

export function treasurySortTime(value: string | undefined) {
  if (!value) return null;
  const key = value.split("#")[0] ?? value;
  const parsed = key.includes("T") ? new Date(key) : endOfIsoDayDate(key);
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
  return parsed.toLocaleDateString(currentUiLocale(), {
    month: "short",
    year: "2-digit",
    timeZone: "UTC",
  });
}

export function formatTreasuryDetailDate(value: string) {
  const parsed = parseIsoDayDate(value);
  if (!parsed) return value;
  return parsed.toLocaleDateString(currentUiLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function formatFiatPrice(value: number, fiatCurrency = "EUR") {
  const rounded = fiatNumberFormatter(fiatCurrency).format(Math.round(value));
  return `${rounded} ${fiatCurrency}`;
}

export function formatEurPrice(eur: number) {
  return formatFiatPrice(eur, "EUR");
}

export function treasuryPrimaryValue(point: TreasuryChartPoint) {
  return point.balanceBtc;
}

export function formatBtcAxis(value: number) {
  const precision = Math.abs(value) >= 10 ? 0 : Math.abs(value) >= 1 ? 2 : 3;
  return formatBtc(value, { precision }).replace("₿ ", "₿");
}

// Axis formatter for fitted (log / auto-fit) domains. A fitted window can be
// far narrower than the value's magnitude — a balance hovering at ₿40.8 needs
// a decimal or every tick rounds to "₿41". Precision follows both the tick
// step and the value's own magnitude, whichever demands more digits.
export function formatBtcAxisFitted(
  value: number,
  domain: [number, number] | null,
) {
  if (!domain || !(domain[1] > domain[0]) || !(Math.abs(value) > 0)) {
    return formatBtcAxis(value);
  }
  const step = (domain[1] - domain[0]) / 4;
  const stepDecimals = step > 0 ? Math.ceil(-Math.log10(step)) : 0;
  const valueDecimals = 2 - Math.floor(Math.log10(Math.abs(value)));
  const precision = Math.min(6, Math.max(0, stepDecimals, valueDecimals));
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
  const flow = flowForOverviewTx(tx);
  return flow === "swap" || flow === "transfer" ? "movement" : flow;
}

// i18n keys in the `overview` namespace, indexed by activity flow. Resolved via
// `t()` at the call site.
export const activityFlowLabelKeys = {
  incoming: "activityFlow.incoming",
  outgoing: "activityFlow.outgoing",
  movement: "activityFlow.movement",
  fee: "activityFlow.fee",
} as const satisfies Record<ActivityFlow, string>;

// English fallback labels, kept for call sites not yet migrated to i18n
// (e.g. ActivityScatterDot's aria-label). Visible overview copy uses
// `activityFlowLabelKeys` + `t()`.
export const activityFlowLabels: Record<ActivityFlow, string> = {
  incoming: "Received",
  outgoing: "Spent",
  movement: "Movement",
  fee: "Fee",
};

// Marker colors need different weights per theme: the airy 400-tier hues read
// well on the dark card but wash out on white, so light mode steps down to
// the 600-tier of the same hues. Fee remains a flow for summaries/tooltips but
// is not rendered as its own overview marker.
export const activityFlowPalettes: Record<
  "light" | "dark",
  Record<ActivityFlow, string>
> = {
  light: {
    incoming: "#059669",
    outgoing: "#dc2626",
    movement: "#0284c7",
    fee: "#71717a",
  },
  dark: {
    incoming: "#34d399",
    outgoing: "#f87171",
    movement: "#38bdf8",
    fee: "#a1a1aa",
  },
};

export function useActivityFlowColors() {
  return activityFlowPalettes[useResolvedColorMode()];
}

export const activityFlowKeys: ActivityFlow[] = [
  "incoming",
  "outgoing",
  "movement",
];

export function activityTxs(snapshot: OverviewSnapshot): TreasuryActivityEvent[] {
  const txs = snapshot.activityTxs?.length ? snapshot.activityTxs : snapshot.txs;
  const fiatRate = activeMarketFiatRate(snapshot);
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
        flow === "fee"
          ? Math.max(btc, feeBtc)
          : flow === "movement"
            ? pairedVolume
            : btc;
      if (volumeBtc <= 0 && feeBtc <= 0) return [];
      const valueEur = Math.abs(tx.eur ?? 0);
      const priceEur =
        valueEur > 0 && btc > 0 ? valueEur / btc : tx.rate ?? fiatRate;
      const postBalanceBtc =
        typeof tx.balanceBtc === "number" && Number.isFinite(tx.balanceBtc)
          ? tx.balanceBtc
          : undefined;
      const postCostBasisEur =
        typeof tx.costBasisEur === "number" && Number.isFinite(tx.costBasisEur)
          ? tx.costBasisEur
          : undefined;
      return [
        {
          tx,
          btc,
          signedBtc,
          feeBtc,
          postBalanceBtc,
          postCostBasisEur,
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
  period: TimePeriod | ResolvedTimePeriod,
) {
  if (period === "auto") return true;
  if (period === "all") return true;
  if (period === "ytd") {
    return event.occurredAt.getUTCFullYear() === latestDate.getUTCFullYear();
  }
  const start = new Date(latestDate);
  if (period === "30days") {
    start.setUTCDate(start.getUTCDate() - 30);
  } else if (period === "3months") {
    start.setUTCMonth(start.getUTCMonth() - 3);
  } else if (period === "6months") {
    start.setUTCMonth(start.getUTCMonth() - 6);
  } else if (period === "1year") {
    start.setUTCFullYear(start.getUTCFullYear() - 1);
  } else if (period === "5years") {
    start.setUTCFullYear(start.getUTCFullYear() - 5);
  } else if (period === "10years") {
    start.setUTCFullYear(start.getUTCFullYear() - 10);
  } else if (period === "15years") {
    start.setUTCFullYear(start.getUTCFullYear() - 15);
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
  const fiatRate = activeMarketFiatRate(snapshot);
  const bitcoinPriceEur =
    point.priceEur && point.priceEur > 0
      ? point.priceEur
      : point.balanceBtc > 0
        ? point.valueEur / point.balanceBtc
        : fiatRate;
  const avgCostEur =
    point.balanceBtc > 0 && point.costBasisEur > 0
      ? point.costBasisEur / point.balanceBtc
      : null;
  return {
    ...point,
    bitcoinPriceEur,
    avgCostEur,
    lineBalanceBtc: point.balanceBtc,
    linePortfolioValueEur: point.valueEur,
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
  options: {
    drawLineValues?: boolean;
    markerAnchor?: TreasuryChartPoint | null;
  } = {},
): TreasuryChartPoint {
  const balanceBtc =
    event.postBalanceBtc ?? anchor?.balanceBtc ?? latestPortfolioBalanceBtc(snapshot);
  const costBasisEur =
    event.postCostBasisEur ?? anchor?.costBasisEur ?? snapshot.fiat.eurCostBasis;
  const valueEur =
    balanceBtc > 0
      ? balanceBtc * event.priceEur
      : anchor?.valueEur ?? snapshot.fiat.eurBalance;
  const avgCostEur =
    balanceBtc > 0 && costBasisEur > 0 ? costBasisEur / balanceBtc : null;
  const markerAnchor = options.markerAnchor ?? null;
  const eventDate = activityDateKey(event);
  const date = options.drawLineValues
    ? eventDate
    : markerAnchor?.date ?? eventDate;
  return {
    date,
    month: formatTreasuryTick(date),
    detailLabel: formatTreasuryDetailDate(eventDate),
    thisYear: valueEur,
    prevYear: costBasisEur,
    balanceBtc,
    valueEur,
    costBasisEur,
    unrealizedEur: valueEur - costBasisEur,
    bitcoinPriceEur: event.priceEur,
    avgCostEur,
    lineBalanceBtc: options.drawLineValues ? balanceBtc : undefined,
    linePortfolioValueEur: options.drawLineValues ? valueEur : undefined,
    lineBitcoinPriceEur: undefined,
    lineAvgCostEur: options.drawLineValues ? avgCostEur : undefined,
    brushBalanceBtc: balanceBtc,
    reserveValueEur: valueEur,
    activityBtc: event.volumeBtc,
    activityCount: 1,
    activityValueEur: event.valueEur,
    eventPriceEur: event.priceEur,
    eventBalanceBtc: balanceBtc,
    markerBalanceBtc: markerAnchor?.balanceBtc ?? balanceBtc,
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
  const resolvedPeriod = resolveAutoTimePeriod(snapshot, period);
  const basePoints = points.map((point) => buildTreasuryBasePoint(point, snapshot));
  const events = activityTxs(snapshot);
  const drawActivityLineValues = resolvedPeriod === "30days";
  const basePointsByDay = new Map(
    basePoints.map((point) => [String(point.date).slice(0, 10), point]),
  );
  const findMarkerAnchor = (event: TreasuryActivityEvent) => {
    if (drawActivityLineValues) return null;
    const eventDay = event.occurredAt.toISOString().slice(0, 10);
    const sameDayPoint = basePointsByDay.get(eventDay);
    if (sameDayPoint) return sameDayPoint;
    const eventTime = event.occurredAt.valueOf();
    return (
      basePoints.find((point) => point.sortTimeMs >= eventTime) ??
      nearestTreasuryAnchor(basePoints, event)
    );
  };
  const candidateTimes = [
    ...basePoints.map((point) => point.sortTimeMs).filter((time) => time > 0),
    ...events.map((event) => event.occurredAt.valueOf()),
  ];
  const latestTime = candidateTimes.length ? Math.max(...candidateTimes) : Date.now();
  const latestDate = new Date(latestTime);
  const eventPoints = events
    .filter((event) => isActivityInTreasuryPeriod(event, latestDate, resolvedPeriod))
    .map((event) =>
      buildTreasuryActivityPoint(
        event,
        nearestTreasuryAnchor(basePoints, event),
        snapshot,
        {
          drawLineValues: drawActivityLineValues,
          markerAnchor: findMarkerAnchor(event),
        },
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
  includeActivityPointsInDisplay = false,
): ActivityMarkerView {
  const activityPoints = plottedData.filter((point) => point.isActivityEvent);
  const visibleActivityMarkers = activityPoints.filter(
    (point) =>
      showEvents &&
      point.eventFlow !== "fee" &&
      (point.eventSize || point.activityBtc) >= markerMinimumForPoint(point),
  );
  const chartDisplayData = plottedData.filter(
    (point) => !point.isActivityEvent || includeActivityPointsInDisplay,
  );
  return {
    activityPoints,
    chartDisplayData,
    visibleActivityMarkers,
  };
}

const DEFAULT_ACTIVITY_MARKER_CLUSTER_TARGET = 36;

function markerGroupToleranceBtc(balanceBtc: number) {
  return Math.max(Math.abs(balanceBtc) * 0.006, 0.00005);
}

function dominantActivityFlow(points: TreasuryChartPoint[]): ActivityFlow {
  const totals: Record<ActivityFlow, number> = {
    incoming: 0,
    outgoing: 0,
    movement: 0,
    fee: 0,
  };
  for (const point of points) {
    if (!point.eventFlow) continue;
    totals[point.eventFlow] += point.eventSize || point.activityBtc || 0;
  }
  return (Object.entries(totals) as Array<[ActivityFlow, number]>).reduce(
    (winner, entry) => (entry[1] > winner[1] ? entry : winner),
    ["movement", 0] as [ActivityFlow, number],
  )[0];
}

function representativeActivityPoint(points: TreasuryChartPoint[]) {
  return points.reduce((winner, point) =>
    (point.eventSize || point.activityBtc || 0) >
    (winner.eventSize || winner.activityBtc || 0)
      ? point
      : winner,
  );
}

export function clusterActivityMarkers(
  markers: TreasuryChartPoint[],
  options: ActivityMarkerClusterOptions = {},
): TreasuryChartPoint[] {
  if (markers.length < 2) return markers;
  const target = options.maxVisibleMarkers ?? DEFAULT_ACTIVITY_MARKER_CLUSTER_TARGET;
  const ordered = [...markers].sort((a, b) => {
    const timeDelta = a.sortTimeMs - b.sortTimeMs;
    if (timeDelta !== 0) return timeDelta;
    return (a.markerBalanceBtc ?? 0) - (b.markerBalanceBtc ?? 0);
  });
  const firstTime = ordered[0]?.sortTimeMs ?? 0;
  const lastTime = ordered.at(-1)?.sortTimeMs ?? firstTime;
  const densityBucketMs =
    markers.length > target && lastTime > firstTime
      ? Math.max((lastTime - firstTime) / target, 60 * 60 * 1000)
      : 0;
  const groups: TreasuryChartPoint[][] = [];

  for (const marker of ordered) {
    const markerBalance = marker.markerBalanceBtc ?? marker.balanceBtc;
    const markerTime = marker.sortTimeMs;
    const group = groups.find((candidate) => {
      const anchor = candidate[0];
      if (!anchor) return false;
      const anchorBalance = anchor.markerBalanceBtc ?? anchor.balanceBtc;
      const sameAnchor = String(anchor.date) === String(marker.date);
      const nearbyDenseTime =
        densityBucketMs > 0 && Math.abs(markerTime - anchor.sortTimeMs) <= densityBucketMs;
      return (
        (sameAnchor || nearbyDenseTime) &&
        Math.abs(markerBalance - anchorBalance) <=
          markerGroupToleranceBtc(anchorBalance)
      );
    });
    if (group) {
      group.push(marker);
    } else {
      groups.push([marker]);
    }
  }

  const clustered: TreasuryChartPoint[] = [];
  for (const group of groups) {
    if (group.length === 1) {
      clustered.push(group[0]);
      continue;
    }
    const representative = representativeActivityPoint(group);
    const eventFlow = dominantActivityFlow(group);
    const eventSize = Math.max(
      ...group.map((point) => point.eventSize || point.activityBtc || 0),
    );
    const flowCount = new Set(group.map((point) => point.eventFlow).filter(Boolean))
      .size;
    clustered.push({
      ...representative,
      eventFlow,
      eventSize: eventSize * (1 + Math.min(group.length - 1, 8) * 0.12),
      markerCount: group.length,
      markerGroupedPoints: group,
      markerMixedFlows: flowCount > 1,
      eventId: undefined,
      eventTransactionId: undefined,
    });
  }

  return clustered.sort((a, b) => a.sortTimeMs - b.sortTimeMs);
}

export function brushedActivityMarkers(
  activityMarkers: TreasuryChartPoint[],
  selectedChartDisplayData: TreasuryChartPoint[],
) {
  const selectedDates = new Set(
    selectedChartDisplayData.map((point) => String(point.date)),
  );
  return activityMarkers.filter((point) => selectedDates.has(String(point.date)));
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
  period: TimePeriod | ResolvedTimePeriod,
  density: "compact" | "detailed",
) {
  const maxPoints =
    density === "detailed"
      ? period === "all" ||
        period === "5years" ||
        period === "10years" ||
        period === "15years"
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

export function formatPortfolioTick(
  value: string,
  period: TimePeriod | ResolvedTimePeriod,
) {
  const date = parseSeriesDate(value);
  if (period === "5years") {
    return date.toLocaleDateString(currentUiLocale(), {
      month: "short",
      year: "2-digit",
      timeZone: "UTC",
    });
  }
  if (period === "30days" || period === "3months") {
    return date.toLocaleDateString(currentUiLocale(), {
      month: "short",
      day: "numeric",
      timeZone: "UTC",
    });
  }
  return date.toLocaleDateString(currentUiLocale(), {
    month: "short",
    year: "2-digit",
    timeZone: "UTC",
  });
}

export function portfolioTickBucket(
  point: PortfolioChartPoint,
  period: TimePeriod | ResolvedTimePeriod,
) {
  if (point.date.startsWith("fallback-") || point.date.startsWith("series-")) {
    return point.month;
  }
  if (period === "30days" || period === "3months") return point.month;
  if (
    period === "5years" ||
    period === "10years" ||
    period === "15years" ||
    period === "all"
  ) {
    const date = parseSeriesDate(point.date);
    return `${date.getUTCFullYear()}-${Math.floor(date.getUTCMonth() / 3)}`;
  }
  const date = parseSeriesDate(point.date);
  return `${date.getUTCFullYear()}-${date.getUTCMonth()}`;
}

export function portfolioAxisTicks(
  points: PortfolioChartPoint[],
  period: TimePeriod | ResolvedTimePeriod,
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
  return date.toLocaleDateString(currentUiLocale(), {
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
  const fiatRate = activeMarketFiatRate(snapshot);
  const byRail: Record<BalanceRail, number> = {
    onchain: 0,
    lightning: 0,
    liquid: 0,
    other: 0,
  };
  for (const connection of snapshot.connections) {
    if (connection.balance <= 0) continue;
    const rail = railForConnection(connection.kind, connection.label);
    byRail[rail] += connection.balance * fiatRate;
  }
  const total = Object.values(byRail).reduce((sum, value) => sum + value, 0);
  const items = [
    {
      key: "onchain",
      labelKey: "holdings.rail.onchain",
      value: byRail.onchain,
      percent: percentOf(byRail.onchain, total),
      color: palette.primary,
    },
    {
      key: "lightning",
      labelKey: "holdings.rail.lightning",
      value: byRail.lightning,
      percent: percentOf(byRail.lightning, total),
      color: palette.secondary.light,
    },
    {
      key: "liquid",
      labelKey: "holdings.rail.liquid",
      value: byRail.liquid,
      percent: percentOf(byRail.liquid, total),
      color: palette.tertiary.light,
    },
    {
      key: "other",
      labelKey: "holdings.rail.other",
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
  const fiatRate = activeMarketFiatRate(snapshot);
  const rows = snapshot.connections
    .filter((connection) => connection.balance > 0)
    .map((connection) => ({
      name: connection.label,
      value: connection.balance * fiatRate,
    }))
    .sort((a, b) => b.value - a.value);
  const total = rows.reduce((sum, item) => sum + item.value, 0);
  const visibleRows: Array<{ name: string; nameKey?: string; value: number }> =
    rows.length > 4
      ? [
          ...rows.slice(0, 3),
          {
            name: "",
            nameKey: "holdings.otherSources",
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
    nameKey: item.nameKey,
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
      labelKey: "drivers.incoming",
      valueBtc: totals.incomingBtc,
      count: totals.incomingCount,
      icon: ArrowDownRight,
      toneClassName: "text-emerald-700 dark:text-emerald-300",
    },
    {
      key: "outgoing",
      labelKey: "drivers.outgoing",
      valueBtc: totals.outgoingBtc,
      count: totals.outgoingCount,
      icon: ArrowUpRight,
      toneClassName: "text-red-700 dark:text-red-300",
    },
    {
      key: "swap",
      labelKey: "drivers.swapVolume",
      valueBtc: totals.swapBtc,
      count: totals.swapCount,
      icon: ArrowLeftRight,
      toneClassName: "text-sky-700 dark:text-sky-300",
    },
    {
      key: "fees",
      labelKey: "drivers.fees",
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

// i18n keys in the `overview` namespace, resolved via `t()` at the call site.
export const statusLabelKeys = {
  confirmed: "txStatus.confirmed",
  pending: "txStatus.pending",
  review: "txStatus.review",
  failed: "txStatus.failed",
} as const satisfies Record<TransactionStatus, string>;

export const overviewFlowLabelKeys = {
  incoming: "txFlow.incoming",
  outgoing: "txFlow.outgoing",
  transfer: "txFlow.transfer",
  swap: "txFlow.swap",
} as const satisfies Record<OverviewTransactionFlow, string>;

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

export function toDashboardTransaction(
  tx: OverviewTx,
  index: number,
  t?: OverviewTranslate,
): Transaction {
  const displayAmountSat =
    tx.type === "Consolidation" && tx.amountSat === 0 && tx.feeSat
      ? -Math.abs(tx.feeSat)
      : tx.amountSat;
  const amount =
    tx.eur !== null
      ? tx.eur
      : tx.rate !== null
        ? (displayAmountSat / 100_000_000) * tx.rate
        : null;
  const unassignedLabel =
    typeof t === "function" ? t("recentTx.unassigned") : "Unassigned";
  const account = tx.account || tx.counter || unassignedLabel;
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
    amountBtc: displayAmountSat / 100_000_000,
    fiatCurrency: tx.fiatCurrency,
    date: tx.date,
  };
}

export function overviewTransactions(
  snapshot: OverviewSnapshot,
  t?: OverviewTranslate,
) {
  return snapshot.txs.map((tx, index) => toDashboardTransaction(tx, index, t));
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

export function transactionSetHref(transactionIds: string[]) {
  const params = new URLSearchParams();
  if (typeof window !== "undefined") {
    const currentParams = new URLSearchParams(window.location.search);
    const period = currentParams.get("period");
    if (period) params.set("period", period);
  }
  params.set("txids", transactionIds.join(","));
  return `/transactions?${params.toString()}#transactions-table`;
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
  const sourceDetail: OverviewCopy = totalConnections
    ? {
        key: "readiness.sourceDetail",
        params: {
          count: totalConnections,
          synced: syncedConnections,
          total: totalConnections,
        },
      }
    : { key: "readiness.noSources" };

  if (!snapshot.txs.length && !totalConnections) {
    return {
      title: { key: "readiness.connectSource.title" },
      detail: { key: "readiness.connectSource.detail" },
      icon: Plus,
      tone: "neutral",
    };
  }

  if (erroredConnections) {
    return {
      title: { key: "readiness.sourceAttention.title" },
      detail: {
        key: "readiness.sourceAttention.detail",
        params: { count: erroredConnections },
      },
      icon: WalletCards,
      tone: "alert",
    };
  }

  if (needsJournals) {
    return {
      title: { key: "readiness.reprocessJournals.title" },
      detail: { key: "readiness.reprocessJournals.detail" },
      icon: RefreshCw,
      tone: "warning",
    };
  }

  if (quarantines > 0) {
    return {
      title: { key: "readiness.reviewQueueOpen.title" },
      detail: {
        key: "readiness.reviewQueueOpen.detail",
        params: { count: quarantines },
      },
      icon: ShieldAlert,
      tone: "alert",
    };
  }

  if (syncingConnections) {
    return {
      title: { key: "readiness.syncInProgress.title" },
      detail: sourceDetail,
      icon: RefreshCw,
      tone: "warning",
    };
  }

  return {
    title: { key: "readiness.readyForReports.title" },
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
      title: { key: "health.journals.title" },
      value: {
        key: needsJournals ? "health.journals.reprocess" : "health.journals.current",
      },
      detail: {
        key: needsJournals
          ? "health.journals.detailReprocess"
          : "health.journals.detailCurrent",
      },
      href: "/journals",
      icon: needsJournals ? RefreshCw : CheckCircle2,
      tone: needsJournals ? "warning" : "good",
    },
    {
      key: "review",
      title: { key: "health.review.title" },
      value: quarantines
        ? { key: "health.review.open", params: { count: quarantines } }
        : { key: "health.review.clear" },
      detail: {
        key: quarantines
          ? "health.review.detailOpen"
          : "health.review.detailClear",
      },
      href: "/quarantine",
      icon: quarantines ? ShieldAlert : CheckCircle2,
      tone: quarantines ? "alert" : "good",
    },
    {
      key: "connections",
      title: { key: "health.connections.title" },
      value: erroredConnections
        ? { key: "health.connections.issues", params: { count: erroredConnections } }
        : syncingConnections
          ? {
              key: "health.connections.refreshing",
              params: { count: syncingConnections },
            }
          : totalConnections
            ? {
                key: "health.connections.current",
                params: { synced: syncedConnections, total: totalConnections },
              }
            : { key: "health.connections.none" },
      detail: totalConnections
        ? {
            key: "health.connections.detail",
            params: { count: totalConnections },
          }
        : { key: "health.connections.detailEmpty" },
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
      title: { key: "health.primary.reviewQuarantines.title" } as OverviewCopy,
      detail: { key: "health.primary.reviewQuarantines.detail" } as OverviewCopy,
      href: "/quarantine",
      icon: ShieldAlert,
      tone: "alert" as const,
    };
  }
  if (snapshot.connections.some((connection) => connection.status === "error")) {
    return {
      title: { key: "health.primary.checkConnections.title" } as OverviewCopy,
      detail: { key: "health.primary.checkConnections.detail" } as OverviewCopy,
      href: "/connections",
      icon: WalletCards,
      tone: "alert" as const,
    };
  }
  return {
    title: {
      key: snapshot.txs.length
        ? "health.primary.openReports.title"
        : "health.primary.addConnection.title",
    } as OverviewCopy,
    detail: {
      key: snapshot.txs.length
        ? "health.primary.openReports.detail"
        : "health.primary.addConnection.detail",
    } as OverviewCopy,
    href: snapshot.txs.length ? "/reports" : "/connections",
    icon: snapshot.txs.length ? FileText : Plus,
    tone: "good" as const,
  };
}
