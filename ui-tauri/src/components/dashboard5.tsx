import { Link } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Bell,
  ChevronLeft,
  ChevronRight,
  CircleDollarSign,
  ClipboardList,
  CreditCard,
  Download,
  Filter,
  Landmark,
  Maximize2,
  MoreHorizontal,
  PieChartIcon,
  Plus,
  Users,
} from "lucide-react";
import * as React from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { Button } from "@/components/ui/button";
import { type ChartConfig, ChartContainer } from "@/components/ui/chart";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip as ShadTooltip,
  TooltipContent as ShadTooltipContent,
  TooltipProvider as ShadTooltipProvider,
  TooltipTrigger as ShadTooltipTrigger,
} from "@/components/ui/tooltip";
import { AddConnectionFlow } from "@/components/kb/AddConnectionFlow";
import { cn } from "@/lib/utils";

type StatItem = {
  title: string;
  previousValue: number;
  value: number;
  changePercent: number;
  isPositive: boolean;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  format: "currency" | "number";
};

type SalesCategoryItem = {
  name: string;
  value: number;
  percent: number;
  color: string;
};

type RevenueFlowColors = {
  thisYear: string;
  prevYear: string;
};

type TransactionStatus = "confirmed" | "pending" | "review" | "failed";

type Transaction = {
  id: string;
  txid: string;
  counterparty: string;
  counterpartyInitials: string;
  tags: string[];
  status: TransactionStatus;
  amount: number;
  date: string;
};

type ActivityItem = {
  title: string;
  detail: string;
  time: string;
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

const salesCategoryChartConfig = {
  onchain: { label: "On-chain BTC", color: palette.primary },
  lightning: { label: "Lightning", theme: palette.secondary },
  liquid: { label: "Liquid", theme: palette.tertiary },
  other: { label: "Other", theme: palette.quaternary },
} satisfies ChartConfig;

const revenueFlowChartConfig = {
  thisYear: { label: "Value", color: palette.primary },
  prevYear: { label: "Cost Basis", theme: palette.secondary },
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
  },
  {
    title: "Transactions",
    previousValue: 184,
    value: 218,
    changePercent: 18.4,
    isPositive: true,
    icon: ClipboardList,
    format: "number",
  },
  {
    title: "Reviewed events",
    previousValue: 412,
    value: 497,
    changePercent: 20.8,
    isPositive: true,
    icon: Users,
    format: "number",
  },
  {
    title: "Open review",
    previousValue: 98,
    value: 84,
    changePercent: 13.73,
    isPositive: false,
    icon: CreditCard,
    format: "currency",
  },
];

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

type TimePeriod = "30days" | "3months" | "ytd" | "1year" | "5years";

const periodLabels: Record<TimePeriod, string> = {
  "30days": "30 Days",
  "3months": "3 Months",
  ytd: "YTD",
  "1year": "1 Year",
  "5years": "5 Years",
};

const periodKeys: TimePeriod[] = [
  "30days",
  "3months",
  "ytd",
  "1year",
  "5years",
];

function getDataForPeriod(period: TimePeriod) {
  if (period === "30days") return fullYearData.slice(-4);
  if (period === "3months") return fullYearData.slice(-3);
  if (period === "ytd") return fullYearData.slice(0, 6);
  if (period === "5years") return fiveYearData;
  return fullYearData;
}

const revenueSourceData = {
  total: 124700,
  onchain: { value: 72400, percent: 58 },
  lightning: { value: 31200, percent: 25 },
  liquid: { value: 16100, percent: 13 },
  manual: { value: 5000, percent: 4 },
};

const salesCategoryData: SalesCategoryItem[] = [
  {
    name: "On-chain BTC",
    value: 145200,
    percent: 58,
    color: palette.primary,
  },
  {
    name: "Lightning",
    value: 62400,
    percent: 25,
    color: `color-mix(in oklch, var(--primary) 80%, ${mixBase})`,
  },
  {
    name: "Liquid",
    value: 32500,
    percent: 13,
    color: `color-mix(in oklch, var(--primary) 60%, ${mixBase})`,
  },
  {
    name: "Other",
    value: 10000,
    percent: 4,
    color: `color-mix(in oklch, var(--primary) 42%, ${mixBase})`,
  },
];

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

const recentActivity: ActivityItem[] = [
  {
    title: "Wallet sync completed",
    detail: "Cold Storage",
    time: "2 minutes ago",
  },
  {
    title: "Journal processed",
    detail: "Austrian profile",
    time: "2 minutes ago",
  },
  {
    title: "Transaction priced",
    detail: "BTC-EUR rate cache",
    time: "2 minutes ago",
  },
  {
    title: "Report exported",
    detail: "Capital gains",
    time: "6 minutes ago",
  },
  {
    title: "Label imported",
    detail: "BIP329",
    time: "12 minutes ago",
  },
  {
    title: "Review item opened",
    detail: "Missing fiat price",
    time: "18 minutes ago",
  },
  {
    title: "Attachment verified",
    detail: "Receipt hash matched",
    time: "24 minutes ago",
  },
  {
    title: "Manual pair added",
    detail: "BTC to LBTC peg-out",
    time: "32 minutes ago",
  },
  {
    title: "Diagnostics collected",
    detail: "Public-safe bundle",
    time: "45 minutes ago",
  },
  {
    title: "Profile switched",
    detail: "Business wallet",
    time: "1 hour ago",
  },
];

const WelcomeSection = ({
  onAddConnection,
}: {
  onAddConnection: () => void;
}) => {
  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-end sm:justify-between sm:gap-6">
      <div className="space-y-2 sm:space-y-5">
        <h2 className="text-lg leading-relaxed font-semibold sm:text-[22px]">
          Welcome back, Hal.
        </h2>
        <p className="text-sm text-muted-foreground sm:text-base">
          Today you have{" "}
          <span className="font-medium text-foreground">18 transactions</span>{" "}
          waiting for review and{" "}
          <span className="font-medium text-foreground">3 reports</span> ready
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-2 sm:gap-3">
        <Button
          asChild
          variant="outline"
          size="sm"
          className="h-8 gap-2 sm:h-9"
          aria-label="Export"
        >
          <Link to="/reports">
            <Download className="size-4" aria-hidden="true" />
            <span className="hidden sm:inline">Export</span>
          </Link>
        </Button>
        <Button
          size="sm"
          className="h-8 gap-2 sm:h-9"
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

const StatsCards = () => {
  return (
    <div className="rounded-xl border bg-card">
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4 lg:divide-x">
        {statsData.map((stat) => {
          const formatter =
            stat.format === "currency" ? currencyFormatter : numberFormatter;

          return (
            <div key={stat.title} className="space-y-4 p-4 sm:p-6">
              <div className="text-muted-foreground">
                <span className="text-xs font-medium sm:text-sm">
                  {stat.title}
                </span>
              </div>
              <p className="text-2xl font-semibold tracking-tight sm:text-[28px]">
                {formatter.format(stat.value)}
              </p>
              <div className="flex flex-wrap items-center gap-2 text-[10px] sm:text-xs xl:flex-nowrap">
                <span
                  className={cn(
                    "font-medium",
                    stat.isPositive
                      ? "text-emerald-600 dark:text-emerald-400"
                      : "text-red-600 dark:text-red-400",
                  )}
                >
                  {stat.isPositive ? "+" : "-"}
                  {stat.changePercent.toFixed(1)}%
                  <span className="hidden sm:inline">
                    (
                    {formatter.format(
                      Math.abs(stat.value - stat.previousValue),
                    )}
                    )
                  </span>
                </span>
                <span className="hidden items-center gap-2 text-muted-foreground sm:inline-flex">
                  <span className="size-1 rounded-full bg-muted-foreground" />
                  <span className="xl:whitespace-nowrap">vs Last Month</span>
                </span>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

const RevenueSourceChart = () => {
  const { active: activeSegment, handleHover } = useHoverHighlight<number>();

  const revenueSourceItems = [
    {
      key: "onchain",
      label: "On-chain",
      ...revenueSourceData.onchain,
      color: palette.primary,
    },
    {
      key: "lightning",
      label: "Lightning",
      ...revenueSourceData.lightning,
      color: palette.secondary.light,
    },
    {
      key: "liquid",
      label: "Liquid",
      ...revenueSourceData.liquid,
      color: palette.tertiary.light,
    },
    {
      key: "manual",
      label: "Manual",
      ...revenueSourceData.manual,
      color: `color-mix(in oklch, var(--primary) 42%, ${mixBase})`,
    },
  ];

  return (
    <div className="flex flex-col gap-4 rounded-xl border bg-card p-4 sm:p-5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Revenue by source"
          >
            <Landmark className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium sm:text-base">
              Revenue by Source
            </span>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              {compactCurrencyFormatter.format(revenueSourceData.total)} this
              month
            </p>
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

      <div className="space-y-3">
        <div className="flex h-3 w-full overflow-hidden rounded-full sm:h-4">
          {revenueSourceItems.map((item, index) => (
            <ShadTooltipProvider key={item.key}>
              <ShadTooltip>
                <ShadTooltipTrigger asChild>
                  <button
                    type="button"
                    className={cn(
                      "h-full border-0 p-0 transition-opacity duration-200 motion-reduce:transition-none",
                      activeSegment !== null &&
                        activeSegment !== index &&
                        "opacity-40",
                    )}
                    style={{
                      width: `${item.percent}%`,
                      backgroundColor: item.color,
                    }}
                    onPointerEnter={() => handleHover(index)}
                    onPointerLeave={() => handleHover(null)}
                    onFocus={() => handleHover(index)}
                    onBlur={() => handleHover(null)}
                    aria-label={`${item.label}: ${currencyFormatter.format(item.value)} (${item.percent}%)`}
                  />
                </ShadTooltipTrigger>
                <ShadTooltipContent
                  side="top"
                  sideOffset={8}
                  className="px-3 py-2"
                >
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <span
                        className="size-2 rounded-full"
                        style={{ backgroundColor: item.color }}
                      />
                      <span className="font-medium">{item.label}</span>
                      <span className="text-muted-foreground tabular-nums">
                        {item.percent}%
                      </span>
                    </div>
                    <span className="text-muted-foreground tabular-nums">
                      {currencyFormatter.format(item.value)}
                    </span>
                  </div>
                </ShadTooltipContent>
              </ShadTooltip>
            </ShadTooltipProvider>
          ))}
        </div>

        <div className="flex items-center justify-between text-[10px] sm:text-xs">
          {revenueSourceItems.map((item, index) => (
            <span
              key={item.key}
              className={cn(
                "text-muted-foreground tabular-nums transition-opacity duration-200 motion-reduce:transition-none",
                activeSegment !== null &&
                  activeSegment !== index &&
                  "opacity-40",
              )}
            >
              {item.percent}%
            </span>
          ))}
        </div>

        <div className="flex flex-wrap items-center gap-3 sm:gap-4">
          {revenueSourceItems.map((item, index) => (
            <ShadTooltipProvider key={item.key}>
              <ShadTooltip>
                <ShadTooltipTrigger asChild>
                  <button
                    type="button"
                    className={cn(
                      "flex items-center gap-1.5 border-0 bg-transparent p-0 transition-opacity duration-200 motion-reduce:transition-none",
                      activeSegment !== null &&
                        activeSegment !== index &&
                        "opacity-40",
                    )}
                    onPointerEnter={() => handleHover(index)}
                    onPointerLeave={() => handleHover(null)}
                    onFocus={() => handleHover(index)}
                    onBlur={() => handleHover(null)}
                  >
                    <span
                      className="size-2.5 rounded-full sm:size-3"
                      style={{ backgroundColor: item.color }}
                    />
                    <span className="text-[10px] text-muted-foreground sm:text-xs">
                      {item.label}
                    </span>
                  </button>
                </ShadTooltipTrigger>
                <ShadTooltipContent
                  side="top"
                  sideOffset={8}
                  className="px-3 py-2"
                >
                  <div className="grid gap-1">
                    <div className="flex items-center gap-2">
                      <span
                        className="size-2 rounded-full"
                        style={{ backgroundColor: item.color }}
                      />
                      <span className="font-medium">{item.label}</span>
                      <span className="text-muted-foreground tabular-nums">
                        {item.percent}%
                      </span>
                    </div>
                    <span className="text-muted-foreground tabular-nums">
                      {currencyFormatter.format(item.value)}
                    </span>
                  </div>
                </ShadTooltipContent>
              </ShadTooltip>
            </ShadTooltipProvider>
          ))}
        </div>
      </div>
    </div>
  );
};

const SalesByCategoryChart = () => {
  const { active: activeSlice, handleHover: setHoveredSlice } =
    useHoverHighlight<number>();
  const totalSales = salesCategoryData.reduce(
    (acc, item) => acc + item.value,
    0,
  );

  return (
    <div className="flex flex-1 flex-col gap-4 rounded-xl border bg-card p-4 sm:p-5">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Sales by category"
          >
            <PieChartIcon className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium sm:text-base">
              Holdings by Source
            </span>
            <p className="flex items-center gap-1 text-[10px] text-muted-foreground sm:text-xs">
              <ArrowUpRight
                className="size-3 text-emerald-600"
                aria-hidden="true"
              />
              <span className="text-emerald-600">+8.4%</span>
              <span>vs cost basis</span>
            </p>
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

      <div className="flex flex-1 items-center gap-4 sm:gap-6">
        <div className="relative size-[100px] shrink-0 sm:size-[120px]">
          <ChartContainer
            config={salesCategoryChartConfig}
            className="h-full w-full"
          >
            <PieChart>
              <Pie
                data={salesCategoryData}
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
                {salesCategoryData.map((entry) => (
                  <Cell key={entry.name} fill={entry.color} />
                ))}
              </Pie>
            </PieChart>
          </ChartContainer>
          <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
            <span className="text-sm font-semibold sm:text-base">
              {compactCurrencyFormatter.format(totalSales)}
            </span>
            <span className="text-[8px] text-muted-foreground sm:text-[10px]">
              Total
            </span>
          </div>
        </div>

        <div className="flex flex-1 flex-col gap-2 sm:gap-3">
          {salesCategoryData.map((item, index) => (
            <div
              key={item.name}
              className={cn(
                "flex items-center justify-between gap-2 transition-opacity duration-200 motion-reduce:transition-none",
                activeSlice !== null && activeSlice !== index && "opacity-50",
              )}
              onMouseEnter={() => setHoveredSlice(index)}
              onMouseLeave={() => setHoveredSlice(null)}
            >
              <div className="flex items-center gap-2">
                <div
                  className="size-2 rounded-full sm:size-2.5"
                  style={{ backgroundColor: item.color }}
                />
                <span className="text-[10px] text-muted-foreground sm:text-xs">
                  {item.name}
                </span>
              </div>
              <div className="flex items-center gap-2 text-[10px] sm:text-xs">
                <span className="font-medium tabular-nums">
                  {compactCurrencyFormatter.format(item.value)}
                </span>
                <span className="text-muted-foreground tabular-nums">
                  {item.percent}%
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
};

const SideChartsSection = () => {
  return (
    <div className="flex w-full flex-col gap-4 xl:w-[410px]">
      <RevenueSourceChart />
      <SalesByCategoryChart />
    </div>
  );
};

interface RevenueTooltipPayload {
  dataKey?: string | number;
  value?: number | string;
}

interface RevenueTooltipProps {
  active?: boolean;
  payload?: RevenueTooltipPayload[];
  label?: string | number;
  colors: RevenueFlowColors;
}

function CustomTooltip({
  active,
  payload,
  label,
  colors,
}: RevenueTooltipProps) {
  if (!active || !payload?.length) return null;

  const thisYear = payload.find((p) => p.dataKey === "thisYear")?.value || 0;
  const prevYear = payload.find((p) => p.dataKey === "prevYear")?.value || 0;
  const diff = Number(thisYear) - Number(prevYear);
  const percentage = prevYear ? Math.round((diff / Number(prevYear)) * 100) : 0;
  const currentYear = new Date().getFullYear();

  return (
    <div className="rounded-lg border border-border bg-popover p-2 shadow-lg sm:p-3">
      <p className="mb-1.5 text-xs font-medium text-foreground sm:mb-2 sm:text-sm">
        {label}, {currentYear}
      </p>
      <div className="space-y-1 sm:space-y-1.5">
        <div className="flex items-center gap-1.5 sm:gap-2">
          <div
            className="size-2 rounded-full sm:size-2.5"
            style={{ backgroundColor: colors.thisYear }}
          />
          <span className="text-[10px] text-muted-foreground sm:text-sm">
            Value:
          </span>
          <span className="text-[10px] font-medium text-foreground sm:text-sm">
            {currencyFormatter.format(Number(thisYear))}
          </span>
        </div>
        <div className="flex items-center gap-1.5 sm:gap-2">
          <div
            className="size-2 rounded-full sm:size-2.5"
            style={{ backgroundColor: colors.prevYear }}
          />
          <span className="text-[10px] text-muted-foreground sm:text-sm">
            Cost Basis:
          </span>
          <span className="text-[10px] font-medium text-foreground sm:text-sm">
            {currencyFormatter.format(Number(prevYear))}
          </span>
        </div>
        <div className="mt-1 border-t border-border pt-1">
          <span
            className={cn(
              "text-[10px] font-medium sm:text-xs",
              diff >= 0 ? "text-emerald-500" : "text-red-500",
            )}
          >
            {diff >= 0 ? "+" : ""}
            {percentage}% vs basis
          </span>
        </div>
      </div>
    </div>
  );
}

const RevenueFlowChart = () => {
  const [period, setPeriod] = React.useState<TimePeriod>("1year");
  const { active: activeSeries, handleHover } = useHoverHighlight<
    "thisYear" | "prevYear"
  >();

  const legendItems = [
    { key: "thisYear", label: "Value", color: palette.primary },
    { key: "prevYear", label: "Cost Basis", color: palette.secondary.light },
  ] as const;

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const nextPeriod = params.get("period");
    if (periodKeys.includes(nextPeriod as TimePeriod)) {
      setPeriod(nextPeriod as TimePeriod);
    }
  }, []);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    if (period !== "1year") {
      params.set("period", period);
    } else {
      params.delete("period");
    }
    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [period]);

  const chartData = getDataForPeriod(period);
  const totalRevenue = chartData.reduce((acc, item) => acc + item.thisYear, 0);

  const renderChartCard = (expanded = false) => (
    <div className="flex min-w-0 flex-1 flex-col gap-4 rounded-xl border bg-card p-4 sm:gap-6 sm:p-6">
      <div className="flex flex-wrap items-center gap-2 sm:gap-4">
        <div className="flex flex-1 flex-col gap-1">
          <p className="text-xl leading-tight font-semibold tracking-tight sm:text-2xl">
            {currencyFormatter.format(totalRevenue)}
          </p>
          <p className="text-xs text-muted-foreground">
            Portfolio Value ({periodLabels[period]})
          </p>
        </div>
        <div className="hidden items-center gap-3 sm:flex sm:gap-5">
          {legendItems.map((item) => (
            <div
              key={item.key}
              className={cn(
                "flex items-center gap-1.5 transition-opacity duration-200 motion-reduce:transition-none",
                activeSeries !== null &&
                  activeSeries !== item.key &&
                  "opacity-40",
              )}
              onMouseEnter={() => handleHover(item.key)}
              onMouseLeave={() => handleHover(null)}
            >
              <div
                className="size-2.5 rounded-full sm:size-3"
                style={{ backgroundColor: item.color }}
              />
              <span className="text-[10px] text-muted-foreground sm:text-xs">
                {item.label}
              </span>
            </div>
          ))}
        </div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 sm:size-8"
              aria-label="Select time period"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48">
            <DropdownMenuLabel>Time Period</DropdownMenuLabel>
            <DropdownMenuSeparator />
            {periodKeys.map((key) => (
              <DropdownMenuCheckboxItem
                key={key}
                checked={period === key}
                onCheckedChange={() => setPeriod(key)}
              >
                {periodLabels[key]}
              </DropdownMenuCheckboxItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        {!expanded && (
          <DialogTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 sm:size-8"
              aria-label="Expand portfolio value chart"
            >
              <Maximize2 className="size-4" aria-hidden="true" />
            </Button>
          </DialogTrigger>
        )}
      </div>

      <div
        className={
          expanded
            ? "h-[min(62vh,620px)] w-full min-w-0"
            : "h-[200px] w-full min-w-0 sm:h-[240px] lg:h-[280px]"
        }
      >
        <ChartContainer
          config={revenueFlowChartConfig}
          className="h-full w-full"
        >
          <AreaChart data={chartData}>
            <defs>
              <linearGradient
                id={expanded ? "thisYearGradientExpanded" : "thisYearGradient"}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stopColor="var(--color-thisYear)"
                  stopOpacity={0.3}
                />
                <stop
                  offset="100%"
                  stopColor="var(--color-thisYear)"
                  stopOpacity={0.05}
                />
              </linearGradient>
              <linearGradient
                id={expanded ? "prevYearGradientExpanded" : "prevYearGradient"}
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stopColor="var(--color-prevYear)"
                  stopOpacity={0.2}
                />
                <stop
                  offset="100%"
                  stopColor="var(--color-prevYear)"
                  stopOpacity={0.02}
                />
              </linearGradient>
            </defs>
            <CartesianGrid strokeDasharray="0" vertical={false} />
            <XAxis
              dataKey="month"
              axisLine={false}
              tickLine={false}
              tick={{ fontSize: 10 }}
              dy={8}
            />
            <YAxis
              axisLine={false}
              tickLine={false}
              tick={{ fontSize: 10 }}
              dx={-5}
              tickFormatter={(value) => compactCurrencyFormatter.format(value)}
              width={40}
            />
            <Tooltip
              content={
                <CustomTooltip
                  colors={{
                    thisYear: "var(--color-thisYear)",
                    prevYear: "var(--color-prevYear)",
                  }}
                />
              }
              cursor={{ strokeOpacity: 0.2 }}
            />
            <Area
              type="monotone"
              dataKey="thisYear"
              stroke="var(--color-thisYear)"
              strokeWidth={activeSeries === "thisYear" ? 3 : 2}
              fill={`url(#${expanded ? "thisYearGradientExpanded" : "thisYearGradient"})`}
              fillOpacity={
                activeSeries === null || activeSeries === "thisYear" ? 1 : 0.3
              }
              strokeOpacity={
                activeSeries === null || activeSeries === "thisYear" ? 1 : 0.3
              }
            />
            <Area
              type="monotone"
              dataKey="prevYear"
              stroke="var(--color-prevYear)"
              strokeWidth={activeSeries === "prevYear" ? 3 : 2}
              fill={`url(#${expanded ? "prevYearGradientExpanded" : "prevYearGradient"})`}
              fillOpacity={
                activeSeries === null || activeSeries === "prevYear" ? 1 : 0.3
              }
              strokeOpacity={
                activeSeries === null || activeSeries === "prevYear" ? 1 : 0.3
              }
            />
          </AreaChart>
        </ChartContainer>
      </div>
    </div>
  );

  return (
    <Dialog>
      {renderChartCard()}
      <DialogContent className="max-w-[calc(100vw-2rem)] p-0 sm:max-w-[min(1120px,calc(100vw-2rem))]">
        <DialogTitle className="sr-only">
          Expanded portfolio value chart
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

const RecentTransactionsTable = ({ className }: { className?: string }) => {
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
    if (statusFilter === "all") return transactionRecords;
    return transactionRecords.filter((t) => t.status === statusFilter);
  }, [statusFilter]);

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
    <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-4 pt-4 sm:px-6">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0 sm:size-8"
            aria-label="Recent transactions"
          >
            <ClipboardList className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <span className="text-sm font-medium sm:text-base">
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

      <div className="px-4 pt-3 pb-4 sm:px-6">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">
                Record
              </TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">
                Account
              </TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">
                Total
              </TableHead>
              <TableHead className="text-xs font-medium text-muted-foreground sm:text-sm">
                Status
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {paginatedTransactions.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={4}
                  className="h-20 text-center text-sm text-muted-foreground"
                >
                  No transactions found.
                </TableCell>
              </TableRow>
            ) : (
              paginatedTransactions.map((t) => (
                <TableRow key={t.id}>
                  <TableCell className="text-xs font-medium text-muted-foreground sm:text-sm">
                    {t.txid}
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground sm:text-sm">
                    {t.counterparty}
                  </TableCell>
                  <TableCell className="text-xs text-foreground tabular-nums sm:text-sm">
                    {currencyFormatter.format(t.amount)}
                  </TableCell>
                  <TableCell>
                    <span
                      className={cn(
                        "inline-flex items-center rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                        statusStyles[t.status],
                      )}
                    >
                      {statusLabels[t.status]}
                    </span>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>

      <div className="flex items-center justify-between border-t px-4 py-3 text-[10px] text-muted-foreground sm:px-6 sm:text-xs">
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
  );
};

const RecentActivity = ({ className }: { className?: string }) => {
  return (
    <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-4 pt-4 sm:px-6">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0 sm:size-8"
            aria-label="Recent activity"
          >
            <Bell className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <span className="text-sm font-medium sm:text-base">
            Recent Activity
          </span>
        </div>
      </div>

      <ScrollArea className="h-[360px] px-4 pt-2 pb-4 text-xs sm:px-6 sm:text-sm">
        <div className="divide-y">
          {recentActivity.map((item) => (
            <div key={`${item.title}-${item.time}`} className="py-3">
              <div className="flex items-center justify-between gap-3">
                <span className="font-medium text-foreground">
                  {item.title}
                </span>
                <span className="text-[10px] text-muted-foreground sm:text-xs">
                  {item.time}
                </span>
              </div>
              <span className="text-[10px] text-muted-foreground sm:text-xs">
                {item.detail}
              </span>
            </div>
          ))}
        </div>
      </ScrollArea>
    </div>
  );
};

const Dashboard5 = ({ className }: { className?: string }) => {
  const [addConnectionOpen, setAddConnectionOpen] = React.useState(false);

  return (
    <>
      <div
        className={cn(
          "w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6",
          className,
        )}
      >
        <WelcomeSection onAddConnection={() => setAddConnectionOpen(true)} />
        <StatsCards />
        <div className="flex flex-col gap-4 sm:gap-6 xl:flex-row">
          <RevenueFlowChart />
          <SideChartsSection />
        </div>
        <div className="flex flex-col gap-4 xl:flex-row">
          <RecentTransactionsTable className="flex-1" />
          <RecentActivity className="xl:w-[360px]" />
        </div>
      </div>
      <AddConnectionFlow
        open={addConnectionOpen}
        onClose={() => setAddConnectionOpen(false)}
      />
    </>
  );
};

export { Dashboard5 };
