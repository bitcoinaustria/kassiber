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

const mixBase = "var(--background)";

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

export const readinessToneStyles: Record<OverviewHealthTone, string> = {
  good:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};


export function statusForOverviewTx(tx: OverviewTx): TransactionStatus {
  if (tx.internal) return "pending";
  if (tx.conf > 0) return "confirmed";
  return tx.tag.toLowerCase().includes("review") ? "review" : "pending";
}


type BalanceRail = "onchain" | "lightning" | "liquid" | "other";

function percentOf(value: number, total: number) {
  if (total <= 0) return 0;
  return Math.round((value / total) * 100);
}

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

function initials(value: string) {
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

export const healthToneStyles: Record<OverviewHealthTone, string> = {
  good: "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};
