import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  Bitcoin,
  BookMarked,
  CalendarClock,
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
  Hash,
  Link2,
  ListChecks,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Search,
  ShieldAlert,
  Tags,
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

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { type ChartConfig, ChartContainer } from "@/components/ui/chart";
import { openExternalUrl } from "@/daemon/transport";
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
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { formatBtc, useCurrency, type Currency } from "@/lib/currency";
import {
  explorerTargetForTransaction,
  type ExplorerSettings,
  type ExplorerTarget,
} from "@/lib/explorer";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";
import { MOCK_OVERVIEW, type Tx } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

type TransactionStatus = "completed" | "pending" | "failed" | "review";

type TransactionDirection = "Receive" | "Send" | "Transfer";

type TransactionFlow =
  | "incoming"
  | "outgoing"
  | "transfer"
  | "swap"
  | "layer-transition";

type Transaction = {
  id: string;
  txnId: string;
  explorerId?: string;
  amount: number;
  amountBtc?: number;
  feeBtc?: number;
  feeEur?: number;
  asset?: string;
  rate?: number;
  note?: string;
  counterparty: string;
  counterpartyInitials: string;
  direction: TransactionDirection;
  flow?: TransactionFlow;
  wallet?: string;
  tag?: string;
  sourceType?: Tx["type"];
  paymentMethod: "On-chain" | "Exchange" | "Lightning" | "Liquid";
  date: string;
  status: TransactionStatus;
};

type TransactionEditDraft = {
  label: string;
  tags: string[];
  note: string;
  taxTreatment: string;
  priceMode: "imported" | "rate-cache" | "manual" | "missing";
  manualCurrency: string;
  manualPrice: string;
  manualValue: string;
  manualSource: string;
  reviewStatus: TransactionStatus;
  taxable: boolean;
  excluded: boolean;
};

type NewTransactionDraft = {
  sourceKind: "onchain" | "offchain" | "exchange" | "manual" | "internal";
  flow: TransactionFlow;
  occurredAt: string;
  confirmedAt: string;
  wallet: string;
  counterparty: string;
  transactionId: string;
  fromWallet: string;
  toWallet: string;
  fromExternal: string;
  toExternal: string;
  swapService: string;
  asset: string;
  amountSats: string;
  sendAsset: string;
  receiveAsset: string;
  sendAmountSats: string;
  receiveAmountSats: string;
  feeSats: string;
  network:
    | "Bitcoin"
    | "Lightning"
    | "Liquid"
    | "Ecash"
    | "Exchange"
    | "Other";
  priceSource: NewTransactionPriceSource;
  fiatCurrency: "EUR";
  pricePerBtc: string;
  totalValue: string;
  movementId: string;
  label: string;
  taxTreatment: string;
  tags: string;
  note: string;
  reviewStatus: TransactionStatus;
  taxable: boolean;
  evidence: NewTransactionEvidence;
};

type NewTransactionPriceSource =
  | "manual"
  | "coingecko"
  | "exchange-csv"
  | "btcpay"
  | "wallet-import"
  | "missing";

type NewTransactionEvidence = {
  btcpayInvoiceId: string;
  exchangeCsvRow: string;
  swapId: string;
  txidOrPermalink: string;
  preimage: string;
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

const SATS_PER_BTC = 100_000_000;

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

function formatSignedDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return formatBtc(btc, { sign: true });
  const prefix = eur >= 0 ? "+ " : "− ";
  return `${prefix}${currencyFormatter.format(Math.abs(eur))}`;
}

function formatCounterDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return currencyFormatter.format(Math.abs(eur));
  return formatBtc(btc);
}

function formatShortTxid(txid: string) {
  if (txid.length <= 18) return txid;
  return `${txid.slice(0, 10)}...${txid.slice(-6)}`;
}

function localDatetimeInputValue(date = new Date()) {
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
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
  "layer-transition": "oklch(0.65 0.11 185)",
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
    amount: Math.abs(tx.eur || (tx.amountSat / SATS_PER_BTC) * tx.rate),
    amountBtc: Math.abs(tx.amountSat / SATS_PER_BTC),
    feeBtc: tx.feeSat ? Math.abs(tx.feeSat / SATS_PER_BTC) : 0,
    feeEur: tx.feeSat ? Math.abs((tx.feeSat / SATS_PER_BTC) * tx.rate) : 0,
    asset: "BTC",
    rate: tx.rate,
    note: tx.note,
    counterparty: tx.counter || tx.account || "Unassigned",
    counterpartyInitials: initials(tx.counter || tx.account || "TX"),
    direction,
    flow,
    wallet: tx.account || "Unassigned wallet",
    tag,
    sourceType: tx.type,
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

function normalizePeriodParam(value: string | null): PeriodKey | null {
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
  return null;
}

function initialPeriodFromUrl(): PeriodKey {
  if (typeof window === "undefined") return "1year";
  const params = new URLSearchParams(window.location.search);
  return normalizePeriodParam(params.get("period")) ?? "1year";
}

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
    if (flow === "transfer" || flow === "layer-transition") {
      row.transfers += value;
    }
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
}: {
  period: PeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const [swapDialogOpen, setSwapDialogOpen] = React.useState(false);
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
  const missingPriceCount = records.filter((txn) => !txn.rate).length;
  const chartRows = buildFlowChartRows(records, period, currency, swapCandidateIds);
  const activeChartRows = chartRows.filter((row) => flowPointTotal(row) > 0);
  const visibleChartRows = activeChartRows.length ? activeChartRows : chartRows;
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
    <>
      <section className="grid grid-cols-2 overflow-hidden rounded-xl border bg-card md:grid-cols-3 xl:grid-cols-6">
        {metricCards.map((metric, index) => {
          const Icon = metric.icon;
          const className = cn(
            "min-w-0 space-y-2 border-b p-3 text-left sm:p-4",
            index % 2 === 1 && "border-l",
            index % 3 === 0 ? "md:border-l-0" : "md:border-l",
            index > 0 ? "xl:border-l" : "xl:border-l-0",
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

        <div className="col-span-2 border-b p-3 sm:p-4 md:col-span-3 xl:col-span-4 xl:border-b-0">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">
                Flow by active {flowBucketLabel(period)}
              </h2>
              <p className="text-xs text-muted-foreground">
                {records.length} tx across {activeChartRows.length} active{" "}
                {activeChartRows.length === 1
                  ? flowBucketLabel(period)
                  : `${flowBucketLabel(period)}s`}
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
          <div className="h-[185px] min-w-0">
            <ChartContainer config={flowChartConfig} className="h-full w-full">
              <BarChart data={visibleChartRows}>
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

        <div className="col-span-2 grid gap-0 sm:grid-cols-2 md:col-span-3 xl:col-span-2 xl:grid-cols-1 xl:border-l">
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
            <h3 className="mb-2 text-sm font-semibold">Data quality</h3>
            <div className="divide-y text-xs">
              <QualityRow label="No explorer id" value={withoutExplorer} />
              <QualityRow label="Missing price" value={missingPriceCount} />
              <QualityRow label="Failed import" value={failedCount} />
              <QualityRow
                label="Swap candidates"
                value={swapCandidateTotals.count}
                onClick={
                  swapCandidateTotals.count > 0
                    ? () => setSwapDialogOpen(true)
                    : undefined
                }
              />
            </div>
          </div>
        </div>
      </section>

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

    </>
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

function flowPointEntries(row: FlowChartPoint) {
  return [
    ["incoming", row.incoming],
    ["outgoing", row.outgoing],
    ["transfer", row.transfers],
    ["swap", row.swaps],
  ] as const;
}

function flowPointTotal(row: FlowChartPoint) {
  return flowPointEntries(row).reduce((sum, [, value]) => sum + value, 0);
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
    <div className="border-t p-3 first:border-t-0 sm:p-4">
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
    "flex w-full items-center justify-between gap-3 py-2 text-left",
    onClick &&
      "cursor-pointer rounded-md px-1 transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
  );
  const content = (
    <>
      <span className="text-muted-foreground">{label}</span>
      <span className={cn("font-semibold tabular-nums", tone)}>
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
  transfer: "Internal transfer",
  swap: "Swap",
  "layer-transition": "Layer transition",
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
  "layer-transition":
    "border-teal-600/20 bg-teal-50 text-teal-700 dark:bg-teal-900/25 dark:text-teal-300",
};

const allTransactionFlows: TransactionFlow[] = [
  "incoming",
  "outgoing",
  "transfer",
  "swap",
  "layer-transition",
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

function explorerOpenErrorMessage(error: unknown) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "Could not open explorer in the default browser.";
}

function ExplorerOpenDialog({
  transaction,
  target,
  onTransactionChange,
}: {
  transaction: Transaction | null;
  target: ExplorerTarget | null;
  onTransactionChange: (transaction: Transaction | null) => void;
}) {
  const [openError, setOpenError] = React.useState<string | null>(null);
  const [opening, setOpening] = React.useState(false);

  React.useEffect(() => {
    if (!transaction) {
      setOpenError(null);
    }
  }, [transaction]);

  const openExplorer = async () => {
    if (!target) return;
    setOpenError(null);
    setOpening(true);
    try {
      await openExternalUrl(target.url);
      onTransactionChange(null);
    } catch (error) {
      setOpenError(explorerOpenErrorMessage(error));
    } finally {
      setOpening(false);
    }
  };

  return (
    <Dialog
      open={Boolean(transaction)}
      onOpenChange={(open) => {
        if (!open) {
          onTransactionChange(null);
        }
      }}
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <div className="mb-2 flex size-10 items-center justify-center rounded-full bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
            <ShieldAlert className="size-5" aria-hidden="true" />
          </div>
          <DialogTitle>Open transaction in a browser?</DialogTitle>
          <DialogDescription>
            This opens {target?.label ?? "a public explorer"} outside Kassiber.
            The explorer can see your IP address and the transaction id you
            request.
          </DialogDescription>
        </DialogHeader>
        {transaction && target ? (
          <div className="rounded-md border bg-muted/35 p-3 text-sm">
            <p className="font-medium">{transaction.txnId}</p>
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
              {target.url}
            </p>
          </div>
        ) : null}
        {openError ? (
          <p
            role="alert"
            className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            {openError}
          </p>
        ) : null}
        <DialogFooter>
          <DialogClose asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </DialogClose>
          <Button
            type="button"
            disabled={!target || opening}
            onClick={() => void openExplorer()}
          >
            <ExternalLink className="size-4" aria-hidden="true" />
            {opening ? "Opening..." : "Open explorer"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

const classificationOptions = [
  "Unlabeled",
  "Income",
  "Expense",
  "Transfer",
  "Swap",
  "Fee",
  "Merchant payment",
  "Gift",
  "Review",
  "Other",
];

const tagSuggestions = [
  "Revenue",
  "BTCPay",
  "needs invoice",
  "client ACME",
  "Hosting",
  "Capex",
  "Meals",
  "Bank fees",
  "Consolidation",
  "Liquid",
  "Lightning",
  "manual review",
  "accountant",
];

const taxTreatmentOptions = [
  "Unreviewed",
  "Taxable disposal",
  "Income receipt",
  "Expense / fee",
  "Non-taxable transfer",
  "Austrian carry basis",
];

const priceModeOptions: Array<{
  value: TransactionEditDraft["priceMode"];
  label: string;
}> = [
  { value: "imported", label: "Source price" },
  { value: "rate-cache", label: "Rate cache" },
  { value: "manual", label: "Manual override" },
  { value: "missing", label: "Missing / review" },
];

const priceModeStyles: Record<TransactionEditDraft["priceMode"], string> = {
  imported:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  "rate-cache":
    "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/30 dark:text-sky-400 dark:ring-sky-400/20",
  manual:
    "bg-violet-50 text-violet-700 ring-1 ring-inset ring-violet-600/20 dark:bg-violet-900/30 dark:text-violet-300 dark:ring-violet-400/20",
  missing:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
};

const newTransactionPriceSourceOptions: Array<{
  value: NewTransactionPriceSource;
  label: string;
  external: boolean;
}> = [
  { value: "manual", label: "Manual entry", external: false },
  { value: "coingecko", label: "CoinGecko daily rate", external: true },
  { value: "exchange-csv", label: "Exchange CSV row", external: true },
  { value: "btcpay", label: "BTCPay invoice fiat", external: true },
  { value: "wallet-import", label: "Wallet import exact fiat", external: true },
  { value: "missing", label: "Needs pricing", external: false },
];

const newTransactionPriceSourceStyles: Record<NewTransactionPriceSource, string> = {
  manual:
    "bg-zinc-100 text-zinc-700 ring-1 ring-inset ring-zinc-500/20 dark:bg-zinc-800 dark:text-zinc-300 dark:ring-zinc-400/20",
  coingecko:
    "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/30 dark:text-sky-400 dark:ring-sky-400/20",
  "exchange-csv":
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  btcpay:
    "bg-orange-50 text-orange-700 ring-1 ring-inset ring-orange-600/20 dark:bg-orange-900/25 dark:text-orange-300 dark:ring-orange-400/20",
  "wallet-import":
    "bg-indigo-50 text-indigo-700 ring-1 ring-inset ring-indigo-600/20 dark:bg-indigo-900/25 dark:text-indigo-300 dark:ring-indigo-400/20",
  missing:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
};

const austrianTaxTreatmentOptions: Array<{
  value: string;
  label: string;
  shortLabel: string;
  taxable: boolean;
}> = [
  {
    value: "section27b-taxable-disposal",
    label: "§27b taxable disposal",
    shortLabel: "Taxable disposal",
    taxable: true,
  },
  {
    value: "section27b-income",
    label: "§27b income receipt",
    shortLabel: "Income receipt",
    taxable: true,
  },
  {
    value: "section27b-expense-fee",
    label: "§27b expense / fee",
    shortLabel: "Expense / fee",
    taxable: true,
  },
  {
    value: "section27b-transfer",
    label: "§27b own-wallet transfer",
    shortLabel: "Own-wallet transfer",
    taxable: false,
  },
  {
    value: "section27b-layer-transition",
    label: "§27b layer transition / peg",
    shortLabel: "Layer transition",
    taxable: false,
  },
  {
    value: "section27b-carrying-value-swap",
    label: "§27b carrying-value swap",
    shortLabel: "Carrying-value swap",
    taxable: false,
  },
  {
    value: "outside-section27b",
    label: "Outside §27b / Altbestand",
    shortLabel: "Outside §27b",
    taxable: false,
  },
];

const newTransactionNetworkOptions: NewTransactionDraft["network"][] = [
  "Bitcoin",
  "Lightning",
  "Liquid",
  "Ecash",
  "Exchange",
  "Other",
];

function sourceKindForNetwork(
  network: NewTransactionDraft["network"],
): NewTransactionDraft["sourceKind"] {
  if (network === "Bitcoin") return "onchain";
  if (network === "Exchange") return "exchange";
  if (network === "Other") return "manual";
  return "offchain";
}

const mockNewTransactionWalletSourceOptions = [
  ...MOCK_OVERVIEW.connections.map((connection) => connection.label),
  "External",
];

const newTransactionFlowOptions: Array<{
  value: TransactionFlow;
  label: string;
}> = [
  { value: "incoming", label: "Receive" },
  { value: "outgoing", label: "Send" },
  { value: "transfer", label: "Internal transfer" },
  { value: "swap", label: "Swap" },
  { value: "layer-transition", label: "Layer transition" },
];

const mockNewTransactionMovementCandidates = [
  {
    id: "movement-ln-channel-open",
    label: "Join channel open · Home Node",
    detail: "tx8 · same wallet · 96% confidence",
  },
  {
    id: "movement-liquid-peg",
    label: "Join Liquid peg transition",
    detail: "LBTC leg 8 min apart · 89% confidence",
  },
  {
    id: "movement-boltz-swap",
    label: "Join Boltz swap pair",
    detail: "payment hash overlap · 82% confidence",
  },
];

function draftForTransaction(txn: Transaction): TransactionEditDraft {
  const flow = transactionFlow(txn);
  const initialTags = splitDraftTags(txn.tag || "");
  const defaultTaxTreatment =
    flow === "transfer"
      ? "Non-taxable transfer"
      : flow === "incoming"
        ? "Income receipt"
        : flow === "swap"
          ? "Austrian carry basis"
          : "Taxable disposal";
  return {
    label:
      classificationOptions.find((option) => initialTags.includes(option)) ??
      (flow === "incoming"
        ? "Income"
        : flow === "outgoing"
          ? "Expense"
          : flow === "swap"
            ? "Swap"
            : flow === "transfer"
              ? "Transfer"
              : "Unlabeled"),
    tags: uniqueTags(
      initialTags.filter((tag) => !classificationOptions.includes(tag)),
    ),
    note: txn.note || "",
    taxTreatment: defaultTaxTreatment,
    priceMode: txn.rate ? "imported" : "missing",
    manualCurrency: "EUR",
    manualPrice: txn.rate ? String(txn.rate) : "",
    manualValue: txn.amount ? String(txn.amount) : "",
    manualSource: "",
    reviewStatus: txn.status,
    taxable: flow !== "transfer",
    excluded: false,
  };
}

function splitDraftTags(tags: string) {
  return tags
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

function uniqueTags(tags: string[]) {
  return Array.from(new Set(tags.map((tag) => tag.trim()).filter(Boolean)));
}

function priceModeLabel(mode: TransactionEditDraft["priceMode"]) {
  return priceModeOptions.find((option) => option.value === mode)?.label ?? mode;
}

function newTransactionPriceSourceLabel(source: NewTransactionPriceSource) {
  return (
    newTransactionPriceSourceOptions.find((option) => option.value === source)
      ?.label ?? source
  );
}

function isExternalPriceSource(source: NewTransactionPriceSource) {
  return Boolean(
    newTransactionPriceSourceOptions.find((option) => option.value === source)
      ?.external,
  );
}

function austrianTaxTreatmentFor(value: string) {
  return (
    austrianTaxTreatmentOptions.find((option) => option.value === value) ??
    austrianTaxTreatmentOptions[0]
  );
}

function parseManualDecimal(value: string) {
  const normalized = value.trim().replace(",", ".");
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : null;
}

function formatManualFiat(value: number) {
  if (!Number.isFinite(value)) return "";
  return value.toFixed(2);
}

function formatDraftFiat(value: number, currencyCode: NewTransactionDraft["fiatCurrency"]) {
  if (!Number.isFinite(value)) return "-";
  return `${currencyFormatter.format(value)} ${currencyCode}`;
}

function formatManualPrice(value: number) {
  if (!Number.isFinite(value)) return "";
  return value.toFixed(2);
}

function formatBtcAmount(value: number, precision = 8) {
  return `${value.toFixed(precision)} BTC`;
}

function formatAssetAmount(value: number, asset: string, precision = 8) {
  return `${value.toFixed(precision)} ${asset || "BTC"}`;
}

function formatFee(txn: Transaction, currency: Currency) {
  const feeBtc = txn.feeBtc ?? 0;
  if (!feeBtc) return "-";
  if (currency === "btc") return formatBtcAmount(feeBtc);
  return currencyFormatter.format(txn.feeEur ?? 0);
}

function createNewTransactionDraft(): NewTransactionDraft {
  return {
    sourceKind: "onchain",
    flow: "incoming",
    occurredAt: localDatetimeInputValue(),
    confirmedAt: "",
    wallet: "Cold Storage",
    counterparty: "",
    transactionId: "",
    fromWallet: "Cold Storage",
    toWallet: "Cold Storage",
    fromExternal: "",
    toExternal: "",
    swapService: "",
    asset: "BTC",
    amountSats: "",
    sendAsset: "BTC",
    receiveAsset: "BTC",
    sendAmountSats: "",
    receiveAmountSats: "",
    feeSats: "",
    network: "Bitcoin",
    priceSource: "manual",
    fiatCurrency: "EUR",
    pricePerBtc: "",
    totalValue: "",
    movementId: "",
    label: "Income",
    taxTreatment: "section27b-income",
    tags: "",
    note: "",
    reviewStatus: "review",
    taxable: true,
    evidence: {
      btcpayInvoiceId: "",
      exchangeCsvRow: "",
      swapId: "",
      txidOrPermalink: "",
      preimage: "",
    },
  };
}

function parseSatsInput(value: string) {
  const normalized = value.trim().replace(/[,_\s]/g, "");
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? Math.trunc(Math.abs(parsed)) : null;
}

function btcFromSatsInput(value: string) {
  const sats = parseSatsInput(value);
  return sats === null ? null : sats / SATS_PER_BTC;
}

function nextTaxTreatmentForFlow(flow: TransactionFlow) {
  if (flow === "incoming") return "section27b-income";
  if (flow === "transfer") return "section27b-transfer";
  if (flow === "swap") return "section27b-carrying-value-swap";
  if (flow === "layer-transition") return "section27b-layer-transition";
  return "section27b-taxable-disposal";
}

function nextLabelForFlow(flow: TransactionFlow) {
  if (flow === "incoming") return "Income";
  if (flow === "transfer") return "Transfer";
  if (flow === "swap") return "Swap";
  if (flow === "layer-transition") return "Transfer";
  return "Expense";
}

function isTwoLegNewTransactionFlow(flow: TransactionFlow) {
  return flow === "swap" || flow === "layer-transition";
}

function showConfirmedAtForDraft(draft: NewTransactionDraft) {
  return draft.network === "Bitcoin" || draft.network === "Liquid";
}

function showSingleAssetForDraft(draft: NewTransactionDraft) {
  return (
    !isTwoLegNewTransactionFlow(draft.flow) &&
    (draft.network === "Exchange" || draft.network === "Other")
  );
}

function inferredAssetForDraft(draft: NewTransactionDraft) {
  if (showSingleAssetForDraft(draft)) return draft.asset || "BTC";
  if (draft.network === "Liquid") return "LBTC";
  return "BTC";
}

function pricingAmountSatsForDraft(draft: NewTransactionDraft) {
  if (isTwoLegNewTransactionFlow(draft.flow)) {
    return draft.receiveAmountSats || draft.sendAmountSats || draft.amountSats;
  }
  return draft.amountSats;
}

function calculateNewTransactionPricing(
  draft: NewTransactionDraft,
  changed: "amountSats" | "pricePerBtc" | "totalValue",
): NewTransactionDraft {
  const btc = btcFromSatsInput(pricingAmountSatsForDraft(draft));
  const price = parseManualDecimal(draft.pricePerBtc);
  const total = parseManualDecimal(draft.totalValue);

  if (!btc || btc <= 0) return draft;

  if ((changed === "amountSats" || changed === "pricePerBtc") && price !== null) {
    return {
      ...draft,
      totalValue: formatManualFiat(btc * price),
    };
  }
  if ((changed === "amountSats" || changed === "totalValue") && total !== null) {
    return {
      ...draft,
      pricePerBtc: formatManualPrice(total / btc),
    };
  }
  return draft;
}

function signedNewTransactionBtc(draft: NewTransactionDraft) {
  if (isTwoLegNewTransactionFlow(draft.flow)) {
    const received = btcFromSatsInput(draft.receiveAmountSats) ?? 0;
    const sent = btcFromSatsInput(draft.sendAmountSats) ?? 0;
    return received - sent;
  }
  const btc = btcFromSatsInput(draft.amountSats) ?? 0;
  if (draft.flow === "outgoing") return -btc;
  if (draft.flow === "transfer") return 0;
  return btc;
}

function copyText(value: string | undefined) {
  if (!value || typeof navigator === "undefined") return;
  navigator.clipboard?.writeText(value);
}

function NewTransactionDialog({
  open,
  draft,
  walletSourceOptions,
  onOpenChange,
  onDraftChange,
  onSaveDraft,
}: {
  open: boolean;
  draft: NewTransactionDraft;
  walletSourceOptions: string[];
  onOpenChange: (open: boolean) => void;
  onDraftChange: (draft: NewTransactionDraft) => void;
  onSaveDraft: () => void;
}) {
  const bodyRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (open) bodyRef.current?.scrollTo({ top: 0 });
  }, [open]);
  const updateDraft = React.useCallback(
    (patch: Partial<NewTransactionDraft>) => {
      onDraftChange({ ...draft, ...patch });
    },
    [draft, onDraftChange],
  );
  const updateEvidence = React.useCallback(
    (patch: Partial<NewTransactionEvidence>) => {
      onDraftChange({
        ...draft,
        evidence: { ...draft.evidence, ...patch },
      });
    },
    [draft, onDraftChange],
  );
  const updateFlow = React.useCallback(
    (flow: TransactionFlow) => {
      const taxTreatment = nextTaxTreatmentForFlow(flow);
      const fallbackWallet =
        draft.wallet && draft.wallet !== "External" ? draft.wallet : "Cold Storage";
      const fromWallet =
        flow === "incoming"
          ? draft.fromWallet
          : draft.fromWallet === "External"
            ? fallbackWallet
            : draft.fromWallet || fallbackWallet;
      onDraftChange({
        ...draft,
        flow,
        fromWallet,
        toWallet: draft.toWallet === "External" ? fallbackWallet : draft.toWallet,
        label: nextLabelForFlow(flow),
        taxTreatment,
        taxable: austrianTaxTreatmentFor(taxTreatment).taxable,
      });
    },
    [draft, onDraftChange],
  );
  const updatePricingField = React.useCallback(
    (
      field: "amountSats" | "pricePerBtc" | "totalValue",
      value: string,
    ) => {
      onDraftChange(calculateNewTransactionPricing({ ...draft, [field]: value }, field));
    },
    [draft, onDraftChange],
  );
  const twoLegFlow = isTwoLegNewTransactionFlow(draft.flow);
  const showConfirmedAt = showConfirmedAtForDraft(draft);
  const showSingleAsset = showSingleAssetForDraft(draft);
  const ownWalletOptions = walletSourceOptions.filter((wallet) => wallet !== "External");
  const singleLegBtc = btcFromSatsInput(draft.amountSats) ?? 0;
  const sendLegBtc = btcFromSatsInput(draft.sendAmountSats) ?? 0;
  const receiveLegBtc = btcFromSatsInput(draft.receiveAmountSats) ?? 0;
  const movementBtc = twoLegFlow
    ? Math.max(sendLegBtc, receiveLegBtc)
    : singleLegBtc;
  const feeBtc = btcFromSatsInput(draft.feeSats) ?? 0;
  const signedBtc = signedNewTransactionBtc(draft);
  const totalValue = parseManualDecimal(draft.totalValue);
  const priceValue = parseManualDecimal(draft.pricePerBtc);
  const tags = uniqueTags(splitDraftTags(draft.tags));
  const taxTreatment = austrianTaxTreatmentFor(draft.taxTreatment);
  const selectedMovement = mockNewTransactionMovementCandidates.find(
    (candidate) => candidate.id === draft.movementId,
  );
  const movementLabel =
    draft.movementId === "new"
      ? "New movement"
      : selectedMovement?.label || "Standalone";
  const fromDisplay =
    draft.flow === "incoming"
      ? draft.fromExternal || "External"
      : draft.fromWallet || "Unassigned";
  const toDisplay =
    draft.flow === "outgoing"
      ? draft.toExternal || "External"
      : draft.toWallet || "Unassigned";
  const primaryEvidence =
    draft.evidence.txidOrPermalink ||
    draft.evidence.btcpayInvoiceId ||
    draft.evidence.swapId ||
    draft.evidence.exchangeCsvRow ||
    draft.evidence.preimage;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger asChild>
        <Button size="sm" className="h-8 gap-2 sm:h-9" aria-label="New transaction">
          <Plus className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">New transaction</span>
        </Button>
      </DialogTrigger>
      <DialogContent className="flex max-h-[calc(100vh-1rem)] flex-col overflow-hidden p-0 sm:max-w-[80rem]">
        <DialogHeader className="shrink-0 px-5 pt-4 pb-2 pr-12">
          <DialogTitle>New transaction</DialogTitle>
          <DialogDescription>Manual draft</DialogDescription>
        </DialogHeader>

        <div ref={bodyRef} className="min-h-0 flex-1 overflow-y-auto px-5 pb-3">
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_300px]">
          <div className="space-y-3">
            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">Network and timing</h3>
                <Badge variant="outline">{draft.network}</Badge>
              </div>
              <div className="grid gap-2 md:grid-cols-2">
                <div className="space-y-1.5 md:col-span-2">
                  <Label>Network</Label>
                  <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-1 sm:grid-cols-3 xl:grid-cols-6">
                    {newTransactionNetworkOptions.map((network) => (
                      <button
                        key={network}
                        type="button"
                        aria-pressed={draft.network === network}
                        className={cn(
                          "min-w-0 truncate rounded-md px-2.5 py-1.5 text-center text-xs font-medium text-muted-foreground transition-colors hover:text-foreground",
                          draft.network === network && "bg-card text-foreground shadow-sm",
                        )}
                        onClick={() =>
                          updateDraft({
                            network,
                            sourceKind: sourceKindForNetwork(network),
                            asset:
                              network === "Liquid" && draft.asset === "BTC"
                                ? "LBTC"
                                : draft.asset,
                            receiveAsset:
                              network === "Liquid" && draft.receiveAsset === "BTC"
                                ? "LBTC"
                                : draft.receiveAsset,
                          })
                        }
                      >
                        {network}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5 md:col-span-2">
                  <Label>Flow</Label>
                  <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-1 sm:grid-cols-3 xl:grid-cols-5">
                    {newTransactionFlowOptions.map((option) => (
                      <button
                        key={option.value}
                        type="button"
                        aria-pressed={draft.flow === option.value}
                        className={cn(
                          "min-w-0 truncate rounded-md px-2.5 py-1.5 text-center text-xs font-medium text-muted-foreground transition-colors hover:text-foreground",
                          draft.flow === option.value &&
                            "bg-card text-foreground shadow-sm",
                        )}
                        onClick={() => updateFlow(option.value)}
                      >
                        {option.label}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-occurred-at">Occurred at</Label>
                  <Input
                    id="new-txn-occurred-at"
                    type="datetime-local"
                    value={draft.occurredAt}
                    onChange={(event) => updateDraft({ occurredAt: event.target.value })}
                  />
                </div>
                {showConfirmedAt ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-confirmed-at">Confirmed at</Label>
                    <Input
                      id="new-txn-confirmed-at"
                      type="datetime-local"
                      value={draft.confirmedAt}
                      onChange={(event) =>
                        updateDraft({ confirmedAt: event.target.value })
                      }
                    />
                  </div>
                ) : null}
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">Parties and route</h3>
              {twoLegFlow ? (
                <div className="grid gap-2 md:grid-cols-3">
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-from-wallet">From</Label>
                    <Select
                      value={draft.fromWallet}
                      onValueChange={(value) =>
                        updateDraft({ fromWallet: value, wallet: value })
                      }
                    >
                      <SelectTrigger id="new-txn-from-wallet" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {ownWalletOptions.map((wallet) => (
                          <SelectItem key={wallet} value={wallet}>
                            {wallet}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-to-wallet">To</Label>
                    <Select
                      value={draft.toWallet}
                      onValueChange={(value) => updateDraft({ toWallet: value })}
                    >
                      <SelectTrigger id="new-txn-to-wallet" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {ownWalletOptions.map((wallet) => (
                          <SelectItem key={wallet} value={wallet}>
                            {wallet}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-swap-service">Swap service</Label>
                    <Input
                      id="new-txn-swap-service"
                      value={draft.swapService}
                      onChange={(event) =>
                        updateDraft({ swapService: event.target.value })
                      }
                      placeholder="Boltz, exchange, channel peer"
                    />
                  </div>
                </div>
              ) : (
                <div className="grid gap-2 md:grid-cols-2">
                  {draft.flow === "incoming" ? (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-external">From</Label>
                        <Input
                          id="new-txn-from-external"
                          value={draft.fromExternal}
                          onChange={(event) =>
                            updateDraft({
                              fromExternal: event.target.value,
                              counterparty: event.target.value,
                            })
                          }
                          placeholder="External party, payer, or source"
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-wallet">To</Label>
                        <Select
                          value={draft.toWallet}
                          onValueChange={(value) =>
                            updateDraft({ toWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-to-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    </>
                  ) : draft.flow === "outgoing" ? (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-wallet">From</Label>
                        <Select
                          value={draft.fromWallet}
                          onValueChange={(value) =>
                            updateDraft({ fromWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-from-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-external">To</Label>
                        <Input
                          id="new-txn-to-external"
                          value={draft.toExternal}
                          onChange={(event) =>
                            updateDraft({
                              toExternal: event.target.value,
                              counterparty: event.target.value,
                            })
                          }
                          placeholder="External party or destination"
                        />
                      </div>
                    </>
                  ) : (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-wallet">From</Label>
                        <Select
                          value={draft.fromWallet}
                          onValueChange={(value) =>
                            updateDraft({ fromWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-from-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-wallet">To</Label>
                        <Select
                          value={draft.toWallet}
                          onValueChange={(value) => updateDraft({ toWallet: value })}
                        >
                          <SelectTrigger id="new-txn-to-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    </>
                  )}
                </div>
              )}
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">Amount and pricing</h3>
              {twoLegFlow ? (
                <div className="grid gap-2 md:grid-cols-2">
                  <div className="rounded-md border bg-background p-2">
                    <h4 className="mb-2 text-xs font-semibold text-muted-foreground">
                      Leg 1 out
                    </h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-send-amount">Send sats</Label>
                        <Input
                          id="new-txn-send-amount"
                          inputMode="numeric"
                          value={draft.sendAmountSats}
                          onChange={(event) =>
                            updateDraft({ sendAmountSats: event.target.value })
                          }
                          placeholder="2450000"
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-send-asset">Send asset</Label>
                        <Input
                          id="new-txn-send-asset"
                          value={draft.sendAsset}
                          onChange={(event) =>
                            updateDraft({ sendAsset: event.target.value })
                          }
                          placeholder="BTC"
                        />
                      </div>
                    </div>
                  </div>
                  <div className="rounded-md border bg-background p-2">
                    <h4 className="mb-2 text-xs font-semibold text-muted-foreground">
                      Leg 2 in
                    </h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-receive-amount">Receive sats</Label>
                        <Input
                          id="new-txn-receive-amount"
                          inputMode="numeric"
                          value={draft.receiveAmountSats}
                          onChange={(event) =>
                            updateDraft({ receiveAmountSats: event.target.value })
                          }
                          placeholder="2450000"
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-receive-asset">Receive asset</Label>
                        <Input
                          id="new-txn-receive-asset"
                          value={draft.receiveAsset}
                          onChange={(event) =>
                            updateDraft({ receiveAsset: event.target.value })
                          }
                          placeholder={draft.network === "Liquid" ? "LBTC" : "BTC"}
                        />
                      </div>
                    </div>
                  </div>
                </div>
              ) : null}

              <div className="mt-2 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                {!twoLegFlow ? (
                  <>
                    <div className="grid gap-1.5">
                      <Label htmlFor="new-txn-amount">Amount sats</Label>
                      <Input
                        id="new-txn-amount"
                        inputMode="numeric"
                        value={draft.amountSats}
                        onChange={(event) =>
                          updatePricingField("amountSats", event.target.value)
                        }
                        placeholder="2450000"
                      />
                    </div>
                    {showSingleAsset ? (
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-asset">Asset</Label>
                        <Input
                          id="new-txn-asset"
                          value={draft.asset}
                          onChange={(event) =>
                            updateDraft({ asset: event.target.value })
                          }
                        />
                      </div>
                    ) : null}
                  </>
                ) : null}
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-fee">Fee sats</Label>
                  <Input
                    id="new-txn-fee"
                    inputMode="numeric"
                    value={draft.feeSats}
                    onChange={(event) => updateDraft({ feeSats: event.target.value })}
                    placeholder="0"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-price">
                    Price / BTC ({draft.fiatCurrency})
                  </Label>
                  <Input
                    id="new-txn-price"
                    inputMode="decimal"
                    value={draft.pricePerBtc}
                    onChange={(event) =>
                      updatePricingField("pricePerBtc", event.target.value)
                    }
                    placeholder="71420.18"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-value">
                    Total value ({draft.fiatCurrency})
                  </Label>
                  <Input
                    id="new-txn-value"
                    inputMode="decimal"
                    value={draft.totalValue}
                    onChange={(event) =>
                      updatePricingField("totalValue", event.target.value)
                    }
                    placeholder="1749.79"
                  />
                </div>
                <div className="grid gap-1.5 md:col-span-2 xl:col-span-1">
                  <Label>Pricing method</Label>
                  <Select
                    value={draft.priceSource}
                    onValueChange={(value) =>
                      updateDraft({ priceSource: value as NewTransactionPriceSource })
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {newTransactionPriceSourceOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">Part of movement</h3>
                <Button
                  type="button"
                  variant={draft.movementId === "new" ? "default" : "outline"}
                  size="sm"
                  className="h-7"
                  onClick={() => updateDraft({ movementId: "new" })}
                >
                  New movement
                </Button>
              </div>
              <div className="grid gap-2">
                <Input
                  value={
                    selectedMovement?.label ??
                    (draft.movementId === "new" ? "" : draft.movementId)
                  }
                  onChange={(event) =>
                    updateDraft({ movementId: event.target.value })
                  }
                  placeholder="Search movement, swap, channel, or peg"
                />
                <div className="grid gap-1 sm:grid-cols-3">
                  {mockNewTransactionMovementCandidates.map((candidate) => (
                    <button
                      key={candidate.id}
                      type="button"
                      className={cn(
                        "min-w-0 rounded-md border p-1.5 text-left text-[11px] transition-colors hover:bg-muted/40",
                        draft.movementId === candidate.id && "bg-muted",
                      )}
                      onClick={() => updateDraft({ movementId: candidate.id })}
                    >
                      <span className="block truncate font-medium">
                        {candidate.label}
                      </span>
                      <span className="block truncate text-muted-foreground">
                        {candidate.detail}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">Classification</h3>
                <Badge variant={taxTreatment.taxable ? "default" : "outline"}>
                  {taxTreatment.taxable ? "Taxable" : "Not taxable"}
                </Badge>
              </div>
              <div className="grid gap-2 md:grid-cols-[minmax(150px,0.8fr)_minmax(220px,1.2fr)]">
                <div className="grid gap-1.5">
                  <Label>Label</Label>
                  <Select
                    value={draft.label}
                    onValueChange={(value) => updateDraft({ label: value })}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {classificationOptions.map((option) => (
                        <SelectItem key={option} value={option}>
                          {option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1.5">
                  <Label>Tax treatment</Label>
                  <Select
                    value={draft.taxTreatment}
                    onValueChange={(value) =>
                      updateDraft({
                        taxTreatment: value,
                        taxable: austrianTaxTreatmentFor(value).taxable,
                      })
                    }
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {austrianTaxTreatmentOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-tags">Tags</Label>
                  <Input
                    id="new-txn-tags"
                    value={draft.tags}
                    onChange={(event) => updateDraft({ tags: event.target.value })}
                    placeholder="Revenue, BTCPay, client ACME"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-note">Note</Label>
                  <Textarea
                    id="new-txn-note"
                    value={draft.note}
                    onChange={(event) => updateDraft({ note: event.target.value })}
                    placeholder="Freeform review commentary"
                  />
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">Evidence</h3>
              <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-txid">Txid or permalink</Label>
                  <Input
                    id="new-txn-evidence-txid"
                    value={draft.evidence.txidOrPermalink}
                    onChange={(event) =>
                      updateEvidence({ txidOrPermalink: event.target.value })
                    }
                    placeholder="txid, payment hash, or URL"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-btcpay">BTCPay invoice ID</Label>
                  <Input
                    id="new-txn-evidence-btcpay"
                    value={draft.evidence.btcpayInvoiceId}
                    onChange={(event) =>
                      updateEvidence({ btcpayInvoiceId: event.target.value })
                    }
                    placeholder="invoice id"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-exchange">Exchange CSV row</Label>
                  <Input
                    id="new-txn-evidence-exchange"
                    value={draft.evidence.exchangeCsvRow}
                    onChange={(event) =>
                      updateEvidence({ exchangeCsvRow: event.target.value })
                    }
                    placeholder="file.csv:42"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-swap">Boltz swap ID</Label>
                  <Input
                    id="new-txn-evidence-swap"
                    value={draft.evidence.swapId}
                    onChange={(event) => updateEvidence({ swapId: event.target.value })}
                    placeholder="swap id"
                  />
                </div>
                <div className="grid gap-1.5 md:col-span-2">
                  <Label htmlFor="new-txn-evidence-preimage">Preimage</Label>
                  <Input
                    id="new-txn-evidence-preimage"
                    value={draft.evidence.preimage}
                    onChange={(event) =>
                      updateEvidence({ preimage: event.target.value })
                    }
                    placeholder="payment preimage"
                  />
                </div>
              </div>
            </section>
          </div>

          <aside className="space-y-2.5 rounded-lg border bg-muted/20 p-2.5 lg:sticky lg:top-0 lg:self-start">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-xs text-muted-foreground">Live preview</p>
                <p className="truncate text-lg font-semibold">
                  {transactionFlowLabels[draft.flow]}
                </p>
                <p className="truncate text-xs text-muted-foreground">
                  {fromDisplay} → {toDisplay}
                </p>
              </div>
              {isExternalPriceSource(draft.priceSource) ? (
                <Badge className={cn(newTransactionPriceSourceStyles[draft.priceSource])}>
                  {newTransactionPriceSourceLabel(draft.priceSource)}
                </Badge>
              ) : null}
            </div>

            <div className="rounded-md border bg-background p-3">
              {twoLegFlow ? (
                <div className="grid gap-2 text-sm">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">Out</span>
                    <span className="truncate text-right font-semibold">
                      {formatAssetAmount(sendLegBtc, draft.sendAsset)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">In</span>
                    <span className="truncate text-right font-semibold">
                      {formatAssetAmount(
                        receiveLegBtc,
                        draft.receiveAsset || inferredAssetForDraft(draft),
                      )}
                    </span>
                  </div>
                </div>
              ) : (
                <>
                  <p
                    className={cn(
                      "text-xl font-semibold",
                      draft.flow === "incoming"
                        ? "text-emerald-500"
                        : draft.flow === "outgoing"
                          ? "text-rose-400"
                          : "text-foreground",
                    )}
                  >
                    {draft.flow === "outgoing"
                      ? "− "
                      : draft.flow === "incoming"
                        ? "+ "
                        : ""}
                    {formatBtcAmount(Math.abs(movementBtc))}
                  </p>
                  <p className="mt-1 text-sm text-muted-foreground">
                    Net {formatBtc(signedBtc, { sign: true })}
                  </p>
                </>
              )}
              <p className="mt-2 text-sm text-muted-foreground">
                {totalValue !== null
                  ? formatDraftFiat(totalValue, draft.fiatCurrency)
                  : `No ${draft.fiatCurrency} value`}
              </p>
            </div>

            <div className="grid gap-2 text-sm">
              <PreviewRow label="Network" value={draft.network} />
              <PreviewRow label="From" value={fromDisplay} />
              <PreviewRow label="To" value={toDisplay} />
              {draft.swapService ? (
                <PreviewRow label="Service" value={draft.swapService} />
              ) : null}
              <PreviewRow
                label="Movement"
                value={
                  <button
                    type="button"
                    className="truncate underline-offset-4 hover:underline"
                    onClick={() =>
                      updateDraft({
                        movementId: draft.movementId ? "" : "new",
                      })
                    }
                  >
                    {movementLabel}
                  </button>
                }
              />
              <PreviewRow
                label="Asset"
                value={
                  twoLegFlow
                    ? `${draft.sendAsset || "BTC"} → ${
                        draft.receiveAsset || inferredAssetForDraft(draft)
                      }`
                    : inferredAssetForDraft(draft)
                }
              />
              <PreviewRow
                label="Fee"
                value={feeBtc ? formatBtcAmount(feeBtc) : "-"}
              />
              <PreviewRow
                label={`Value (${draft.fiatCurrency})`}
                value={
                  totalValue !== null
                    ? formatDraftFiat(totalValue, draft.fiatCurrency)
                    : "-"
                }
              />
              <PreviewRow
                label={`Price (${draft.fiatCurrency}/BTC)`}
                value={
                  priceValue !== null
                    ? `${formatDraftFiat(priceValue, draft.fiatCurrency)} / BTC`
                    : "-"
                }
              />
              <PreviewRow
                label="Pricing"
                value={newTransactionPriceSourceLabel(draft.priceSource)}
              />
              <PreviewRow label="Tax" value={taxTreatment.shortLabel} />
              {primaryEvidence ? (
                <PreviewRow label="Evidence" value={primaryEvidence} />
              ) : null}
            </div>

            <div className="grid gap-1.5">
              <Label>Status</Label>
              <Select
                value={draft.reviewStatus}
                onValueChange={(value) =>
                  updateDraft({ reviewStatus: value as TransactionStatus })
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {allTransactionStatuses.map((status) => (
                    <SelectItem key={status} value={status}>
                      {transactionStatusLabels[status]}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="flex flex-wrap gap-1">
              <Badge variant="secondary">{draft.label}</Badge>
              <Badge variant={taxTreatment.taxable ? "default" : "outline"}>
                {taxTreatment.taxable ? "Taxable" : "Not taxable"}
              </Badge>
              {tags.map((tag) => (
                <Badge key={tag} variant="outline">
                  {tag}
                </Badge>
              ))}
            </div>
          </aside>
          </div>
        </div>

        <DialogFooter className="shrink-0 border-t bg-background/95 px-5 py-2.5 backdrop-blur">
          <DialogClose asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </DialogClose>
          <Button type="button" variant="secondary" onClick={onSaveDraft}>
            Save draft
          </Button>
          <Button type="button" onClick={onSaveDraft}>
            Save and review
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function PreviewRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b pb-2 last:border-b-0 last:pb-0">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate text-right font-medium">{value}</span>
    </div>
  );
}

function DetailField({
  label,
  value,
  copyValue,
  hidden,
}: {
  label: string;
  value: React.ReactNode;
  copyValue?: string;
  hidden?: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md border bg-background p-3">
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="text-[10px] font-medium uppercase text-muted-foreground">
          {label}
        </span>
        {copyValue ? (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-6 text-muted-foreground"
            aria-label={`Copy ${label}`}
            onClick={() => copyText(copyValue)}
          >
            <Copy className="size-3.5" aria-hidden="true" />
          </Button>
        ) : null}
      </div>
      <div
        className={cn(
          "min-w-0 truncate text-sm font-medium",
          hidden && "sensitive",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function LedgerRow({
  label,
  value,
  align = "left",
  muted,
}: {
  label: string;
  value: React.ReactNode;
  align?: "left" | "right";
  muted?: boolean;
}) {
  return (
    <div
      className={cn(
        "grid min-h-10 grid-cols-[minmax(140px,0.9fr)_minmax(0,1.1fr)] items-center gap-3 border-b px-3 py-2 last:border-b-0",
        muted && "bg-muted/35",
      )}
    >
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn(
          "min-w-0 text-sm font-medium",
          align === "right" && "text-right tabular-nums",
        )}
      >
        {value}
      </div>
    </div>
  );
}

function TransactionDetailSheet({
  transaction,
  draft,
  initialTab,
  hideSensitive,
  currency,
  explorerSettings,
  onOpenChange,
  onOpenExplorer,
  onSave,
}: {
  transaction: Transaction | null;
  draft: TransactionEditDraft | null;
  initialTab: string;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  onOpenChange: (open: boolean) => void;
  onOpenExplorer: (transaction: Transaction) => void;
  onSave: (transactionId: string, draft: TransactionEditDraft) => void;
}) {
  const [activeTab, setActiveTab] = React.useState(initialTab);
  const [localDraft, setLocalDraft] = React.useState<TransactionEditDraft | null>(
    draft,
  );
  const [tagInput, setTagInput] = React.useState("");

  React.useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab, transaction?.id]);

  React.useEffect(() => {
    setLocalDraft(draft);
    setTagInput("");
  }, [draft, transaction?.id]);

  if (!transaction || !localDraft) return null;

  const StatusIcon = transactionStatusIcons[localDraft.reviewStatus];
  const flow = transactionFlow(transaction);
  const explorer = explorerForTransaction(transaction, explorerSettings);
  const amountBtc = transactionBtc(transaction);
  const signedPrefix =
    flow === "incoming" ? "+" : flow === "outgoing" ? "-" : "";
  const tags = localDraft.tags;

  const updateDraft = <K extends keyof TransactionEditDraft>(
    key: K,
    value: TransactionEditDraft[K],
  ) => {
    setLocalDraft((current) =>
      current ? { ...current, [key]: value } : current,
    );
  };
  const addTag = (rawTag: string) => {
    const tag = rawTag.trim();
    if (!tag) return;
    updateDraft("tags", uniqueTags([...localDraft.tags, tag]));
    setTagInput("");
  };
  const removeTag = (tag: string) => {
    updateDraft(
      "tags",
      localDraft.tags.filter((candidate) => candidate !== tag),
    );
  };
  const availableTagSuggestions = tagSuggestions.filter(
    (suggestion) => !localDraft.tags.includes(suggestion),
  );
  const updateManualPrice = (rawPrice: string) => {
    const parsedPrice = parseManualDecimal(rawPrice);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            priceMode: "manual",
            manualPrice: rawPrice,
            manualValue:
              parsedPrice !== null && amountBtc > 0
                ? formatManualFiat(parsedPrice * amountBtc)
                : "",
          }
        : current,
    );
  };
  const updateManualValue = (rawValue: string) => {
    const parsedValue = parseManualDecimal(rawValue);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            priceMode: "manual",
            manualValue: rawValue,
            manualPrice:
              parsedValue !== null && amountBtc > 0
                ? formatManualPrice(parsedValue / amountBtc)
                : "",
          }
        : current,
    );
  };

  return (
    <Sheet open={Boolean(transaction)} onOpenChange={onOpenChange}>
      <SheetContent
        className="w-[min(100vw,1120px)] overflow-hidden p-0 sm:max-w-none"
        showCloseButton={false}
      >
        <SheetHeader className="border-b p-0">
          <div className="flex items-start justify-between gap-4 px-4 py-4 sm:px-6">
            <div className="min-w-0">
              <div className="mb-2 flex flex-wrap items-center gap-2">
                <Badge variant="outline" className="gap-1 rounded-md">
                  <Bitcoin className="size-3 text-amber-500" aria-hidden="true" />
                  {transaction.asset ?? "BTC"}
                </Badge>
                <Badge
                  variant="outline"
                  className={cn("rounded-md", transactionFlowStyles[flow])}
                >
                  {transactionFlowLabels[flow]}
                </Badge>
                <Badge
                  variant="outline"
                  className={cn(
                    "gap-1 rounded-md",
                    transactionStatusStyles[localDraft.reviewStatus],
                  )}
                >
                  <StatusIcon className="size-3" aria-hidden="true" />
                  {transactionStatusLabels[localDraft.reviewStatus]}
                </Badge>
              </div>
              <SheetTitle className="truncate text-xl sm:text-2xl">
                {signedPrefix}
                <span className={blurClass(hideSensitive)}>
                  {formatBtcAmount(amountBtc)}
                </span>
              </SheetTitle>
              <SheetDescription className="mt-1 truncate">
                {transaction.wallet} · {transaction.counterparty}
              </SheetDescription>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {explorer ? (
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label="Open explorer"
                  onClick={() => onOpenExplorer(transaction)}
                >
                  <ExternalLink className="size-4" aria-hidden="true" />
                </Button>
              ) : null}
              <Button
                type="button"
                size="icon"
                variant="ghost"
                aria-label="Close transaction detail"
                onClick={() => onOpenChange(false)}
              >
                <X className="size-4" aria-hidden="true" />
              </Button>
            </div>
          </div>
        </SheetHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="grid gap-4 p-4 sm:p-6 xl:grid-cols-[minmax(0,1fr)_340px]">
            <div className="min-w-0 space-y-4">
              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                <DetailField
                  label="Timestamp"
                  value={transaction.date}
                  copyValue={transaction.date}
                />
                <DetailField
                  label="Wallet"
                  value={transaction.wallet ?? "Unassigned"}
                  hidden={hideSensitive}
                />
                <DetailField
                  label="Transaction ID"
                  value={formatShortTxid(transaction.explorerId ?? transaction.txnId)}
                  copyValue={transaction.explorerId ?? transaction.txnId}
                  hidden={hideSensitive}
                />
                <DetailField
                  label="Price"
                  value={
                    localDraft.priceMode === "manual" && localDraft.manualPrice
                      ? `${localDraft.manualPrice} ${localDraft.manualCurrency}/BTC`
                      : transaction.rate
                      ? `${currencyFormatter.format(transaction.rate)} / BTC`
                      : "Missing"
                  }
                  hidden={hideSensitive}
                />
              </div>

              <Tabs value={activeTab} onValueChange={setActiveTab}>
                <TabsList className="grid w-full grid-cols-5">
                  <TabsTrigger value="details">Details</TabsTrigger>
                  <TabsTrigger value="classify">Classify</TabsTrigger>
                  <TabsTrigger value="pricing">Pricing</TabsTrigger>
                  <TabsTrigger value="tax">Tax</TabsTrigger>
                  <TabsTrigger value="ledger">Ledger</TabsTrigger>
                </TabsList>

                <TabsContent value="details" className="mt-4 space-y-4">
                  <div className="grid gap-3 lg:grid-cols-2">
                    <div className="rounded-md border">
                      <LedgerRow
                        label="Type"
                        value={transaction.sourceType ?? transaction.direction}
                      />
                      <LedgerRow label="Network" value={transaction.paymentMethod} />
                      <LedgerRow label="Counterparty" value={transaction.counterparty} />
                      <LedgerRow
                        label="External id"
                        value={formatShortTxid(transaction.txnId)}
                      />
                    </div>
                    <div className="rounded-md border">
                      <LedgerRow label="Label" value={localDraft.label} />
                      <LedgerRow
                        label="Tags"
                        value={
                          tags.length ? (
                            <div className="flex flex-wrap justify-end gap-1">
                              {tags.map((tag) => (
                                <Badge key={tag} variant="secondary" className="rounded-md">
                                  {tag}
                                </Badge>
                              ))}
                            </div>
                          ) : (
                            "None"
                          )
                        }
                      />
                      <LedgerRow
                        label="Included"
                        value={localDraft.excluded ? "Excluded" : "Included"}
                      />
                      <LedgerRow label="Last edited" value="Local draft" />
                    </div>
                  </div>
                  <div className="rounded-md border bg-muted/25 p-3">
                    <div className="mb-2 text-xs font-medium text-muted-foreground">
                      Note
                    </div>
                    <p className="min-h-10 whitespace-pre-wrap text-sm">
                      {localDraft.note || "-"}
                    </p>
                  </div>
                </TabsContent>

                <TabsContent value="ledger" className="mt-4">
                  <div className="overflow-hidden rounded-md border">
                    <LedgerRow
                      label="Asset"
                      value={transaction.asset ?? "BTC"}
                      align="right"
                    />
                    <LedgerRow
                      label="Amount"
                      value={
                        <span className={blurClass(hideSensitive)}>
                          {signedPrefix}
                          {formatBtcAmount(amountBtc)}
                        </span>
                      }
                      align="right"
                      muted
                    />
                    <LedgerRow
                      label="Value"
                      value={
                        <CurrencyToggleText className={blurClass(hideSensitive)}>
                          {formatDisplayMoney(transaction.amount, amountBtc, currency)}
                        </CurrencyToggleText>
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Fee"
                      value={
                        <span className={blurClass(hideSensitive)}>
                          {formatFee(transaction, currency)}
                        </span>
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Running balance"
                      value="Calculated after journal processing"
                      align="right"
                    />
                  </div>
                </TabsContent>

                <TabsContent value="pricing" className="mt-4">
                  <div className="grid gap-4">
                    <div className="grid gap-3 md:grid-cols-4">
                      {priceModeOptions.map((option) => (
                        <button
                          key={option.value}
                          type="button"
                          className={cn(
                            "rounded-md border p-3 text-left transition-colors hover:bg-muted/40",
                            localDraft.priceMode === option.value &&
                              "border-primary bg-muted/60",
                          )}
                          onClick={() => updateDraft("priceMode", option.value)}
                        >
                          <div className="text-sm font-medium">{option.label}</div>
                          <div className="mt-1 text-xs text-muted-foreground">
                            {option.value === "manual"
                              ? "Use invoice or receipt evidence"
                              : option.value === "rate-cache"
                                ? "Use cached market rate"
                                : option.value === "missing"
                                  ? "Keep in review queue"
                                  : "Use source-provided price"}
                          </div>
                        </button>
                      ))}
                    </div>
                    <div className="grid gap-3 rounded-md border bg-muted/20 p-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div>
                          <div className="text-sm font-medium">Manual price override</div>
                          <div className="text-xs text-muted-foreground">
                            Calculated from the fixed amount: {formatBtcAmount(amountBtc)}.
                          </div>
                        </div>
                        <Badge
                          variant="outline"
                          className={cn(
                            "rounded-md",
                            localDraft.priceMode === "manual"
                              ? "border-amber-600/30 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
                              : "text-muted-foreground",
                          )}
                        >
                          {priceModeLabel(localDraft.priceMode)}
                        </Badge>
                      </div>
                      <div className="grid gap-3 md:grid-cols-[100px_1fr_1fr]">
                        <div className="grid gap-2">
                          <Label htmlFor="tx-manual-currency">Currency</Label>
                          <Input
                            id="tx-manual-currency"
                            value={localDraft.manualCurrency}
                            onChange={(event) =>
                              updateDraft(
                                "manualCurrency",
                                event.target.value.toUpperCase(),
                              )
                            }
                            maxLength={3}
                          />
                        </div>
                        <div className="grid gap-2">
                          <Label htmlFor="tx-manual-price">Price / BTC</Label>
                          <Input
                            id="tx-manual-price"
                            inputMode="decimal"
                            value={localDraft.manualPrice}
                            onFocus={() => updateDraft("priceMode", "manual")}
                            onChange={(event) => updateManualPrice(event.target.value)}
                            placeholder="69453.46"
                          />
                        </div>
                        <div className="grid gap-2">
                          <Label htmlFor="tx-manual-value">Total value</Label>
                          <Input
                            id="tx-manual-value"
                            inputMode="decimal"
                            value={localDraft.manualValue}
                            onFocus={() => updateDraft("priceMode", "manual")}
                            onChange={(event) => updateManualValue(event.target.value)}
                            placeholder="17086.29"
                          />
                        </div>
                      </div>
                      <div className="grid gap-2">
                        <Label htmlFor="tx-manual-source">Evidence / source</Label>
                        <Input
                          id="tx-manual-source"
                          value={localDraft.manualSource}
                          onFocus={() => updateDraft("priceMode", "manual")}
                          onChange={(event) =>
                            updateDraft("manualSource", event.target.value)
                          }
                          placeholder="BTCPay invoice, bank receipt, accountant review"
                        />
                      </div>
                    </div>
                    <div className="grid gap-3 md:grid-cols-3">
                      <DetailField
                        label="Imported price"
                        value={
                          transaction.rate
                            ? `${currencyFormatter.format(transaction.rate)} / BTC`
                            : "None"
                        }
                        hidden={hideSensitive}
                      />
                      <DetailField
                        label="Source value"
                        value={currencyFormatter.format(transaction.amount)}
                        hidden={hideSensitive}
                      />
                      <DetailField
                        label="Manual source"
                        value={localDraft.manualSource || "-"}
                      />
                    </div>
                  </div>
                </TabsContent>

                <TabsContent value="tax" className="mt-4 space-y-3">
                  <div className="grid gap-3 md:grid-cols-3">
                    <DetailField label="Treatment" value={localDraft.taxTreatment} />
                    <DetailField label="Taxable" value={localDraft.taxable ? "Yes" : "No"} />
                    <DetailField
                      label="Price source"
                      value={priceModeLabel(localDraft.priceMode)}
                    />
                  </div>
                  <div className="overflow-hidden rounded-md border">
                    <LedgerRow
                      label="Cost basis"
                      value={currencyFormatter.format(transaction.amount)}
                      align="right"
                    />
                    <LedgerRow
                      label="Proceeds"
                      value={
                        flow === "outgoing"
                          ? currencyFormatter.format(transaction.amount)
                          : currencyFormatter.format(0)
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Gain / loss"
                      value="Pending journal run"
                      align="right"
                      muted
                    />
                    <LedgerRow
                      label="Austrian bucket"
                      value={
                        localDraft.taxTreatment === "Austrian carry basis"
                          ? "Swap basis carry"
                          : "Standard"
                      }
                      align="right"
                    />
                    {localDraft.priceMode === "manual" ? (
                      <LedgerRow
                        label="Manual price evidence"
                        value={localDraft.manualSource || "Source missing"}
                        align="right"
                        muted
                      />
                    ) : null}
                  </div>
                </TabsContent>

                <TabsContent value="classify" className="mt-4">
                  <div className="grid gap-4 lg:grid-cols-2">
                    <div className="grid gap-2">
                      <Label htmlFor="tx-label">Label</Label>
                      <Select
                        value={localDraft.label}
                        onValueChange={(value) => updateDraft("label", value)}
                      >
                        <SelectTrigger id="tx-label">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {classificationOptions.map((label) => (
                            <SelectItem key={label} value={label}>
                              {label}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid gap-2">
                      <Label htmlFor="tx-status">Review status</Label>
                      <Select
                        value={localDraft.reviewStatus}
                        onValueChange={(value) =>
                          updateDraft("reviewStatus", value as TransactionStatus)
                        }
                      >
                        <SelectTrigger id="tx-status">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {allTransactionStatuses.map((status) => (
                            <SelectItem key={status} value={status}>
                              {transactionStatusLabels[status]}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    <div className="grid gap-2 lg:col-span-2">
                      <Label htmlFor="tx-tag-input">Tags</Label>
                      <div className="rounded-md border bg-background p-2">
                        <div className="mb-2 flex min-h-8 flex-wrap gap-1.5">
                          {tags.length ? (
                            tags.map((tag) => (
                              <button
                                key={tag}
                                type="button"
                                className="inline-flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-xs font-medium text-secondary-foreground"
                                onClick={() => removeTag(tag)}
                                aria-label={`Remove ${tag} tag`}
                              >
                                {tag}
                                <X className="size-3" aria-hidden="true" />
                              </button>
                            ))
                          ) : (
                            <span className="px-1 py-1 text-sm text-muted-foreground">
                              No tags yet
                            </span>
                          )}
                        </div>
                        <div className="flex gap-2">
                          <Input
                            id="tx-tag-input"
                            value={tagInput}
                            onChange={(event) => setTagInput(event.target.value)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter" || event.key === ",") {
                                event.preventDefault();
                                addTag(tagInput);
                              }
                            }}
                            placeholder="Add tag"
                          />
                          <Button
                            type="button"
                            variant="outline"
                            size="icon"
                            aria-label="Add tag"
                            onClick={() => addTag(tagInput)}
                          >
                            <Plus className="size-4" aria-hidden="true" />
                          </Button>
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-1.5">
                        {availableTagSuggestions.slice(0, 7).map((tag) => (
                          <button
                            key={tag}
                            type="button"
                            className="rounded-md border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                            onClick={() => addTag(tag)}
                          >
                            + {tag}
                          </button>
                        ))}
                      </div>
                    </div>
                    <div className="rounded-md border bg-background p-3 lg:col-span-2">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <h3 className="text-sm font-semibold">Tax handling</h3>
                        <Badge variant={localDraft.taxable ? "default" : "outline"}>
                          {localDraft.excluded
                            ? "Excluded"
                            : localDraft.taxable
                              ? "Taxable"
                              : "Not taxable"}
                        </Badge>
                      </div>
                      <div className="grid gap-3 xl:grid-cols-[minmax(220px,0.9fr)_minmax(0,1fr)_minmax(0,1fr)]">
                        <div className="grid gap-2">
                          <Label htmlFor="tx-tax-treatment">Tax treatment</Label>
                          <Select
                            value={localDraft.taxTreatment}
                            onValueChange={(value) =>
                              updateDraft("taxTreatment", value)
                            }
                          >
                            <SelectTrigger id="tx-tax-treatment">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {taxTreatmentOptions.map((option) => (
                                <SelectItem key={option} value={option}>
                                  {option}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label htmlFor="tx-taxable">Taxable</Label>
                            <p className="text-xs text-muted-foreground">
                              Included in tax event preparation.
                            </p>
                          </div>
                          <Switch
                            id="tx-taxable"
                            checked={localDraft.taxable}
                            onCheckedChange={(checked) =>
                              updateDraft("taxable", checked)
                            }
                          />
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label htmlFor="tx-excluded">Excluded</Label>
                            <p className="text-xs text-muted-foreground">
                              Kept out of journal processing.
                            </p>
                          </div>
                          <Switch
                            id="tx-excluded"
                            checked={localDraft.excluded}
                            onCheckedChange={(checked) =>
                              updateDraft("excluded", checked)
                            }
                          />
                        </div>
                      </div>
                    </div>
                    <div className="grid gap-2 lg:col-span-2">
                      <Label htmlFor="tx-note">Note</Label>
                      <Textarea
                        id="tx-note"
                        value={localDraft.note}
                        onChange={(event) => updateDraft("note", event.target.value)}
                        className="min-h-28 resize-none"
                        placeholder="Receipt, invoice, counterparty, or review context"
                      />
                    </div>
                  </div>
                </TabsContent>
              </Tabs>
            </div>

            <aside className="space-y-3">
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <ListChecks className="size-4 text-muted-foreground" aria-hidden="true" />
                  Review
                </div>
                <div className="space-y-2 text-sm">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Label</span>
                    <span className="font-medium">{localDraft.label}</span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Tax</span>
                    <span className="text-right font-medium">{localDraft.taxTreatment}</span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Price</span>
                    <span className="text-right font-medium">
                      {priceModeLabel(localDraft.priceMode)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Flags</span>
                    <span className="font-medium">
                      {localDraft.excluded
                        ? "Excluded"
                        : localDraft.taxable
                          ? "Taxable"
                          : "Non-taxable"}
                    </span>
                  </div>
                </div>
              </div>
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <Hash className="size-4 text-muted-foreground" aria-hidden="true" />
                  Identity
                </div>
                <div className="space-y-2 text-xs text-muted-foreground">
                  <button
                    type="button"
                    className="flex w-full min-w-0 items-center justify-between gap-2 rounded-md border px-2 py-2 text-left hover:bg-muted/40"
                    onClick={() => copyText(transaction.txnId)}
                  >
                    <span>Transaction</span>
                    <span className={cn("truncate font-mono", blurClass(hideSensitive))}>
                      {formatShortTxid(transaction.txnId)}
                    </span>
                  </button>
                  <div className="flex items-center gap-2 rounded-md border px-2 py-2">
                    <CalendarClock className="size-3.5" aria-hidden="true" />
                    <span>{transaction.date}</span>
                  </div>
                  <div className="flex items-center gap-2 rounded-md border px-2 py-2">
                    <Link2 className="size-3.5" aria-hidden="true" />
                    <span>{explorer ? explorer.label : "No public explorer"}</span>
                  </div>
                </div>
              </div>
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <Tags className="size-4 text-muted-foreground" aria-hidden="true" />
                  Tags
                </div>
                <div className="flex min-h-8 flex-wrap gap-1.5">
                  {tags.length ? (
                    tags.map((tag) => (
                      <Badge key={tag} variant="secondary" className="rounded-md">
                        {tag}
                      </Badge>
                    ))
                  ) : (
                    <span className="text-sm text-muted-foreground">None</span>
                  )}
                </div>
              </div>
            </aside>
          </div>
        </div>

        <SheetFooter className="border-t p-3 sm:flex-row sm:items-center sm:justify-between sm:px-6">
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <BookMarked className="size-4" aria-hidden="true" />
            <span>Metadata changes require journal reprocessing.</span>
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              className="gap-2"
              onClick={() => {
                onSave(transaction.id, localDraft);
                onOpenChange(false);
              }}
            >
              <Save className="size-4" aria-hidden="true" />
              Save draft
            </Button>
          </div>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
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

const detailTabValues = ["details", "classify", "pricing", "tax", "ledger"] as const;

function readTransactionDetailParams() {
  if (typeof window === "undefined") return { transactionId: null, tab: "details" };
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab");
  return {
    transactionId:
      params.get("tx") ?? params.get("transaction") ?? params.get("transactionId"),
    tab: detailTabValues.includes(tab as (typeof detailTabValues)[number])
      ? tab ?? "details"
      : "details",
  };
}

function updateTransactionDetailParams(
  transactionId: string | null,
  tab = "details",
) {
  if (typeof window === "undefined") return;
  const params = new URLSearchParams(window.location.search);
  if (transactionId) {
    params.set("tx", transactionId);
    if (tab !== "details") {
      params.set("tab", tab);
    } else {
      params.delete("tab");
    }
  } else {
    params.delete("tx");
    params.delete("transaction");
    params.delete("transactionId");
    params.delete("tab");
  }
  const nextQuery = params.toString();
  window.history.replaceState(
    null,
    "",
    nextQuery ? `${window.location.pathname}?${nextQuery}` : window.location.pathname,
  );
}

function matchesTransactionDeepLink(txn: Transaction, transactionId: string) {
  const target = transactionId.trim().toLowerCase();
  return [txn.id, txn.txnId, txn.explorerId]
    .filter(Boolean)
    .some((value) => value?.toLowerCase() === target);
}

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
  const [detailTransaction, setDetailTransaction] =
    React.useState<Transaction | null>(null);
  const [detailInitialTab, setDetailInitialTab] = React.useState("details");
  const pendingDetailLinkRef = React.useRef(readTransactionDetailParams());
  const [drafts, setDrafts] = React.useState<Record<string, TransactionEditDraft>>(
    {},
  );
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const displayFlow = React.useCallback(
    (txn: Transaction): TransactionFlow =>
      swapCandidateIds.has(txn.id) ? "swap" : transactionFlow(txn),
    [swapCandidateIds],
  );
  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
  );

  const openTransactionDetail = React.useCallback(
    (txn: Transaction, tab = "details") => {
      setDetailInitialTab(tab);
      setDetailTransaction(txn);
      updateTransactionDetailParams(txn.id, tab);
    },
    [],
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

  React.useEffect(() => {
    const pending = pendingDetailLinkRef.current;
    if (!pending.transactionId) return;
    const transaction = records.find((txn) =>
      matchesTransactionDeepLink(txn, pending.transactionId ?? ""),
    );
    if (!transaction) return;
    pendingDetailLinkRef.current = { transactionId: null, tab: "details" };
    openTransactionDetail(transaction, pending.tab);
  }, [records, openTransactionDetail]);

  const filteredTransactions = React.useMemo(() => {
    const query = searchQuery.toLowerCase();
    return records.filter((txn) => {
      const draft = getDraft(txn);
      const matchesSearch =
        txn.txnId.toLowerCase().includes(query) ||
        txn.counterparty.toLowerCase().includes(query) ||
        (txn.wallet ?? "").toLowerCase().includes(query) ||
        (draft.label ?? "").toLowerCase().includes(query) ||
        draft.tags.join(" ").toLowerCase().includes(query) ||
        (draft.note ?? "").toLowerCase().includes(query) ||
        txn.paymentMethod.toLowerCase().includes(query);

      const matchesStatus =
        statusFilter === "all" || draft.reviewStatus === statusFilter;

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
    getDraft,
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
        <div className="flex flex-1 items-center gap-2">
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
        <Table className="min-w-[980px]">
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="min-w-[280px] text-xs font-medium text-muted-foreground sm:text-sm">
                Transaction
              </TableHead>
              <TableHead className="min-w-[140px] text-right text-xs font-medium text-muted-foreground sm:text-sm">
                Amount
              </TableHead>
              <TableHead className="hidden min-w-[190px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                Accounting
              </TableHead>
              <TableHead className="hidden min-w-[150px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Pricing
              </TableHead>
              <TableHead className="hidden min-w-[150px] text-xs font-medium text-muted-foreground sm:text-sm xl:table-cell">
                Network
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
                  colSpan={7}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  No transactions found matching your filters.
                </TableCell>
              </TableRow>
            ) : (
              paginatedTransactions.map((txn) => {
                const draft = getDraft(txn);
                const StatusIcon = transactionStatusIcons[draft.reviewStatus];
                const explorer = explorerForTransaction(txn, explorerSettings);
                const flow = displayFlow(txn);
                const tagPreview = draft.tags;
                const amountBtc = transactionBtc(txn);
                const signedAmountBtc =
                  flow === "outgoing" ? -amountBtc : amountBtc;
                const signedAmountEur =
                  flow === "outgoing" ? -txn.amount : txn.amount;
                const primaryAmount =
                  flow === "incoming" || flow === "outgoing"
                    ? formatSignedDisplayMoney(
                        signedAmountEur,
                        signedAmountBtc,
                        currency,
                      )
                    : formatDisplayMoney(txn.amount, amountBtc, currency);
                const FlowIcon =
                  flow === "incoming"
                    ? ArrowDownRight
                    : flow === "outgoing"
                      ? ArrowUpRight
                      : ArrowLeftRight;
                const amountTone =
                  flow === "incoming"
                    ? "text-emerald-700 dark:text-emerald-300"
                    : flow === "outgoing"
                      ? "text-red-700 dark:text-red-300"
                      : "text-muted-foreground";
                return (
                  <TableRow
                    key={txn.id}
                    className="cursor-pointer align-top hover:bg-muted/35"
                    onClick={() => openTransactionDetail(txn)}
                  >
                    <TableCell className="min-w-[280px]">
                      <div className="flex min-w-0 items-start gap-3">
                        <span
                          className={cn(
                            "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
                            transactionFlowStyles[flow],
                          )}
                          aria-hidden="true"
                        >
                          <FlowIcon className="size-4" />
                        </span>
                        <div className="min-w-0">
                          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                            <span
                              className={cn(
                                "truncate text-sm font-medium text-foreground",
                                blurClass(hideSensitive),
                              )}
                            >
                              {txn.counterparty}
                            </span>
                            <Badge variant="secondary" className="rounded-md">
                              {draft.label}
                            </Badge>
                          </div>
                          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
                            <span
                              className={cn("truncate", blurClass(hideSensitive))}
                            >
                              {txn.wallet || txn.paymentMethod}
                            </span>
                            <span aria-hidden="true">·</span>
                            <span>{txn.date}</span>
                            <span aria-hidden="true">·</span>
                            {explorer ? (
                              <button
                                type="button"
                                className={cn(
                                  "inline-flex max-w-[20ch] items-center gap-1 truncate font-mono text-left underline-offset-4 hover:underline",
                                  blurClass(hideSensitive),
                                )}
                                title={`Open ${txn.txnId} on ${explorer.label}`}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setExplorerTransaction(txn);
                                }}
                              >
                                <span className="truncate">
                                  {formatShortTxid(txn.txnId)}
                                </span>
                                <ExternalLink
                                  className="size-3 shrink-0 text-muted-foreground"
                                  aria-hidden="true"
                                />
                              </button>
                            ) : (
                              <span
                                className={cn(
                                  "truncate font-mono",
                                  blurClass(hideSensitive),
                                )}
                              >
                                {formatShortTxid(txn.txnId)}
                              </span>
                            )}
                          </div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="min-w-[140px] text-right">
                      <CurrencyToggleText
                        className={cn(
                          "text-sm font-semibold tabular-nums",
                          amountTone,
                          blurClass(hideSensitive),
                        )}
                      >
                        {primaryAmount}
                      </CurrencyToggleText>
                      <div
                        className={cn(
                          "mt-1 text-[10px] text-muted-foreground tabular-nums sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {formatCounterDisplayMoney(
                          txn.amount,
                          amountBtc,
                          currency,
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div className="flex max-w-[210px] flex-wrap gap-1">
                        {tagPreview.slice(0, 2).map((tag) => (
                          <Badge key={tag} variant="outline" className="rounded-md">
                            {tag}
                          </Badge>
                        ))}
                        {tagPreview.length > 2 && (
                          <Badge variant="outline" className="rounded-md">
                            +{tagPreview.length - 2}
                          </Badge>
                        )}
                      </div>
                      <p className="mt-1 truncate text-[10px] text-muted-foreground sm:text-xs">
                        {draft.taxTreatment}
                      </p>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          priceModeStyles[draft.priceMode],
                        )}
                      >
                        {priceModeLabel(draft.priceMode)}
                      </span>
                      <p
                        className={cn(
                          "mt-1 truncate text-[10px] text-muted-foreground sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {draft.priceMode === "manual"
                          ? `${draft.manualCurrency} ${draft.manualValue || "value pending"}`
                          : txn.rate
                            ? `${currencyFormatter.format(txn.rate)} / BTC`
                            : "Awaiting price"}
                      </p>
                    </TableCell>
                    <TableCell className="hidden xl:table-cell">
                      <div className="flex flex-wrap gap-1">
                        <span
                          className={cn(
                            "inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal sm:text-xs",
                            transactionFlowStyles[flow],
                          )}
                        >
                          {transactionFlowLabels[flow]}
                        </span>
                        <span className="inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal text-muted-foreground sm:text-xs">
                          {txn.paymentMethod}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="min-w-[120px]">
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          transactionStatusStyles[draft.reviewStatus],
                        )}
                      >
                        <StatusIcon className="size-3" aria-hidden="true" />
                        {transactionStatusLabels[draft.reviewStatus]}
                      </span>
                      <p className="mt-1 hidden text-[10px] text-muted-foreground sm:block sm:text-xs">
                        {draft.excluded
                          ? "Excluded"
                          : draft.taxable
                            ? "Taxable"
                            : "Not taxable"}
                      </p>
                    </TableCell>
                    <TableCell>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-7 text-muted-foreground hover:text-foreground sm:size-8"
                            aria-label={`Open actions for ${txn.txnId}`}
                            onClick={(event) => event.stopPropagation()}
                          >
                            <MoreHorizontal className="size-3.5 sm:size-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onSelect={() => openTransactionDetail(txn)}>
                            <Eye className="mr-2 size-4" aria-hidden="true" />
                            View Details
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => openTransactionDetail(txn, "classify")}
                          >
                            <Pencil
                              className="mr-2 size-4"
                              aria-hidden="true"
                            />
                            Classify
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => copyText(txn.explorerId ?? txn.txnId)}
                          >
                            <Copy className="mr-2 size-4" aria-hidden="true" />
                            Copy ID
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
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailSheet
        transaction={detailTransaction}
        draft={detailTransaction ? getDraft(detailTransaction) : null}
        initialTab={detailInitialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        onOpenChange={(open) => {
          if (!open) {
            setDetailTransaction(null);
            updateTransactionDetailParams(null);
          }
        }}
        onOpenExplorer={(transaction) => setExplorerTransaction(transaction)}
        onSave={(transactionId, draft) =>
          setDrafts((current) => ({
            ...current,
            [transactionId]: draft,
          }))
        }
      />
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
  const [period, setPeriod] = React.useState<PeriodKey>(initialPeriodFromUrl);
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [newTransactionDraft, setNewTransactionDraft] =
    React.useState<NewTransactionDraft>(createNewTransactionDraft);
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
    params.set("period", period);
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
          <NewTransactionDialog
            open={newTxnOpen}
            draft={newTransactionDraft}
            walletSourceOptions={mockNewTransactionWalletSourceOptions}
            onOpenChange={setNewTxnOpen}
            onDraftChange={setNewTransactionDraft}
            onSaveDraft={() => {
              setNewTxnOpen(false);
              setNewTransactionDraft(createNewTransactionDraft());
            }}
          />
        </div>
      </div>

      <TransactionWorkbench
        period={period}
        records={periodRecords}
        hideSensitive={hideSensitive}
        currency={currency}
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
