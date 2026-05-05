import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  Clock,
  Copy,
  Eye,
  ExternalLink,
  Filter,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  ShieldAlert,
  ShoppingCart,
  Wallet,
  X,
  XCircle,
} from "lucide-react";
import * as React from "react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Button } from "@/components/ui/button";
import { type ChartConfig, ChartContainer } from "@/components/ui/chart";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";
import { formatBtc, useCurrency, type Currency } from "@/lib/currency";
import {
  explorerTargetForTransaction,
  type ExplorerSettings,
} from "@/lib/explorer";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";
import type { Tx } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

type TransactionStatus = "completed" | "pending" | "failed" | "review";

type TransactionDirection = "Receive" | "Send" | "Transfer";

type TransactionFlow = "incoming" | "outgoing" | "transfer" | "swap";

type Transaction = {
  id: string;
  txnId: string;
  explorerId?: string;
  amount: number;
  amountBtc?: number;
  counterparty: string;
  counterpartyInitials: string;
  direction: TransactionDirection;
  flow?: TransactionFlow;
  wallet?: string;
  tag?: string;
  paymentMethod: "On-chain" | "Exchange" | "Lightning" | "Liquid";
  date: string;
  status: TransactionStatus;
};

type PeriodKey = "ytd" | "30days" | "3months" | "1year" | "5years";

type FlowChartPoint = {
  date: string;
  incoming: number;
  outgoing: number;
  transfers: number;
  swaps: number;
};

type FlowBucket = {
  key: string;
  label: string;
};

type SwapCandidate = {
  in: Transaction;
  out: Transaction;
  eur: number;
  btc: number;
};

const currencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
});

const compactCurrencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
  notation: "compact",
  maximumFractionDigits: 0,
});

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

function formatInlineBtc(btc: number, precision = 8) {
  return `₿${Math.abs(btc).toFixed(precision)}`;
}

function formatDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return formatInlineBtc(btc);
  return currencyFormatter.format(eur);
}

function formatShortTxid(txid: string) {
  if (txid.length <= 18) return txid;
  return `${txid.slice(0, 10)}...${txid.slice(-6)}`;
}

function transactionBtc(txn: Transaction) {
  return txn.amountBtc ?? 0;
}

function transactionFlow(txn: Transaction): TransactionFlow {
  if (txn.flow) return txn.flow;
  if (txn.tag?.toLowerCase().includes("swap")) return "swap";
  if (txn.direction === "Transfer") return "transfer";
  return txn.direction === "Receive" ? "incoming" : "outgoing";
}

const flowColors: Record<TransactionFlow, string> = {
  incoming: "oklch(0.56 0.16 150)",
  outgoing: "oklch(0.58 0.2 27)",
  transfer: "oklch(0.56 0.04 260)",
  swap: "oklch(0.62 0.16 246)",
};

const flowChartConfig = {
  incoming: { label: "Incoming", color: flowColors.incoming },
  outgoing: {
    label: "Outgoing",
    color: flowColors.outgoing,
  },
  transfers: {
    label: "Transfers",
    color: flowColors.transfer,
  },
  swaps: {
    label: "Swaps",
    color: flowColors.swap,
  },
} satisfies ChartConfig;

const transactionRecords: Transaction[] = [
  {
    id: "1",
    txnId: "TXN-100201",
    amount: 2499.0,
    counterparty: "Cold Storage",
    counterpartyInitials: "CS",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "Today",
    status: "completed",
  },
  {
    id: "2",
    txnId: "TXN-100202",
    amount: 850.0,
    counterparty: "Bitstamp",
    counterpartyInitials: "BS",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "Today",
    status: "completed",
  },
  {
    id: "3",
    txnId: "TXN-100203",
    amount: 124.5,
    counterparty: "Phoenix Wallet",
    counterpartyInitials: "PW",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "Today",
    status: "pending",
  },
  {
    id: "4",
    txnId: "TXN-100204",
    amount: 89.4,
    counterparty: "Hetzner",
    counterpartyInitials: "HZ",
    direction: "Send",
    paymentMethod: "On-chain",
    date: "Today",
    status: "review",
  },
  {
    id: "5",
    txnId: "TXN-100205",
    amount: 35.0,
    counterparty: "Voltage Cloud",
    counterpartyInitials: "VC",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "Today",
    status: "failed",
  },
  {
    id: "6",
    txnId: "TXN-100206",
    amount: 5000.0,
    counterparty: "Multisig Vault",
    counterpartyInitials: "MV",
    direction: "Transfer",
    paymentMethod: "On-chain",
    date: "1 day ago",
    status: "completed",
  },
  {
    id: "7",
    txnId: "TXN-100207",
    amount: 248.0,
    counterparty: "BTCPay Server",
    counterpartyInitials: "BP",
    direction: "Receive",
    paymentMethod: "Lightning",
    date: "1 day ago",
    status: "completed",
  },
  {
    id: "8",
    txnId: "TXN-100208",
    amount: 1500.0,
    counterparty: "Kraken",
    counterpartyInitials: "KR",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "1 day ago",
    status: "pending",
  },
  {
    id: "9",
    txnId: "TXN-100209",
    amount: 750.0,
    counterparty: "Bitcoin Austria",
    counterpartyInitials: "BA",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "1 day ago",
    status: "completed",
  },
  {
    id: "10",
    txnId: "TXN-100210",
    amount: 12.0,
    counterparty: "Mullvad VPN",
    counterpartyInitials: "MU",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "2 days ago",
    status: "completed",
  },
  {
    id: "11",
    txnId: "TXN-100211",
    amount: 3200.0,
    counterparty: "ACME GmbH",
    counterpartyInitials: "AG",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "2 days ago",
    status: "completed",
  },
  {
    id: "12",
    txnId: "TXN-100212",
    amount: 8000.0,
    counterparty: "Hardware Wallet",
    counterpartyInitials: "HW",
    direction: "Transfer",
    paymentMethod: "On-chain",
    date: "2 days ago",
    status: "completed",
  },
  {
    id: "13",
    txnId: "TXN-100213",
    amount: 250.0,
    counterparty: "Coinbase",
    counterpartyInitials: "CB",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "2 days ago",
    status: "failed",
  },
  {
    id: "14",
    txnId: "TXN-100214",
    amount: 67.5,
    counterparty: "Alby Hub",
    counterpartyInitials: "AH",
    direction: "Receive",
    paymentMethod: "Lightning",
    date: "3 days ago",
    status: "completed",
  },
  {
    id: "15",
    txnId: "TXN-100215",
    amount: 99.0,
    counterparty: "Bitrefill",
    counterpartyInitials: "BR",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "3 days ago",
    status: "completed",
  },
  {
    id: "16",
    txnId: "TXN-100216",
    amount: 2500.0,
    counterparty: "River Financial",
    counterpartyInitials: "RF",
    direction: "Receive",
    paymentMethod: "Exchange",
    date: "3 days ago",
    status: "completed",
  },
  {
    id: "17",
    txnId: "TXN-100217",
    amount: 50.0,
    counterparty: "Strike",
    counterpartyInitials: "SK",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "3 days ago",
    status: "completed",
  },
  {
    id: "18",
    txnId: "TXN-100218",
    amount: 320.0,
    counterparty: "Bull Bitcoin",
    counterpartyInitials: "BB",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "4 days ago",
    status: "pending",
  },
  {
    id: "19",
    txnId: "TXN-100219",
    amount: 150.0,
    counterparty: "Mobile Wallet",
    counterpartyInitials: "MW",
    direction: "Transfer",
    paymentMethod: "Lightning",
    date: "4 days ago",
    status: "completed",
  },
  {
    id: "20",
    txnId: "TXN-100220",
    amount: 100.0,
    counterparty: "OpenSats",
    counterpartyInitials: "OS",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "5 days ago",
    status: "completed",
  },
  {
    id: "21",
    txnId: "TXN-100221",
    amount: 88.0,
    counterparty: "Plebs Verein",
    counterpartyInitials: "PV",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "5 days ago",
    status: "review",
  },
  {
    id: "22",
    txnId: "TXN-100222",
    amount: 199.0,
    counterparty: "Lightning Labs",
    counterpartyInitials: "LL",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "5 days ago",
    status: "completed",
  },
  {
    id: "23",
    txnId: "TXN-100223",
    amount: 45.0,
    counterparty: "Cashu Wallet",
    counterpartyInitials: "CW",
    direction: "Send",
    paymentMethod: "Liquid",
    date: "6 days ago",
    status: "completed",
  },
  {
    id: "24",
    txnId: "TXN-100224",
    amount: 22.5,
    counterparty: "Sphinx Chat",
    counterpartyInitials: "SC",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "6 days ago",
    status: "completed",
  },
  {
    id: "25",
    txnId: "TXN-100225",
    amount: 4200.0,
    counterparty: "Bitstamp",
    counterpartyInitials: "BS",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "7 days ago",
    status: "completed",
  },
  {
    id: "26",
    txnId: "TXN-100226",
    amount: 8.5,
    counterparty: "Tip Jar",
    counterpartyInitials: "TJ",
    direction: "Receive",
    paymentMethod: "Lightning",
    date: "7 days ago",
    status: "completed",
  },
  {
    id: "27",
    txnId: "TXN-100227",
    amount: 1500.0,
    counterparty: "Personal Vault",
    counterpartyInitials: "PV",
    direction: "Transfer",
    paymentMethod: "On-chain",
    date: "7 days ago",
    status: "pending",
  },
  {
    id: "28",
    txnId: "TXN-100228",
    amount: 600.0,
    counterparty: "Bitpanda",
    counterpartyInitials: "BD",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "8 days ago",
    status: "completed",
  },
  {
    id: "29",
    txnId: "TXN-100229",
    amount: 25.0,
    counterparty: "Voltage Cloud",
    counterpartyInitials: "VC",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "8 days ago",
    status: "completed",
  },
  {
    id: "30",
    txnId: "TXN-100230",
    amount: 12000.0,
    counterparty: "Treasury",
    counterpartyInitials: "TR",
    direction: "Transfer",
    paymentMethod: "Liquid",
    date: "9 days ago",
    status: "completed",
  },
  {
    id: "31",
    txnId: "TXN-100231",
    amount: 18.5,
    counterparty: "Phoenix Wallet",
    counterpartyInitials: "PW",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "9 days ago",
    status: "failed",
  },
  {
    id: "32",
    txnId: "TXN-100232",
    amount: 89.4,
    counterparty: "Hetzner",
    counterpartyInitials: "HZ",
    direction: "Send",
    paymentMethod: "On-chain",
    date: "10 days ago",
    status: "completed",
  },
  {
    id: "33",
    txnId: "TXN-100233",
    amount: 2200.0,
    counterparty: "Hardware Wallet",
    counterpartyInitials: "HW",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "10 days ago",
    status: "review",
  },
  {
    id: "34",
    txnId: "TXN-100234",
    amount: 35.0,
    counterparty: "Start9",
    counterpartyInitials: "S9",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "10 days ago",
    status: "completed",
  },
  {
    id: "35",
    txnId: "TXN-100235",
    amount: 75.0,
    counterparty: "Bitcoin Pizza",
    counterpartyInitials: "BZ",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "12 days ago",
    status: "completed",
  },
  {
    id: "36",
    txnId: "TXN-100236",
    amount: 9500.0,
    counterparty: "Cold Storage",
    counterpartyInitials: "CS",
    direction: "Transfer",
    paymentMethod: "On-chain",
    date: "12 days ago",
    status: "completed",
  },
  {
    id: "37",
    txnId: "TXN-100237",
    amount: 1850.0,
    counterparty: "ACME GmbH",
    counterpartyInitials: "AG",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "14 days ago",
    status: "pending",
  },
  {
    id: "38",
    txnId: "TXN-100238",
    amount: 14.0,
    counterparty: "Umbrel",
    counterpartyInitials: "UM",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "14 days ago",
    status: "completed",
  },
  {
    id: "39",
    txnId: "TXN-100239",
    amount: 750.0,
    counterparty: "Bitstamp",
    counterpartyInitials: "BS",
    direction: "Send",
    paymentMethod: "Exchange",
    date: "15 days ago",
    status: "completed",
  },
  {
    id: "40",
    txnId: "TXN-100240",
    amount: 32.5,
    counterparty: "Mobile Wallet",
    counterpartyInitials: "MW",
    direction: "Receive",
    paymentMethod: "Lightning",
    date: "16 days ago",
    status: "completed",
  },
  {
    id: "41",
    txnId: "TXN-100241",
    amount: 1200.0,
    counterparty: "River Financial",
    counterpartyInitials: "RF",
    direction: "Receive",
    paymentMethod: "Exchange",
    date: "17 days ago",
    status: "completed",
  },
  {
    id: "42",
    txnId: "TXN-100242",
    amount: 12.0,
    counterparty: "Mullvad VPN",
    counterpartyInitials: "MU",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "18 days ago",
    status: "completed",
  },
  {
    id: "43",
    txnId: "TXN-100243",
    amount: 7500.0,
    counterparty: "Multisig Vault",
    counterpartyInitials: "MV",
    direction: "Transfer",
    paymentMethod: "On-chain",
    date: "18 days ago",
    status: "completed",
  },
  {
    id: "44",
    txnId: "TXN-100244",
    amount: 500.0,
    counterparty: "Bitcoin Austria",
    counterpartyInitials: "BA",
    direction: "Send",
    paymentMethod: "On-chain",
    date: "20 days ago",
    status: "review",
  },
  {
    id: "45",
    txnId: "TXN-100245",
    amount: 45.0,
    counterparty: "Cashu Wallet",
    counterpartyInitials: "CW",
    direction: "Receive",
    paymentMethod: "Liquid",
    date: "22 days ago",
    status: "completed",
  },
  {
    id: "46",
    txnId: "TXN-100246",
    amount: 199.0,
    counterparty: "Lightning Labs",
    counterpartyInitials: "LL",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "22 days ago",
    status: "completed",
  },
  {
    id: "47",
    txnId: "TXN-100247",
    amount: 200.0,
    counterparty: "Hot Wallet",
    counterpartyInitials: "HT",
    direction: "Transfer",
    paymentMethod: "Lightning",
    date: "25 days ago",
    status: "completed",
  },
  {
    id: "48",
    txnId: "TXN-100248",
    amount: 1800.0,
    counterparty: "Watch-only Wallet",
    counterpartyInitials: "WO",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "27 days ago",
    status: "completed",
  },
  {
    id: "49",
    txnId: "TXN-100249",
    amount: 75.0,
    counterparty: "Strike",
    counterpartyInitials: "SK",
    direction: "Send",
    paymentMethod: "Lightning",
    date: "30 days ago",
    status: "completed",
  },
  {
    id: "50",
    txnId: "TXN-100250",
    amount: 4500.0,
    counterparty: "BTCPay Server",
    counterpartyInitials: "BP",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "33 days ago",
    status: "completed",
  },
];

function toDashboardTransaction(tx: Tx, index: number): Transaction {
  const tag = tx.tag || "";
  const isSwap =
    tag.toLowerCase().includes("swap") ||
    tx.type === "Swap" ||
    tx.type === "Mint" ||
    tx.type === "Melt";
  const flow: TransactionFlow = isSwap
    ? "swap"
    : tx.internal || tx.type === "Transfer" || tx.type === "Rebalance"
      ? "transfer"
      : tx.amountSat >= 0
        ? "incoming"
        : "outgoing";
  const direction: TransactionDirection =
    flow === "transfer" || flow === "swap"
      ? "Transfer"
      : flow === "incoming"
        ? "Receive"
        : "Send";
  const status: TransactionStatus = tx.conf > 0 ? "completed" : "pending";
  const paymentMethod =
    tx.account.toLowerCase().includes("lightning") ||
    tx.account.toLowerCase().includes("ln") ||
    tx.account.toLowerCase().includes("phoenix")
      ? "Lightning"
      : tx.account.toLowerCase().includes("liquid") ||
          tx.account.toLowerCase().includes("lbtc")
        ? "Liquid"
        : "On-chain";
  return {
    id: tx.id,
    txnId: tx.externalId || tx.id || `TX-${index + 1}`,
    explorerId: tx.explorerId || undefined,
    amount: Math.abs(tx.eur || (tx.amountSat / 100_000_000) * tx.rate),
    amountBtc: Math.abs(tx.amountSat / 100_000_000),
    counterparty: tx.counter || tx.account || "Unassigned",
    counterpartyInitials: initials(tx.counter || tx.account || "TX"),
    direction,
    flow,
    wallet: tx.account || "Unassigned wallet",
    tag,
    paymentMethod,
    date: tx.date,
    status: tag.toLowerCase().includes("review") ? "review" : status,
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

const PAGE_SIZE_OPTIONS = [10, 25, 50, 100];

const periodLabels: Record<PeriodKey, string> = {
  ytd: "YTD",
  "1year": "1 Year",
  "3months": "3 Months",
  "30days": "30 Days",
  "5years": "5 Years",
};

const periodKeys: PeriodKey[] = [
  "30days",
  "3months",
  "ytd",
  "1year",
  "5years",
];

function periodLimit(period: PeriodKey) {
  if (period === "30days") return 10;
  if (period === "3months") return 18;
  if (period === "ytd") return 40;
  if (period === "5years") return 60;
  return 30;
}

function recordsForPeriod(records: Transaction[], period: PeriodKey) {
  const dated = records
    .map((record) => ({ record, date: parseTransactionDate(record.date) }))
    .filter(
      (entry): entry is { record: Transaction; date: Date } =>
        entry.date !== null,
  );

  if (!dated.length) {
    return records.slice(0, periodLimit(period));
  }

  const end = periodAnchorDate(dated.map((entry) => entry.date));
  const start = periodStartDate(end, period);

  return dated
    .filter((entry) => entry.date >= start && entry.date <= end)
    .sort((a, b) => b.date.getTime() - a.date.getTime())
    .map((entry) => entry.record);
}

function parseTransactionDate(value: string) {
  const normalized = value.includes("T") ? value : value.replace(" ", "T");
  const parsed = new Date(normalized);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
}

function startOfLocalDay(date: Date) {
  const start = new Date(date);
  start.setHours(0, 0, 0, 0);
  return start;
}

function periodAnchorDate(dates: Date[]) {
  const now = new Date();
  if (!dates.length) return now;
  const latest = dates.reduce((max, date) => (date > max ? date : max), dates[0]);
  return latest > now ? latest : now;
}

function periodStartDate(end: Date, period: PeriodKey) {
  const start = startOfLocalDay(end);
  if (period === "30days") {
    start.setDate(start.getDate() - 29);
  } else if (period === "3months") {
    start.setMonth(start.getMonth() - 3);
  } else if (period === "ytd") {
    start.setMonth(0, 1);
    start.setHours(0, 0, 0, 0);
  } else if (period === "5years") {
    start.setFullYear(start.getFullYear() - 5);
  } else {
    start.setFullYear(start.getFullYear() - 1);
  }
  return start;
}

function startOfIsoWeek(date: Date) {
  const start = new Date(date);
  const day = start.getDay() || 7;
  start.setDate(start.getDate() - day + 1);
  start.setHours(0, 0, 0, 0);
  return start;
}

function monthLabel(date: Date) {
  return date.toLocaleDateString("en-US", {
    month: "short",
    year: "2-digit",
  });
}

function quarterLabel(date: Date) {
  return `Q${Math.floor(date.getMonth() / 3) + 1} ${String(
    date.getFullYear(),
  ).slice(-2)}`;
}

function localDateKey(date: Date) {
  return [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, "0"),
    String(date.getDate()).padStart(2, "0"),
  ].join("-");
}

function addBucketStep(date: Date, period: PeriodKey) {
  const next = new Date(date);
  if (period === "30days") {
    next.setDate(next.getDate() + 1);
  } else if (period === "3months") {
    next.setDate(next.getDate() + 7);
  } else if (period === "5years") {
    next.setMonth(next.getMonth() + 3);
  } else {
    next.setMonth(next.getMonth() + 1);
  }
  return next;
}

function bucketTransactionDate(date: Date, period: PeriodKey): FlowBucket {
  if (period === "30days") {
    return {
      key: localDateKey(date),
      label: localDateKey(date),
    };
  }
  if (period === "3months") {
    const week = startOfIsoWeek(date);
    return {
      key: localDateKey(week),
      label: `Week ${week.toLocaleDateString("en-US", {
        month: "short",
        day: "numeric",
      })}`,
    };
  }
  if (period === "5years") {
    const quarterStart = new Date(date);
    quarterStart.setMonth(Math.floor(date.getMonth() / 3) * 3, 1);
    quarterStart.setHours(0, 0, 0, 0);
    return {
      key: `${quarterStart.getFullYear()}-Q${
        Math.floor(quarterStart.getMonth() / 3) + 1
      }`,
      label: quarterLabel(quarterStart),
    };
  }
  return {
    key: `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`,
    label: monthLabel(date),
  };
}

function buildEmptyFlowBuckets(
  period: PeriodKey,
  records: Transaction[],
): Map<string, FlowChartPoint> {
  const grouped = new Map<string, FlowChartPoint>();
  const dated = records
    .map((record) => parseTransactionDate(record.date))
    .filter((date): date is Date => date !== null);
  if (!dated.length) return grouped;

  const end = periodAnchorDate(dated);
  let cursor = periodStartDate(end, period);
  if (period === "3months") cursor = startOfIsoWeek(cursor);
  if (period === "5years") {
    cursor.setMonth(Math.floor(cursor.getMonth() / 3) * 3, 1);
    cursor.setHours(0, 0, 0, 0);
  }

  while (cursor <= end) {
    const bucket = bucketTransactionDate(cursor, period);
    grouped.set(bucket.key, {
      date: bucket.label,
      incoming: 0,
      outgoing: 0,
      transfers: 0,
      swaps: 0,
    });
    cursor = addBucketStep(cursor, period);
  }
  return grouped;
}

function flowBucketLabel(period: PeriodKey) {
  if (period === "30days") return "day";
  if (period === "3months") return "week";
  if (period === "5years") return "quarter";
  return "month";
}

function sumByFlow(records: Transaction[], flow: TransactionFlow) {
  const rows = records.filter((txn) => transactionFlow(txn) === flow);
  return {
    count: rows.length,
    eur: rows.reduce((sum, txn) => sum + txn.amount, 0),
    btc: rows.reduce((sum, txn) => sum + transactionBtc(txn), 0),
  };
}

function buildSwapCandidates(records: Transaction[]): SwapCandidate[] {
  const inbound = records
    .filter((txn) => transactionFlow(txn) === "incoming")
    .map((txn) => ({ txn, date: parseTransactionDate(txn.date) }))
    .filter(
      (entry): entry is { txn: Transaction; date: Date } => entry.date !== null,
    );
  const outbound = records
    .filter((txn) => transactionFlow(txn) === "outgoing")
    .map((txn) => ({ txn, date: parseTransactionDate(txn.date) }))
    .filter(
      (entry): entry is { txn: Transaction; date: Date } => entry.date !== null,
    );
  const usedInbound = new Set<string>();
  const candidates: SwapCandidate[] = [];

  for (const out of outbound) {
    let best:
      | {
          txn: Transaction;
          date: Date;
          score: number;
        }
      | null = null;
    for (const input of inbound) {
      if (usedInbound.has(input.txn.id)) continue;
      if (input.txn.wallet === out.txn.wallet) continue;
      if (input.txn.paymentMethod === out.txn.paymentMethod) continue;
      const deltaMs = Math.abs(input.date.getTime() - out.date.getTime());
      if (deltaMs > 6 * 60 * 60 * 1000) continue;
      const largerBtc = Math.max(transactionBtc(input.txn), transactionBtc(out.txn));
      const smallerBtc = Math.min(transactionBtc(input.txn), transactionBtc(out.txn));
      if (largerBtc <= 0) continue;
      const relativeDiff = (largerBtc - smallerBtc) / largerBtc;
      if (relativeDiff > 0.03) continue;
      const score = relativeDiff + deltaMs / (6 * 60 * 60 * 1000);
      if (!best || score < best.score) {
        best = { ...input, score };
      }
    }
    if (!best) continue;
    usedInbound.add(best.txn.id);
    candidates.push({
      in: best.txn,
      out: out.txn,
      eur: Math.min(best.txn.amount, out.txn.amount),
      btc: Math.min(transactionBtc(best.txn), transactionBtc(out.txn)),
    });
  }

  return candidates;
}

function swapCandidateDelta(candidate: SwapCandidate) {
  const inDate = parseTransactionDate(candidate.in.date);
  const outDate = parseTransactionDate(candidate.out.date);
  if (!inDate || !outDate) return null;
  return Math.abs(inDate.getTime() - outDate.getTime());
}

function formatCandidateDelta(candidate: SwapCandidate) {
  const deltaMs = swapCandidateDelta(candidate);
  if (deltaMs === null) return "Unknown timing";
  const minutes = Math.round(deltaMs / 60_000);
  if (minutes < 60) return `${minutes} min apart`;
  const hours = Math.round((minutes / 60) * 10) / 10;
  return `${hours} h apart`;
}

function buildFlowChartRows(
  records: Transaction[],
  period: PeriodKey,
  currency: Currency,
  candidateIds = new Set<string>(),
): FlowChartPoint[] {
  const grouped = buildEmptyFlowBuckets(period, records);

  for (const txn of records) {
    const parsedDate = parseTransactionDate(txn.date);
    const bucket = parsedDate
      ? bucketTransactionDate(parsedDate, period)
      : { key: txn.date || "Unknown", label: txn.date || "Unknown" };
    const row =
      grouped.get(bucket.key) ??
      {
        date: bucket.label,
        incoming: 0,
        outgoing: 0,
        transfers: 0,
        swaps: 0,
      };
    const value = currency === "btc" ? transactionBtc(txn) : txn.amount;
    const flow = candidateIds.has(txn.id) ? "swap" : transactionFlow(txn);
    if (flow === "incoming") row.incoming += value;
    if (flow === "outgoing") row.outgoing += value;
    if (flow === "transfer") row.transfers += value;
    if (flow === "swap") row.swaps += value;
    grouped.set(bucket.key, row);
  }

  return Array.from(grouped.values());
}

function buildBreakdown<T extends string>(
  records: Transaction[],
  getKey: (txn: Transaction) => T,
) {
  const rows = new Map<T, { key: T; count: number; eur: number; btc: number }>();
  for (const txn of records) {
    const key = getKey(txn);
    const row = rows.get(key) ?? { key, count: 0, eur: 0, btc: 0 };
    row.count += 1;
    row.eur += txn.amount;
    row.btc += transactionBtc(txn);
    rows.set(key, row);
  }
  return Array.from(rows.values()).sort((a, b) => b.eur - a.eur);
}

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
}

interface ChartTooltipProps {
  active?: boolean;
  payload?: ChartTooltipPayload[];
  label?: string | number;
  hideSensitive: boolean;
  currency: Currency;
}

const TransactionWorkbench = ({
  period,
  records,
  hideSensitive,
  currency,
  explorerSettings,
}: {
  period: PeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
}) => {
  const [swapDialogOpen, setSwapDialogOpen] = React.useState(false);
  const [largestExplorerTransaction, setLargestExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const largestExplorerTarget = largestExplorerTransaction
    ? explorerForTransaction(largestExplorerTransaction, explorerSettings)
    : null;
  const swapCandidates = buildSwapCandidates(records);
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
      eur: sum.eur + candidate.eur,
      btc: sum.btc + candidate.btc,
    }),
    { count: 0, eur: 0, btc: 0 },
  );
  const swaps = {
    count: markedSwaps.count + swapCandidateTotals.count,
    eur: markedSwaps.eur + swapCandidateTotals.eur,
    btc: markedSwaps.btc + swapCandidateTotals.btc,
  };
  const netEur = incoming.eur - outgoing.eur;
  const netBtc = incoming.btc - outgoing.btc;
  const reviewCount = records.filter((txn) => txn.status === "review").length;
  const pendingCount = records.filter((txn) => txn.status === "pending").length;
  const failedCount = records.filter((txn) => txn.status === "failed").length;
  const withoutExplorer = records.filter((txn) => !txn.explorerId).length;
  const chartRows = buildFlowChartRows(records, period, currency, swapCandidateIds);
  const networkRows = buildBreakdown(records, (txn) => txn.paymentMethod);
  const walletRows = buildBreakdown(records, (txn) => txn.wallet ?? "Unassigned");
  const maxNetworkValue = Math.max(...networkRows.map((row) => row.eur), 1);
  const maxWalletValue = Math.max(...walletRows.map((row) => row.eur), 1);
  const largest = records.reduce<Transaction | null>(
    (current, txn) => (!current || txn.amount > current.amount ? txn : current),
    null,
  );
  const largestExplorer = largest
    ? explorerForTransaction(largest, explorerSettings)
    : null;
  const metricCards = [
    {
      label: "Incoming",
      value: incoming,
      meta: `${incoming.count} tx`,
      icon: ArrowDownRight,
      tone: "text-emerald-600",
    },
    {
      label: "Outgoing",
      value: outgoing,
      meta: `${outgoing.count} tx`,
      icon: ArrowUpRight,
      tone: "text-red-600",
    },
    {
      label: "Net flow",
      value: { eur: netEur, btc: netBtc },
      meta: netEur >= 0 ? "inflow" : "outflow",
      icon: ArrowLeftRight,
      tone: netEur >= 0 ? "text-emerald-600" : "text-red-600",
    },
    {
      label: "Transfers",
      value: transfers,
      meta: `${transfers.count} moves`,
      icon: Wallet,
      tone: "text-muted-foreground",
    },
    {
      label: "Swap candidates",
      value: swaps,
      meta:
        swapCandidateTotals.count > 0
          ? `${swapCandidateTotals.count} unpaired`
          : `${markedSwaps.count} marked`,
      icon: RefreshCw,
      tone: swapCandidateTotals.count > 0 ? "text-amber-600" : "text-muted-foreground",
      onClick:
        swapCandidateTotals.count > 0
          ? () => setSwapDialogOpen(true)
          : undefined,
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
    },
  ];

  return (
    <section className="rounded-xl border bg-card">
      <div className="grid grid-cols-2 border-b md:grid-cols-3 xl:grid-cols-6">
        {metricCards.map((metric, index) => {
          const Icon = metric.icon;
          const className = cn(
            "min-w-0 space-y-2 p-3 text-left sm:p-4",
            index > 0 && "border-l",
            index === 3 && "md:border-l-0 xl:border-l",
            metric.onClick &&
              "w-full cursor-pointer transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
          );
          const content = (
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
          return metric.onClick ? (
            <button
              key={metric.label}
              type="button"
              className={className}
              onClick={metric.onClick}
              aria-label="Open swap candidates"
            >
              {content}
            </button>
          ) : (
            <div key={metric.label} className={className}>
              {content}
            </div>
          );
        })}
      </div>

      <Dialog open={swapDialogOpen} onOpenChange={setSwapDialogOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Swap candidates</DialogTitle>
            <DialogDescription>
              Cross-wallet, cross-network legs that match by time and amount.
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-[60vh] space-y-3 overflow-y-auto pr-1">
            {swapCandidates.map((candidate, index) => (
              <div
                key={`${candidate.out.id}-${candidate.in.id}`}
                className="rounded-lg border bg-background p-3"
              >
                <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <p className="text-sm font-medium">Candidate {index + 1}</p>
                    <p className="text-xs text-muted-foreground">
                      {formatCandidateDelta(candidate)}
                    </p>
                  </div>
                  <CurrencyToggleText
                    className={cn(
                      "text-right text-sm font-semibold",
                      blurClass(hideSensitive),
                    )}
                  >
                    {formatDisplayMoney(candidate.eur, candidate.btc, currency)}
                  </CurrencyToggleText>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  <SwapCandidateLeg
                    title="Outgoing leg"
                    transaction={candidate.out}
                    currency={currency}
                    hideSensitive={hideSensitive}
                  />
                  <SwapCandidateLeg
                    title="Incoming leg"
                    transaction={candidate.in}
                    currency={currency}
                    hideSensitive={hideSensitive}
                  />
                </div>
              </div>
            ))}
          </div>
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Close
              </Button>
            </DialogClose>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <div className="grid gap-0 lg:grid-cols-[minmax(0,1.35fr)_minmax(300px,0.65fr)]">
        <div className="border-b p-3 lg:border-r lg:border-b-0 sm:p-4">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">
                Flow by {flowBucketLabel(period)}
              </h2>
              <p className="text-xs text-muted-foreground">
                Incoming, outgoing, transfers, and swaps by {flowBucketLabel(period)}.
              </p>
            </div>
            <div className="flex flex-wrap justify-end gap-x-3 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
              {[
                ["incoming", "Incoming"],
                ["outgoing", "Outgoing"],
                ["transfer", "Transfers"],
                ["swap", "Swaps"],
              ].map(([flow, label]) => (
                <span key={flow} className="inline-flex items-center gap-1.5">
                  <span
                    className="size-2.5 rounded-sm"
                    style={{ backgroundColor: flowColors[flow as TransactionFlow] }}
                    aria-hidden="true"
                  />
                  {label}
                </span>
              ))}
            </div>
          </div>
          <div className="h-[190px] min-w-0">
            <ChartContainer config={flowChartConfig} className="h-full w-full">
              <BarChart data={chartRows}>
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
                  tickFormatter={(value) =>
                    hideSensitive
                      ? ""
                      : currency === "btc"
                        ? formatInlineBtc(Number(value), 4)
                        : compactCurrencyFormatter.format(value)
                  }
                />
                <Tooltip
                  cursor={{ fillOpacity: 0.05 }}
                  content={
                    <FlowTooltip
                      hideSensitive={hideSensitive}
                      currency={currency}
                    />
                  }
                />
                <Bar
                  dataKey="incoming"
                  fill={flowColors.incoming}
                  radius={[2, 2, 0, 0]}
                />
                <Bar
                  dataKey="outgoing"
                  fill={flowColors.outgoing}
                  radius={[2, 2, 0, 0]}
                />
                <Bar
                  dataKey="transfers"
                  fill={flowColors.transfer}
                  radius={[2, 2, 0, 0]}
                />
                <Bar
                  dataKey="swaps"
                  fill={flowColors.swap}
                  radius={[2, 2, 0, 0]}
                />
              </BarChart>
            </ChartContainer>
          </div>
        </div>

        <div className="grid gap-0 sm:grid-cols-2 lg:grid-cols-1">
          <BreakdownPanel
            title="Network mix"
            rows={networkRows}
            maxValue={maxNetworkValue}
            currency={currency}
            hideSensitive={hideSensitive}
          />
          <BreakdownPanel
            title="Wallet/source mix"
            rows={walletRows.slice(0, 4)}
            maxValue={maxWalletValue}
            currency={currency}
            hideSensitive={hideSensitive}
          />
          <div className="border-t p-3 sm:col-span-2 lg:col-span-1 sm:p-4">
            <h3 className="mb-3 text-sm font-semibold">Accounting checks</h3>
            <div className="grid grid-cols-2 gap-2 text-xs">
              <CheckPill label="Needs review" value={reviewCount} />
              <CheckPill label="Pending conf." value={pendingCount} />
              <CheckPill
                label="Swap candidates"
                value={swapCandidateTotals.count}
                onClick={
                  swapCandidateTotals.count > 0
                    ? () => setSwapDialogOpen(true)
                    : undefined
                }
              />
              <CheckPill label="No explorer id" value={withoutExplorer} />
            </div>
            {largest ? (
              <button
                type="button"
                className={cn(
                  "mt-3 w-full rounded-lg border bg-background p-2 text-left text-xs",
                  largestExplorer &&
                    "cursor-pointer transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
                )}
                onClick={() => {
                  if (largestExplorer) setLargestExplorerTransaction(largest);
                }}
                disabled={!largestExplorer}
                title={
                  largestExplorer
                    ? `Open ${largest.txnId} on ${largestExplorer.label}`
                    : undefined
                }
              >
                <div className="text-muted-foreground">Largest transaction</div>
                <div className="mt-1 flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex min-w-0 items-center gap-1 font-mono font-medium">
                      <span className="truncate">
                        {formatShortTxid(largest.txnId)}
                      </span>
                      {largestExplorer ? (
                        <ExternalLink
                          className="size-3 shrink-0 text-muted-foreground"
                          aria-hidden="true"
                        />
                      ) : null}
                    </div>
                    <div className="mt-0.5 truncate text-muted-foreground">
                      {largest.wallet} · {largest.paymentMethod} · {largest.date}
                    </div>
                  </div>
                  <span className={cn("font-semibold", blurClass(hideSensitive))}>
                    {formatDisplayMoney(
                      largest.amount,
                      transactionBtc(largest),
                      currency,
                    )}
                  </span>
                </div>
              </button>
            ) : null}
          </div>
        </div>
      </div>
      <Dialog
        open={Boolean(largestExplorerTransaction)}
        onOpenChange={(open) => {
          if (!open) setLargestExplorerTransaction(null);
        }}
      >
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <div className="mb-2 flex size-10 items-center justify-center rounded-full bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
              <ShieldAlert className="size-5" aria-hidden="true" />
            </div>
            <DialogTitle>Open transaction in a browser?</DialogTitle>
            <DialogDescription>
              This opens {largestExplorerTarget?.label ?? "a public explorer"} outside
              Kassiber. The explorer can see your IP address and the transaction
              id you request.
            </DialogDescription>
          </DialogHeader>
          {largestExplorerTransaction && largestExplorerTarget ? (
            <div className="rounded-md border bg-muted/35 p-3 text-sm">
              <p className="font-medium">{largestExplorerTransaction.txnId}</p>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {largestExplorerTarget.url}
              </p>
            </div>
          ) : null}
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="button"
              disabled={!largestExplorerTarget}
              onClick={() => {
                if (!largestExplorerTarget || typeof window === "undefined") return;
                window.open(
                  largestExplorerTarget.url,
                  "_blank",
                  "noopener,noreferrer",
                );
                setLargestExplorerTransaction(null);
              }}
            >
              <ExternalLink className="size-4" aria-hidden="true" />
              Open explorer
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
};

function SwapCandidateLeg({
  title,
  transaction,
  hideSensitive,
  currency,
}: {
  title: string;
  transaction: Transaction;
  hideSensitive: boolean;
  currency: Currency;
}) {
  return (
    <div className="min-w-0 rounded-md border bg-muted/25 p-3 text-xs">
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="font-medium">{title}</span>
        <span className="rounded-md border bg-background px-1.5 py-0.5 text-muted-foreground">
          {transaction.paymentMethod}
        </span>
      </div>
      <div className="space-y-1.5">
        <div className="flex justify-between gap-3">
          <span className="text-muted-foreground">Wallet</span>
          <span className={cn("truncate text-right", blurClass(hideSensitive))}>
            {transaction.wallet}
          </span>
        </div>
        <div className="flex justify-between gap-3">
          <span className="text-muted-foreground">Date</span>
          <span>{transaction.date}</span>
        </div>
        <div className="flex justify-between gap-3">
          <span className="text-muted-foreground">Amount</span>
          <CurrencyToggleText
            className={cn("font-medium", blurClass(hideSensitive))}
          >
            {formatDisplayMoney(
              transaction.amount,
              transactionBtc(transaction),
              currency,
            )}
          </CurrencyToggleText>
        </div>
        <div className="flex justify-between gap-3">
          <span className="text-muted-foreground">Label</span>
          <span className={cn("truncate text-right", blurClass(hideSensitive))}>
            {transaction.tag || "Unlabeled"}
          </span>
        </div>
        <div className="flex justify-between gap-3">
          <span className="text-muted-foreground">Tx</span>
          <span
            className={cn(
              "truncate text-right font-mono",
              blurClass(hideSensitive),
            )}
          >
            {transaction.txnId}
          </span>
        </div>
      </div>
    </div>
  );
}

function FlowTooltip({
  active,
  payload,
  label,
  hideSensitive,
  currency,
}: ChartTooltipProps) {
  if (!active || !payload?.length) return null;
  const rows = payload.filter((row) => Number(row.value ?? 0) > 0);
  return (
    <div className="rounded-lg border bg-popover p-3 text-xs shadow-lg">
      <p className="mb-2 font-medium">{label}</p>
      <div className="space-y-1.5">
        {rows.map((row) => (
          <div key={String(row.dataKey)} className="flex items-center gap-2">
            <span
              className="size-2 rounded-sm"
              style={{
                backgroundColor:
                  flowColors[
                    (row.dataKey === "transfers"
                      ? "transfer"
                      : row.dataKey === "swaps"
                        ? "swap"
                        : row.dataKey) as TransactionFlow
                  ] ?? "currentColor",
              }}
              aria-hidden="true"
            />
            <span className="capitalize text-muted-foreground">
              {String(row.dataKey)}
            </span>
            <span className={cn("ml-auto font-medium", blurClass(hideSensitive))}>
              {currency === "btc"
                ? formatBtc(Number(row.value))
                : currencyFormatter.format(Number(row.value))}
            </span>
          </div>
        ))}
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
}: {
  title: string;
  rows: Array<{ key: string; count: number; eur: number; btc: number }>;
  maxValue: number;
  currency: Currency;
  hideSensitive: boolean;
}) {
  return (
    <div className="border-t p-3 first:border-t-0 sm:p-4 lg:first:border-t">
      <h3 className="mb-3 text-sm font-semibold">{title}</h3>
      <div className="space-y-2.5">
        {rows.map((row) => (
          <div key={row.key} className="space-y-1">
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
          </div>
        ))}
      </div>
    </div>
  );
}

function CheckPill({
  label,
  value,
  onClick,
}: {
  label: string;
  value: number;
  onClick?: () => void;
}) {
  const className = cn(
    "rounded-lg border bg-background p-2 text-left",
    onClick &&
      "w-full cursor-pointer transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
  );
  const content = (
    <>
      <div className="text-muted-foreground">{label}</div>
      <div
        className={cn(
          "mt-1 text-lg font-semibold",
          value > 0 ? "text-amber-600" : "text-emerald-600",
        )}
      >
        {value}
      </div>
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

const transactionStatusStyles: Record<TransactionStatus, string> = {
  completed:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  pending:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  failed:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/20 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
  review:
    "bg-blue-50 text-blue-700 ring-1 ring-inset ring-blue-700/10 dark:bg-blue-900/30 dark:text-blue-400 dark:ring-blue-400/20",
};

const transactionStatusIcons: Record<
  TransactionStatus,
  React.ComponentType<React.SVGProps<SVGSVGElement>>
> = {
  completed: CheckCircle2,
  pending: Clock,
  failed: XCircle,
  review: RotateCcw,
};

const transactionStatusLabels: Record<TransactionStatus, string> = {
  completed: "Confirmed",
  pending: "Pending",
  failed: "Failed",
  review: "Needs review",
};

const allTransactionStatuses: TransactionStatus[] = [
  "completed",
  "pending",
  "failed",
  "review",
];

const allPaymentMethods = [
  "On-chain",
  "Exchange",
  "Lightning",
  "Liquid",
] as const;

const transactionFlowLabels: Record<TransactionFlow, string> = {
  incoming: "Incoming",
  outgoing: "Outgoing",
  transfer: "Transfer",
  swap: "Swap",
};

const transactionFlowStyles: Record<TransactionFlow, string> = {
  incoming:
    "border-emerald-600/20 bg-emerald-50 text-emerald-700 dark:bg-emerald-900/25 dark:text-emerald-300",
  outgoing:
    "border-red-600/20 bg-red-50 text-red-700 dark:bg-red-900/25 dark:text-red-300",
  transfer:
    "border-zinc-500/20 bg-zinc-50 text-zinc-700 dark:bg-zinc-800/70 dark:text-zinc-300",
  swap:
    "border-sky-600/20 bg-sky-50 text-sky-700 dark:bg-sky-900/25 dark:text-sky-300",
};

const allTransactionFlows: TransactionFlow[] = [
  "incoming",
  "outgoing",
  "transfer",
  "swap",
];

function explorerForTransaction(
  txn: Transaction,
  settings: ExplorerSettings,
) {
  if (txn.paymentMethod === "Liquid") {
    return explorerTargetForTransaction({
      txid: txn.explorerId,
      network: "liquid",
      settings,
    });
  }
  if (txn.paymentMethod === "On-chain") {
    return explorerTargetForTransaction({
      txid: txn.explorerId,
      network: "bitcoin",
      settings,
    });
  }
  return null;
}

const dateFilterOptions = [
  { label: "All", value: "all" },
  { label: "Today", value: "today" },
  { label: "Yesterday", value: "yesterday" },
  { label: "Last 7 days", value: "7days" },
  { label: "Last 30 days", value: "30days" },
];

const filterChipClassName =
  "inline-flex h-5 cursor-pointer items-center gap-1 rounded-md bg-gray-50 px-2 text-[10px] font-medium text-gray-600 ring-1 ring-inset ring-gray-500/10 sm:h-6 sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20";

const TransactionsTable = ({
  records,
  hideSensitive,
  currency,
  explorerSettings,
  swapCandidateIds = new Set<string>(),
}: {
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  swapCandidateIds?: Set<string>;
}) => {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState<string>("all");
  const [dateFilter, setDateFilter] = React.useState<string>("all");
  const [flowFilter, setFlowFilter] = React.useState<string>("all");
  const [paymentMethodFilter, setPaymentMethodFilter] =
    React.useState<string>("all");
  const [currentPage, setCurrentPage] = React.useState(1);
  const [pageSize, setPageSize] = React.useState(10);
  const [isHydrated, setIsHydrated] = React.useState(false);
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const displayFlow = React.useCallback(
    (txn: Transaction): TransactionFlow =>
      swapCandidateIds.has(txn.id) ? "swap" : transactionFlow(txn),
    [swapCandidateIds],
  );

  const hasActiveFilters =
    statusFilter !== "all" ||
    dateFilter !== "all" ||
    flowFilter !== "all" ||
    paymentMethodFilter !== "all";

  const clearFilters = () => {
    setStatusFilter("all");
    setDateFilter("all");
    setFlowFilter("all");
    setPaymentMethodFilter("all");
  };

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    setSearchQuery(params.get("q") ?? "");

    const nextStatus = params.get("status");
    if (
      nextStatus &&
      (nextStatus === "all" ||
        allTransactionStatuses.includes(nextStatus as TransactionStatus))
    ) {
      setStatusFilter(nextStatus);
    }

    const nextDate = params.get("date");
    if (
      nextDate &&
      dateFilterOptions.some((option) => option.value === nextDate)
    ) {
      setDateFilter(nextDate);
    }

    const nextFlow = params.get("flow");
    if (
      nextFlow &&
      (nextFlow === "all" ||
        allTransactionFlows.includes(nextFlow as TransactionFlow))
    ) {
      setFlowFilter(nextFlow);
    }

    const nextPayment = params.get("payment");
    if (
      nextPayment &&
      (nextPayment === "all" ||
        allPaymentMethods.includes(
          nextPayment as (typeof allPaymentMethods)[number],
        ))
    ) {
      setPaymentMethodFilter(nextPayment);
    }

    const nextPage = Number(params.get("page"));
    if (!Number.isNaN(nextPage) && nextPage > 0) {
      setCurrentPage(nextPage);
    }

    const nextPageSize = Number(params.get("pageSize"));
    if (
      !Number.isNaN(nextPageSize) &&
      PAGE_SIZE_OPTIONS.includes(nextPageSize)
    ) {
      setPageSize(nextPageSize);
    }

    setIsHydrated(true);
  }, []);

  const filteredTransactions = React.useMemo(() => {
    const query = searchQuery.toLowerCase();
    return records.filter((txn) => {
      const matchesSearch =
        txn.txnId.toLowerCase().includes(query) ||
        txn.counterparty.toLowerCase().includes(query) ||
        (txn.wallet ?? "").toLowerCase().includes(query) ||
        (txn.tag ?? "").toLowerCase().includes(query) ||
        txn.paymentMethod.toLowerCase().includes(query);

      const matchesStatus =
        statusFilter === "all" || txn.status === statusFilter;

      const matchesFlow =
        flowFilter === "all" || displayFlow(txn) === flowFilter;

      const matchesPaymentMethod =
        paymentMethodFilter === "all" ||
        txn.paymentMethod === paymentMethodFilter;

      let matchesDate = true;
      const pd = txn.date.toLowerCase();
      switch (dateFilter) {
        case "today":
          matchesDate = pd === "today";
          break;
        case "yesterday":
          matchesDate = pd === "1 day ago";
          break;
        case "7days":
          matchesDate =
            pd === "today" ||
            pd.includes("day ago") ||
            (pd.includes("days ago") && parseInt(pd) <= 7);
          break;
        case "30days":
          matchesDate =
            pd === "today" ||
            pd.includes("day ago") ||
            (pd.includes("days ago") && parseInt(pd) <= 30);
          break;
      }

      return (
        matchesSearch &&
        matchesStatus &&
        matchesFlow &&
        matchesPaymentMethod &&
        matchesDate
      );
    });
  }, [
    records,
    searchQuery,
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    displayFlow,
  ]);

  const totalPages = Math.ceil(filteredTransactions.length / pageSize);

  const paginatedTransactions = React.useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    const endIndex = startIndex + pageSize;
    return filteredTransactions.slice(startIndex, endIndex);
  }, [filteredTransactions, currentPage, pageSize]);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [
    searchQuery,
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    pageSize,
  ]);

  React.useEffect(() => {
    if (!isHydrated || typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);

    if (searchQuery) {
      params.set("q", searchQuery);
    } else {
      params.delete("q");
    }

    if (statusFilter !== "all") {
      params.set("status", statusFilter);
    } else {
      params.delete("status");
    }

    if (dateFilter !== "all") {
      params.set("date", dateFilter);
    } else {
      params.delete("date");
    }

    if (flowFilter !== "all") {
      params.set("flow", flowFilter);
    } else {
      params.delete("flow");
    }

    if (paymentMethodFilter !== "all") {
      params.set("payment", paymentMethodFilter);
    } else {
      params.delete("payment");
    }

    if (currentPage > 1) {
      params.set("page", String(currentPage));
    } else {
      params.delete("page");
    }

    if (pageSize !== PAGE_SIZE_OPTIONS[0]) {
      params.set("pageSize", String(pageSize));
    } else {
      params.delete("pageSize");
    }

    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [
    searchQuery,
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    currentPage,
    pageSize,
    isHydrated,
  ]);

  const goToPage = (page: number) => {
    setCurrentPage(Math.max(1, Math.min(page, totalPages)));
  };

  return (
    <>
      <div className="rounded-xl border bg-card">
      <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5">
        <div className="flex flex-1 items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0 sm:size-8"
            aria-label="Transactions"
          >
            <ShoppingCart className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <span className="text-sm font-medium sm:text-base">Transactions</span>
          <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {filteredTransactions.length}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <div className="relative flex-1 sm:flex-none">
            <Search
              className="absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground sm:size-5"
              aria-hidden="true"
            />
            <Input
              type="search"
              name="transactions-search"
              inputMode="search"
              autoComplete="off"
              aria-label="Search transactions"
              placeholder="Search txid, wallet, label, tag..."
              value={searchQuery}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
                setSearchQuery(e.target.value)
              }
              className="h-8 w-full pl-9 text-sm sm:h-9 sm:w-[160px] sm:pl-10 lg:w-[200px]"
            />
          </div>

          <Select value={dateFilter} onValueChange={setDateFilter}>
            <SelectTrigger
              className="h-8 w-[120px] text-xs sm:h-9 sm:w-[140px] sm:text-sm"
              aria-label="Filter by date"
            >
              <SelectValue placeholder="Date" />
            </SelectTrigger>
            <SelectContent>
              {dateFilterOptions.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  statusFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by status"
              >
                <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Status</span>
                {statusFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
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
              {allTransactionStatuses.map((status) => (
                <DropdownMenuCheckboxItem
                  key={status}
                  checked={statusFilter === status}
                  onCheckedChange={() => setStatusFilter(status)}
                >
                  {transactionStatusLabels[status]}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  flowFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by flow"
              >
                <ArrowLeftRight
                  className="size-3.5 sm:size-4"
                  aria-hidden="true"
                />
                <span className="hidden sm:inline">Flow</span>
                {flowFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[190px]">
              <DropdownMenuLabel>Filter by flow</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={flowFilter === "all"}
                onCheckedChange={() => setFlowFilter("all")}
              >
                All flows
              </DropdownMenuCheckboxItem>
              {allTransactionFlows.map((flow) => (
                <DropdownMenuCheckboxItem
                  key={flow}
                  checked={flowFilter === flow}
                  onCheckedChange={() => setFlowFilter(flow)}
                >
                  {transactionFlowLabels[flow]}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  paymentMethodFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by payment method"
              >
                <Wallet className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Network</span>
                {paymentMethodFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[200px]">
              <DropdownMenuLabel>Filter by network</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={paymentMethodFilter === "all"}
                onCheckedChange={() => setPaymentMethodFilter("all")}
              >
                All networks
              </DropdownMenuCheckboxItem>
              {allPaymentMethods.map((method) => (
                <DropdownMenuCheckboxItem
                  key={method}
                  checked={paymentMethodFilter === method}
                  onCheckedChange={() => setPaymentMethodFilter(method)}
                >
                  {method}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {hasActiveFilters && (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            Filters:
          </span>
          {statusFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setStatusFilter("all")}
              aria-label={`Clear ${transactionStatusLabels[statusFilter as TransactionStatus]} filter`}
            >
              {transactionStatusLabels[statusFilter as TransactionStatus]}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {dateFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setDateFilter("all")}
              aria-label={`Clear ${dateFilterOptions.find((o) => o.value === dateFilter)?.label} filter`}
            >
              {dateFilterOptions.find((o) => o.value === dateFilter)?.label}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {flowFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setFlowFilter("all")}
              aria-label={`Clear ${transactionFlowLabels[flowFilter as TransactionFlow]} filter`}
            >
              {transactionFlowLabels[flowFilter as TransactionFlow]}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {paymentMethodFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setPaymentMethodFilter("all")}
              aria-label={`Clear ${paymentMethodFilter} filter`}
            >
              {paymentMethodFilter}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          <button
            onClick={clearFilters}
            className="text-[10px] text-destructive hover:underline sm:text-xs"
          >
            Clear all
          </button>
        </div>
      )}

      <div className="overflow-x-auto px-3 pb-3 sm:px-6 sm:pb-4">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Transaction ID
              </TableHead>
              <TableHead className="hidden min-w-[140px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                Wallet/source
              </TableHead>
              <TableHead className="min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Amount
              </TableHead>
              <TableHead className="hidden min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Flow
              </TableHead>
              <TableHead className="hidden min-w-[120px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                Network
              </TableHead>
              <TableHead className="hidden text-xs font-medium text-muted-foreground sm:table-cell sm:text-sm">
                Date
              </TableHead>
              <TableHead className="min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Status
              </TableHead>
              <TableHead className="w-[40px]"></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {paginatedTransactions.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={8}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  No transactions found matching your filters.
                </TableCell>
              </TableRow>
            ) : (
              paginatedTransactions.map((txn) => {
                const StatusIcon = transactionStatusIcons[txn.status];
                const explorer = explorerForTransaction(txn, explorerSettings);
                const flow = displayFlow(txn);
                return (
                  <TableRow key={txn.id}>
                    <TableCell
                      className={cn(
                        "text-xs font-medium sm:text-sm",
                        blurClass(hideSensitive),
                      )}
                    >
                      {explorer ? (
                        <button
                          type="button"
                          className="inline-flex max-w-[28ch] items-center gap-1 truncate font-mono text-left underline-offset-4 hover:underline"
                          title={`Open ${txn.txnId} on ${explorer.label}`}
                          onClick={() => setExplorerTransaction(txn)}
                        >
                          <span className="truncate">{txn.txnId}</span>
                          <ExternalLink
                            className="size-3 shrink-0 text-muted-foreground"
                            aria-hidden="true"
                          />
                        </button>
                      ) : (
                        <span className="font-mono">{txn.txnId}</span>
                      )}
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div className="flex items-center gap-2">
                        <Avatar className="size-6 bg-muted">
                          <AvatarFallback className="text-[8px] font-semibold text-muted-foreground uppercase">
                            {initials(txn.wallet || txn.counterparty || "TX")}
                          </AvatarFallback>
                        </Avatar>
                        <div className="min-w-0">
                          <p
                            className={cn(
                              "truncate text-xs text-foreground sm:text-sm",
                              blurClass(hideSensitive),
                            )}
                          >
                            {txn.wallet || txn.counterparty}
                          </p>
                          <p
                            className={cn(
                              "truncate text-[10px] text-muted-foreground sm:text-xs",
                              blurClass(hideSensitive),
                            )}
                          >
                            {txn.counterparty}
                          </p>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-xs text-foreground tabular-nums sm:text-sm",
                        blurClass(hideSensitive),
                      )}
                    >
                      <CurrencyToggleText>
                        {formatDisplayMoney(
                          txn.amount,
                          transactionBtc(txn),
                          currency,
                        )}
                      </CurrencyToggleText>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal sm:text-xs",
                          transactionFlowStyles[flow],
                        )}
                      >
                        {transactionFlowLabels[flow]}
                      </span>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <span className="inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal text-muted-foreground sm:text-xs">
                        {txn.paymentMethod}
                      </span>
                    </TableCell>
                    <TableCell className="hidden text-xs text-muted-foreground sm:table-cell sm:text-sm">
                      {txn.date}
                    </TableCell>
                    <TableCell>
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          transactionStatusStyles[txn.status],
                        )}
                      >
                        <StatusIcon className="size-3" aria-hidden="true" />
                        {transactionStatusLabels[txn.status]}
                      </span>
                    </TableCell>
                    <TableCell>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-7 text-muted-foreground hover:text-foreground sm:size-8"
                            aria-label={`Open actions for ${txn.txnId}`}
                          >
                            <MoreHorizontal className="size-3.5 sm:size-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem>
                            <Eye className="mr-2 size-4" aria-hidden="true" />
                            View Details
                          </DropdownMenuItem>
                          <DropdownMenuItem>
                            <Pencil
                              className="mr-2 size-4"
                              aria-hidden="true"
                            />
                            Edit Metadata
                          </DropdownMenuItem>
                          <DropdownMenuItem>
                            <Copy className="mr-2 size-4" aria-hidden="true" />
                            Duplicate
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive"
                            onSelect={(event: Event) => {
                              event.preventDefault();
                              if (typeof window === "undefined") return;
                              window.confirm(
                                "Void this transaction? This cannot be undone.",
                              );
                            }}
                          >
                            <X className="mr-2 size-4" aria-hidden="true" />
                            Exclude Transaction
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>

      <div className="flex flex-col items-center justify-between gap-3 border-t px-3 py-3 sm:flex-row sm:px-6">
        <div className="flex items-center gap-2 text-xs text-muted-foreground sm:text-sm">
          <span className="hidden sm:inline">Rows per page:</span>
          <Select
            value={pageSize.toString()}
            onValueChange={(value: string) => setPageSize(Number(value))}
          >
            <SelectTrigger className="h-8 w-[70px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PAGE_SIZE_OPTIONS.map((size) => (
                <SelectItem key={size} value={size.toString()}>
                  {size}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="text-muted-foreground">
            {filteredTransactions.length === 0
              ? "0"
              : `${(currentPage - 1) * pageSize + 1}-${Math.min(
                  currentPage * pageSize,
                  filteredTransactions.length,
                )}`}{" "}
            of {filteredTransactions.length}
          </span>
        </div>

        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(1)}
            disabled={currentPage === 1}
            aria-label="Go to first page"
          >
            <ChevronsLeft className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(currentPage - 1)}
            disabled={currentPage === 1}
            aria-label="Go to previous page"
          >
            <ChevronLeft className="size-4" />
          </Button>

          <div className="flex items-center gap-1 px-2">
            {Array.from({ length: Math.min(totalPages, 5) }, (_, i) => {
              let pageNum: number;
              if (totalPages <= 5) {
                pageNum = i + 1;
              } else if (currentPage <= 3) {
                pageNum = i + 1;
              } else if (currentPage >= totalPages - 2) {
                pageNum = totalPages - 4 + i;
              } else {
                pageNum = currentPage - 2 + i;
              }

              return (
                <Button
                  key={pageNum}
                  variant={currentPage === pageNum ? "default" : "ghost"}
                  size="icon"
                  className="size-8"
                  onClick={() => goToPage(pageNum)}
                >
                  {pageNum}
                </Button>
              );
            })}
          </div>

          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(currentPage + 1)}
            disabled={currentPage === totalPages || totalPages === 0}
            aria-label="Go to next page"
          >
            <ChevronRight className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(totalPages)}
            disabled={currentPage === totalPages || totalPages === 0}
            aria-label="Go to last page"
          >
            <ChevronsRight className="size-4" />
          </Button>
        </div>
      </div>
      </div>
      <Dialog
        open={Boolean(explorerTransaction)}
        onOpenChange={(open) => {
          if (!open) setExplorerTransaction(null);
        }}
      >
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <div className="mb-2 flex size-10 items-center justify-center rounded-full bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
              <ShieldAlert className="size-5" aria-hidden="true" />
            </div>
            <DialogTitle>Open transaction in a browser?</DialogTitle>
            <DialogDescription>
              This opens {explorerTarget?.label ?? "a public explorer"} outside
              Kassiber. The explorer can see your IP address and the transaction
              id you request.
            </DialogDescription>
          </DialogHeader>
          {explorerTransaction && explorerTarget ? (
            <div className="rounded-md border bg-muted/35 p-3 text-sm">
              <p className="font-medium">{explorerTransaction.txnId}</p>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {explorerTarget.url}
              </p>
            </div>
          ) : null}
          <DialogFooter>
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="button"
              disabled={!explorerTarget}
              onClick={() => {
                if (!explorerTarget || typeof window === "undefined") return;
                window.open(explorerTarget.url, "_blank", "noopener,noreferrer");
                setExplorerTransaction(null);
              }}
            >
              <ExternalLink className="size-4" aria-hidden="true" />
              Open explorer
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
};

const Dashboard2 = ({
  className,
  transactions = MOCK_TRANSACTIONS,
}: {
  className?: string;
  transactions?: TransactionsList;
}) => {
  const [period, setPeriod] = React.useState<PeriodKey>("1year");
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [txnNote, setTxnNote] = React.useState("");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const records = React.useMemo(
    () =>
      transactions.txs.length
        ? transactions.txs.map(toDashboardTransaction)
        : transactionRecords,
    [transactions.txs],
  );
  const periodRecords = React.useMemo(
    () => recordsForPeriod(records, period),
    [records, period],
  );
  const periodSwapCandidateIds = React.useMemo(
    () =>
      new Set(
        buildSwapCandidates(periodRecords).flatMap((candidate) => [
          candidate.in.id,
          candidate.out.id,
        ]),
      ),
    [periodRecords],
  );

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const nextPeriod = params.get("period");
    if (periodKeys.includes(nextPeriod as PeriodKey)) {
      setPeriod(nextPeriod as PeriodKey);
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

  return (
    <div
      className={cn(
        "w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6",
        className,
      )}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <PeriodTabs activePeriod={period} onPeriodChange={setPeriod} />
        <div className="flex flex-wrap items-center gap-2 sm:gap-3">
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-2 sm:h-9"
            aria-label="Sync wallets"
            onClick={syncAll}
            disabled={isSyncing}
          >
            <RefreshCw
              className={cn("size-4", isSyncing && "animate-spin")}
              aria-hidden="true"
            />
            <span className="hidden sm:inline">
              {isSyncing ? "Syncing" : "Sync"}
            </span>
          </Button>
          <Dialog open={newTxnOpen} onOpenChange={setNewTxnOpen}>
            <DialogTrigger asChild>
              <Button
                size="sm"
                className="h-8 gap-2 sm:h-9"
                aria-label="New transaction"
              >
                <Plus className="size-4" aria-hidden="true" />
                <span className="hidden sm:inline">New transaction</span>
              </Button>
            </DialogTrigger>
            <DialogContent className="sm:max-w-md">
              <DialogHeader>
                <DialogTitle>Record a transaction</DialogTitle>
                <DialogDescription>
                  Capture a quick note for your close checklist. This demo does
                  not persist data.
                </DialogDescription>
              </DialogHeader>
              <div className="grid gap-4 py-2">
                <div className="grid gap-2">
                  <Label htmlFor="dashboard2-txn-note">Description</Label>
                  <Input
                    id="dashboard2-txn-note"
                    name="txn-note"
                    value={txnNote}
                    onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
                      setTxnNote(e.target.value)
                    }
                    placeholder="e.g. Wire — Q1 tax payment"
                  />
                </div>
              </div>
              <DialogFooter>
                <DialogClose asChild>
                  <Button type="button" variant="outline">
                    Cancel
                  </Button>
                </DialogClose>
                <Button
                  type="button"
                  onClick={() => {
                    setNewTxnOpen(false);
                    setTxnNote("");
                  }}
                >
                  Save draft
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </div>

      <TransactionWorkbench
        period={period}
        records={periodRecords}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
      />

      <TransactionsTable
        records={periodRecords}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        swapCandidateIds={periodSwapCandidateIds}
      />
    </div>
  );
};

export { Dashboard2 };
