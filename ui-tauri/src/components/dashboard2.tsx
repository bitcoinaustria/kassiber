import {
  ArrowDownRight,
  ArrowUpRight,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  CircleDollarSign,
  Clock,
  Copy,
  CreditCard,
  Download,
  Eye,
  Filter,
  Maximize2,
  MoreHorizontal,
  Pencil,
  Plus,
  RotateCcw,
  Search,
  ShoppingCart,
  Wallet,
  X,
  XCircle,
} from "lucide-react";
import * as React from "react";
import {
  Area,
  AreaChart,
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
import {
  MOCK_TRANSACTIONS,
  type TransactionsLedger,
} from "@/mocks/transactions";
import type { Tx } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

type TransactionStatus = "completed" | "pending" | "failed" | "review";

type TransactionDirection = "Receive" | "Send" | "Transfer";

type Transaction = {
  id: string;
  txnId: string;
  amount: number;
  counterparty: string;
  counterpartyInitials: string;
  direction: TransactionDirection;
  paymentMethod: "On-chain" | "Exchange" | "Lightning" | "Liquid";
  date: string;
  status: TransactionStatus;
};

type PeriodKey = "ytd" | "30days" | "3months" | "1year" | "5years";

type VolumeDataPoint = {
  month: string;
  revenue: number;
};

type CostDataPoint = {
  month: string;
  cogs: number;
  operatingExpenses: number;
};

type PeriodSummary = {
  total: number;
  change: number;
  isPositive: boolean;
};

type StatItem = {
  title: string;
  previousValue: number;
  value: number;
  changePercent: number;
  isPositive: boolean;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  format: "currency" | "percentage";
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

const percentFormatter = new Intl.NumberFormat("en-US", {
  style: "percent",
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

const mixBase = "var(--background)";

const palette = {
  primary: "var(--primary)",
  secondary: {
    light: `color-mix(in oklch, var(--primary) 75%, ${mixBase})`,
    dark: `color-mix(in oklch, var(--primary) 85%, ${mixBase})`,
  },
};

const revenueChartConfig = {
  revenue: { label: "Volume", color: palette.primary },
} satisfies ChartConfig;

const costsChartConfig = {
  cogs: { label: "Cost Basis", color: palette.primary },
  operatingExpenses: {
    label: "Fees",
    theme: palette.secondary,
  },
} satisfies ChartConfig;

const volumeData: Record<PeriodKey, VolumeDataPoint[]> = {
  ytd: [
    { month: "Jan", revenue: 398000 },
    { month: "Feb", revenue: 412000 },
    { month: "Mar", revenue: 445000 },
    { month: "Apr", revenue: 428000 },
  ],
  "30days": [
    { month: "Week 1", revenue: 87500 },
    { month: "Week 2", revenue: 95200 },
    { month: "Week 3", revenue: 102800 },
    { month: "Week 4", revenue: 91400 },
    { month: "Week 5", revenue: 110600 },
  ],
  "3months": [
    { month: "Oct", revenue: 342000 },
    { month: "Nov", revenue: 378000 },
    { month: "Dec", revenue: 456000 },
    { month: "Jan", revenue: 398000 },
    { month: "Feb", revenue: 412000 },
    { month: "Mar", revenue: 445000 },
  ],
  "1year": [
    { month: "Jan", revenue: 285000 },
    { month: "Feb", revenue: 312000 },
    { month: "Mar", revenue: 338000 },
    { month: "Apr", revenue: 356000 },
    { month: "May", revenue: 342000 },
    { month: "Jun", revenue: 378000 },
    { month: "Jul", revenue: 395000 },
    { month: "Aug", revenue: 418000 },
    { month: "Sep", revenue: 432000 },
    { month: "Oct", revenue: 456000 },
    { month: "Nov", revenue: 478000 },
    { month: "Dec", revenue: 512000 },
  ],
  "5years": [
    { month: "2022", revenue: 1620000 },
    { month: "2023", revenue: 2140000 },
    { month: "2024", revenue: 3310000 },
    { month: "2025", revenue: 4702000 },
    { month: "2026", revenue: 1835000 },
  ],
};

const costsData: Record<PeriodKey, CostDataPoint[]> = {
  ytd: [
    { month: "Jan", cogs: 143300, operatingExpenses: 79600 },
    { month: "Feb", cogs: 148300, operatingExpenses: 82400 },
    { month: "Mar", cogs: 160200, operatingExpenses: 89000 },
    { month: "Apr", cogs: 154000, operatingExpenses: 85600 },
  ],
  "30days": [
    { month: "Week 1", cogs: 31500, operatingExpenses: 18200 },
    { month: "Week 2", cogs: 34100, operatingExpenses: 19800 },
    { month: "Week 3", cogs: 37000, operatingExpenses: 20500 },
    { month: "Week 4", cogs: 32800, operatingExpenses: 19100 },
    { month: "Week 5", cogs: 39800, operatingExpenses: 21600 },
  ],
  "3months": [
    { month: "Oct", cogs: 123100, operatingExpenses: 68400 },
    { month: "Nov", cogs: 136100, operatingExpenses: 75600 },
    { month: "Dec", cogs: 164200, operatingExpenses: 91200 },
    { month: "Jan", cogs: 143300, operatingExpenses: 79600 },
    { month: "Feb", cogs: 148300, operatingExpenses: 82400 },
    { month: "Mar", cogs: 160200, operatingExpenses: 89000 },
  ],
  "1year": [
    { month: "Jan", cogs: 102600, operatingExpenses: 57000 },
    { month: "Feb", cogs: 112300, operatingExpenses: 62400 },
    { month: "Mar", cogs: 121700, operatingExpenses: 67600 },
    { month: "Apr", cogs: 128200, operatingExpenses: 71200 },
    { month: "May", cogs: 123100, operatingExpenses: 68400 },
    { month: "Jun", cogs: 136100, operatingExpenses: 75600 },
    { month: "Jul", cogs: 142200, operatingExpenses: 79000 },
    { month: "Aug", cogs: 150500, operatingExpenses: 83600 },
    { month: "Sep", cogs: 155500, operatingExpenses: 86400 },
    { month: "Oct", cogs: 164200, operatingExpenses: 91200 },
    { month: "Nov", cogs: 172100, operatingExpenses: 95600 },
    { month: "Dec", cogs: 184300, operatingExpenses: 102400 },
  ],
  "5years": [
    { month: "2022", cogs: 583200, operatingExpenses: 324000 },
    { month: "2023", cogs: 770400, operatingExpenses: 428000 },
    { month: "2024", cogs: 1191600, operatingExpenses: 662000 },
    { month: "2025", cogs: 1692720, operatingExpenses: 1013080 },
    { month: "2026", cogs: 660600, operatingExpenses: 385000 },
  ],
};

const volumeSummary: Record<PeriodKey, PeriodSummary> = {
  ytd: { total: 1683000, change: 15.4, isPositive: true },
  "30days": { total: 487500, change: 14.2, isPositive: true },
  "3months": { total: 2431000, change: 11.8, isPositive: true },
  "1year": { total: 4702000, change: 18.5, isPositive: true },
  "5years": { total: 13607000, change: 31.7, isPositive: true },
};

const costsSummary: Record<PeriodKey, PeriodSummary> = {
  ytd: { total: 942400, change: 10.4, isPositive: false },
  "30days": { total: 274400, change: 8.6, isPositive: false },
  "3months": { total: 1361400, change: 9.3, isPositive: false },
  "1year": { total: 2705800, change: 12.1, isPositive: false },
  "5years": { total: 6010000, change: 22.8, isPositive: false },
};

const statsData: StatItem[] = [
  {
    title: "Transaction Volume",
    previousValue: 426800,
    value: 487500,
    changePercent: 14.2,
    isPositive: true,
    icon: CircleDollarSign,
    format: "currency",
  },
  {
    title: "Realized Gain",
    previousValue: 168200,
    value: 213100,
    changePercent: 26.7,
    isPositive: true,
    icon: Wallet,
    format: "currency",
  },
  {
    title: "Review Rate",
    previousValue: 0.042,
    value: 0.031,
    changePercent: 26.2,
    isPositive: true,
    icon: RotateCcw,
    format: "percentage",
  },
  {
    title: "Avg Transaction",
    previousValue: 68.4,
    value: 74.2,
    changePercent: 8.5,
    isPositive: true,
    icon: CreditCard,
    format: "currency",
  },
];

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
  const direction: TransactionDirection = tx.internal
    ? "Transfer"
    : tx.amountSat >= 0
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
    txnId: tx.id || `TX-${index + 1}`,
    amount: Math.abs(tx.eur || (tx.amountSat / 100_000_000) * tx.rate),
    counterparty: tx.counter || tx.account || "Unassigned",
    counterpartyInitials: initials(tx.counter || tx.account || "TX"),
    direction,
    paymentMethod,
    date: tx.date,
    status: tx.tag.toLowerCase().includes("review") ? "review" : status,
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

const PAGE_SIZE_OPTIONS = [5, 10, 20];

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
  return records.slice(0, periodLimit(period)).reverse();
}

function buildVolumeRows(records: Transaction[], period: PeriodKey): VolumeDataPoint[] {
  if (records.length === 0) return volumeData[period];
  return recordsForPeriod(records, period).map((txn, index) => ({
    month: period === "30days" ? `#${index + 1}` : txn.date.slice(0, 10) || `#${index + 1}`,
    revenue: txn.amount,
  }));
}

function buildCostRows(records: Transaction[], period: PeriodKey): CostDataPoint[] {
  if (records.length === 0) return costsData[period];
  return recordsForPeriod(records, period).map((txn, index) => ({
    month: period === "30days" ? `#${index + 1}` : txn.date.slice(0, 10) || `#${index + 1}`,
    cogs: txn.direction === "Send" ? txn.amount : 0,
    operatingExpenses: txn.status === "review" ? txn.amount : 0,
  }));
}

function summarizeVolume(records: Transaction[], period: PeriodKey): PeriodSummary {
  if (records.length === 0) return volumeSummary[period];
  const rows = recordsForPeriod(records, period);
  const total = rows.reduce((sum, txn) => sum + txn.amount, 0);
  return { total, change: rows.length, isPositive: true };
}

function summarizeCosts(records: Transaction[], period: PeriodKey): PeriodSummary {
  if (records.length === 0) return costsSummary[period];
  const rows = recordsForPeriod(records, period);
  const total = rows
    .filter((txn) => txn.direction === "Send" || txn.status === "review")
    .reduce((sum, txn) => sum + txn.amount, 0);
  return { total, change: rows.filter((txn) => txn.status === "review").length, isPositive: total === 0 };
}

function buildStatsData(records: Transaction[]): StatItem[] {
  const total = records.reduce((sum, txn) => sum + txn.amount, 0);
  const reviewed = records.filter((txn) => txn.status === "review").length;
  const sends = records.filter((txn) => txn.direction === "Send");
  const avg = records.length ? total / records.length : 0;
  return [
    { ...statsData[0], value: total, previousValue: 0, changePercent: records.length, isPositive: true },
    { ...statsData[1], value: sends.reduce((sum, txn) => sum + txn.amount, 0), previousValue: 0, changePercent: sends.length, isPositive: true },
    { ...statsData[2], value: records.length ? reviewed / records.length : 0, previousValue: 0, changePercent: reviewed, isPositive: reviewed === 0 },
    { ...statsData[3], value: avg, previousValue: 0, changePercent: records.length, isPositive: true },
  ];
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
}

function VolumeTooltip({
  active,
  payload,
  label,
  hideSensitive,
}: ChartTooltipProps) {
  if (!active || !payload?.length) return null;

  const value = payload[0]?.value || 0;

  return (
    <div className="rounded-lg border border-border bg-popover p-2 shadow-lg sm:p-3">
      <p className="mb-1.5 text-xs font-medium text-foreground sm:mb-2 sm:text-sm">
        {label}
      </p>
      <div className="flex items-center gap-1.5 sm:gap-2">
        <div className="size-2 rounded-full bg-foreground sm:size-2.5" />
        <span className="text-[10px] text-muted-foreground sm:text-sm">
          Volume:
        </span>
        <span
          className={cn(
            "text-[10px] font-medium text-foreground sm:text-sm",
            blurClass(hideSensitive),
          )}
        >
          {currencyFormatter.format(Number(value))}
        </span>
      </div>
    </div>
  );
}

const VolumeChart = ({
  period,
  records,
  hideSensitive,
}: {
  period: PeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
}) => {
  const data = buildVolumeRows(records, period);
  const summary = summarizeVolume(records, period);

  const renderChartCard = (expanded = false) => {
    const gradientId = expanded ? "revenueGradientExpanded" : "revenueGradient";

    return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 rounded-xl border bg-card p-4 sm:gap-5 sm:p-5">
      <div className="flex items-start justify-between gap-3">
        <div className="flex flex-col gap-1">
          <p className="text-xs text-muted-foreground sm:text-sm">
            Transaction Volume
          </p>
          <div className="flex items-center gap-2">
            <p
              className={cn(
                "text-xl leading-tight font-semibold tracking-tight sm:text-2xl",
                blurClass(hideSensitive),
              )}
            >
              {currencyFormatter.format(summary.total)}
            </p>
            <div className="flex items-center gap-0.5">
              {summary.isPositive ? (
                <ArrowUpRight
                  className="size-3.5 text-emerald-600"
                  aria-hidden="true"
                />
              ) : (
                <ArrowDownRight
                  className="size-3.5 text-red-600"
                  aria-hidden="true"
                />
              )}
              <span
                className={cn(
                  "text-xs font-medium",
                  summary.isPositive ? "text-emerald-600" : "text-red-600",
                )}
              >
                {summary.isPositive ? "+" : "-"}
                {summary.change}
              </span>
            </div>
          </div>
        </div>
        {!expanded && (
          <DialogTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 sm:size-8"
              aria-label="Expand transaction volume chart"
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
            : "h-[180px] w-full min-w-0 sm:h-[220px]"
        }
      >
        <ChartContainer
          config={revenueChartConfig}
          className={cn("h-full w-full", blurClass(hideSensitive))}
        >
          <AreaChart data={data}>
            <defs>
              <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                <stop
                  offset="0%"
                  stopColor="var(--color-revenue)"
                  stopOpacity={0.15}
                />
                <stop
                  offset="100%"
                  stopColor="var(--color-revenue)"
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
              content={<VolumeTooltip hideSensitive={hideSensitive} />}
              cursor={{ strokeOpacity: 0.2 }}
            />
            <Area
              type="monotone"
              dataKey="revenue"
              stroke="var(--color-revenue)"
              strokeWidth={2}
              fill={`url(#${gradientId})`}
            />
          </AreaChart>
        </ChartContainer>
      </div>
    </div>
    );
  };

  return (
    <Dialog>
      {renderChartCard()}
      <DialogContent className="max-w-[calc(100vw-2rem)] p-0 sm:max-w-[min(1120px,calc(100vw-2rem))]">
        <DialogTitle className="sr-only">
          Expanded transaction volume chart
        </DialogTitle>
        {renderChartCard(true)}
      </DialogContent>
    </Dialog>
  );
};

function CostsTooltip({
  active,
  payload,
  label,
  colors,
  hideSensitive,
}: ChartTooltipProps & {
  colors: { primary: string; secondary: string };
}) {
  if (!active || !payload?.length) return null;

  const cogs = payload.find((p) => p.dataKey === "cogs")?.value || 0;
  const operatingExpenses =
    payload.find((p) => p.dataKey === "operatingExpenses")?.value || 0;
  const total = Number(cogs) + Number(operatingExpenses);

  return (
    <div className="rounded-lg border border-border bg-popover p-2 shadow-lg sm:p-3">
      <p className="mb-1.5 text-xs font-medium text-foreground sm:mb-2 sm:text-sm">
        {label}
      </p>
      <div className="space-y-1 sm:space-y-1.5">
        <div className="flex items-center gap-1.5 sm:gap-2">
          <div
            className="size-2 rounded-full sm:size-2.5"
            style={{ backgroundColor: colors.primary }}
          />
          <span className="text-[10px] text-muted-foreground sm:text-sm">
            COGS:
          </span>
          <span
            className={cn(
              "text-[10px] font-medium text-foreground sm:text-sm",
              blurClass(hideSensitive),
            )}
          >
            {currencyFormatter.format(Number(cogs))}
          </span>
        </div>
        <div className="flex items-center gap-1.5 sm:gap-2">
          <div
            className="size-2 rounded-full sm:size-2.5"
            style={{ backgroundColor: colors.secondary }}
          />
          <span className="text-[10px] text-muted-foreground sm:text-sm">
            Operating:
          </span>
          <span
            className={cn(
              "text-[10px] font-medium text-foreground sm:text-sm",
              blurClass(hideSensitive),
            )}
          >
            {currencyFormatter.format(Number(operatingExpenses))}
          </span>
        </div>
        <div className="mt-1 border-t border-border pt-1">
          <span
            className={cn(
              "text-[10px] font-medium text-foreground sm:text-xs",
              blurClass(hideSensitive),
            )}
          >
            Total: {currencyFormatter.format(total)}
          </span>
        </div>
      </div>
    </div>
  );
}

const CostsChart = ({
  period,
  records,
  hideSensitive,
}: {
  period: PeriodKey;
  records: Transaction[];
  hideSensitive: boolean;
}) => {
  const data = buildCostRows(records, period);
  const summary = summarizeCosts(records, period);

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 rounded-xl border bg-card p-4 sm:gap-5 sm:p-5">
      <div className="flex flex-col gap-1">
        <div className="flex items-center justify-between">
          <p className="text-xs text-muted-foreground sm:text-sm">
            Costs Breakdown
          </p>
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-1.5">
              <div
                className="size-2 rounded-full"
                style={{ backgroundColor: "var(--color-cogs)" }}
              />
              <span className="text-[10px] text-muted-foreground sm:text-xs">
                COGS
              </span>
            </div>
            <div className="flex items-center gap-1.5">
              <div
                className="size-2 rounded-full"
                style={{ backgroundColor: "var(--color-operatingExpenses)" }}
              />
              <span className="text-[10px] text-muted-foreground sm:text-xs">
                Operating Expenses
              </span>
            </div>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <p
            className={cn(
              "text-xl leading-tight font-semibold tracking-tight sm:text-2xl",
              blurClass(hideSensitive),
            )}
          >
            {currencyFormatter.format(summary.total)}
          </p>
          <div className="flex items-center gap-0.5">
            {summary.isPositive ? (
              <ArrowUpRight
                className="size-3.5 text-emerald-600"
                aria-hidden="true"
              />
            ) : (
              <ArrowDownRight
                className="size-3.5 text-red-600"
                aria-hidden="true"
              />
            )}
            <span
              className={cn(
                "text-xs font-medium",
                summary.isPositive ? "text-emerald-600" : "text-red-600",
              )}
            >
              {summary.isPositive ? "+" : "-"}
              {summary.change}
            </span>
          </div>
        </div>
      </div>

      <div className="h-[180px] w-full min-w-0 sm:h-[220px]">
        <ChartContainer
          config={costsChartConfig}
          className={cn("h-full w-full", blurClass(hideSensitive))}
        >
          <BarChart data={data}>
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
                <CostsTooltip
                  colors={{
                    primary: "var(--color-cogs)",
                    secondary: "var(--color-operatingExpenses)",
                  }}
                  hideSensitive={hideSensitive}
                />
              }
              cursor={{ fillOpacity: 0.05 }}
            />
            <Bar
              dataKey="cogs"
              stackId="costs"
              fill="var(--color-cogs)"
              radius={[0, 0, 0, 0]}
            />
            <Bar
              dataKey="operatingExpenses"
              stackId="costs"
              fill="var(--color-operatingExpenses)"
              radius={[3, 3, 0, 0]}
            />
          </BarChart>
        </ChartContainer>
      </div>
    </div>
  );
};

const StatsCards = ({
  records,
  hideSensitive,
}: {
  records: Transaction[];
  hideSensitive: boolean;
}) => {
  const stats = buildStatsData(records);
  return (
    <div className="grid grid-cols-2 gap-3 rounded-xl border bg-card p-4 sm:gap-4 sm:p-5 lg:grid-cols-4 lg:gap-6 lg:p-6">
      {stats.map((stat, index) => {
        const formatter =
          stat.format === "currency" ? currencyFormatter : percentFormatter;

        return (
          <div key={stat.title} className="flex items-start">
            <div className="flex-1 space-y-1 sm:space-y-2 lg:space-y-3">
              <div className="flex items-center gap-1.5 text-muted-foreground sm:gap-2">
                <stat.icon className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="truncate text-[10px] font-medium sm:text-xs lg:text-sm">
                  {stat.title}
                </span>
              </div>
              <p
                className={cn(
                  "hidden text-[10px] text-muted-foreground/70 sm:block sm:text-xs",
                  stat.format === "currency" && blurClass(hideSensitive),
                )}
              >
                {formatter.format(stat.previousValue)} previous month
              </p>
              <p
                className={cn(
                  "text-xl leading-tight font-semibold tracking-tight sm:text-2xl lg:text-[28px]",
                  stat.format === "currency" && blurClass(hideSensitive),
                )}
              >
                {formatter.format(stat.value)}
              </p>
              <div className="flex flex-wrap items-center gap-x-1 gap-y-0.5 text-[10px] sm:text-xs">
                {stat.isPositive ? (
                  <ArrowUpRight
                    className="size-3 shrink-0 text-emerald-600 sm:size-3.5"
                    aria-hidden="true"
                  />
                ) : (
                  <ArrowDownRight
                    className="size-3 shrink-0 text-red-600 sm:size-3.5"
                    aria-hidden="true"
                  />
                )}
                <span
                  className={cn(
                    "whitespace-nowrap",
                    stat.isPositive ? "text-emerald-600" : "text-red-600",
                  )}
                >
                  {stat.isPositive ? "+" : "-"}
                  {stat.changePercent.toFixed(1)}%
                </span>
                <span className="whitespace-nowrap text-muted-foreground">
                  vs last month
                </span>
              </div>
            </div>
            {index < statsData.length - 1 && (
              <div className="mx-4 hidden h-full w-px bg-border lg:block xl:mx-6" />
            )}
          </div>
        );
      })}
    </div>
  );
};

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
  completed: "Completed",
  pending: "Pending",
  failed: "Failed",
  review: "Review",
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
}: {
  records: Transaction[];
  hideSensitive: boolean;
}) => {
  const [searchQuery, setSearchQuery] = React.useState("");
  const [statusFilter, setStatusFilter] = React.useState<string>("all");
  const [dateFilter, setDateFilter] = React.useState<string>("all");
  const [paymentMethodFilter, setPaymentMethodFilter] =
    React.useState<string>("all");
  const [currentPage, setCurrentPage] = React.useState(1);
  const [pageSize, setPageSize] = React.useState(10);
  const [isHydrated, setIsHydrated] = React.useState(false);

  const hasActiveFilters =
    statusFilter !== "all" ||
    dateFilter !== "all" ||
    paymentMethodFilter !== "all";

  const clearFilters = () => {
    setStatusFilter("all");
    setDateFilter("all");
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
        txn.counterparty.toLowerCase().includes(query);

      const matchesStatus =
        statusFilter === "all" || txn.status === statusFilter;

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
        matchesSearch && matchesStatus && matchesPaymentMethod && matchesDate
      );
    });
  }, [records, searchQuery, statusFilter, dateFilter, paymentMethodFilter]);

  const totalPages = Math.ceil(filteredTransactions.length / pageSize);

  const paginatedTransactions = React.useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    const endIndex = startIndex + pageSize;
    return filteredTransactions.slice(startIndex, endIndex);
  }, [filteredTransactions, currentPage, pageSize]);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [searchQuery, statusFilter, dateFilter, paymentMethodFilter, pageSize]);

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
    paymentMethodFilter,
    currentPage,
    pageSize,
    isHydrated,
  ]);

  const goToPage = (page: number) => {
    setCurrentPage(Math.max(1, Math.min(page, totalPages)));
  };

  return (
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
              placeholder="Search counterparty, tag, account..."
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
                  paymentMethodFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by payment method"
              >
                <Wallet className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Method</span>
                {paymentMethodFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[200px]">
              <DropdownMenuLabel>Source</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={paymentMethodFilter === "all"}
                onCheckedChange={() => setPaymentMethodFilter("all")}
              >
                All Methods
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
                Counterparty
              </TableHead>
              <TableHead className="min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Amount
              </TableHead>
              <TableHead className="hidden min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Type
              </TableHead>
              <TableHead className="hidden min-w-[120px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                Source
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
                return (
                  <TableRow key={txn.id}>
                    <TableCell
                      className={cn(
                        "text-xs font-medium sm:text-sm",
                        blurClass(hideSensitive),
                      )}
                    >
                      {txn.txnId}
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div className="flex items-center gap-2">
                        <Avatar className="size-6 bg-muted">
                          <AvatarFallback className="text-[8px] font-semibold text-muted-foreground uppercase">
                            {txn.counterpartyInitials}
                          </AvatarFallback>
                        </Avatar>
                        <span
                          className={cn(
                            "text-xs text-muted-foreground sm:text-sm",
                            blurClass(hideSensitive),
                          )}
                        >
                          {txn.counterparty}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-xs text-foreground tabular-nums sm:text-sm",
                        blurClass(hideSensitive),
                      )}
                    >
                      {currencyFormatter.format(txn.amount)}
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <span className="inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal text-muted-foreground sm:text-xs">
                        {txn.direction}
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
  );
};

const Dashboard2 = ({
  className,
  ledger = MOCK_TRANSACTIONS,
}: {
  className?: string;
  ledger?: TransactionsLedger;
}) => {
  const [period, setPeriod] = React.useState<PeriodKey>("1year");
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [txnNote, setTxnNote] = React.useState("");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const records = React.useMemo(
    () =>
      ledger.txs.length
        ? ledger.txs.map(toDashboardTransaction)
        : transactionRecords,
    [ledger.txs],
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
            aria-label="Export"
          >
            <Download className="size-4" aria-hidden="true" />
            <span className="hidden sm:inline">Export</span>
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

      <div className="flex flex-col gap-4 sm:gap-6 lg:flex-row">
        <VolumeChart
          period={period}
          records={records}
          hideSensitive={hideSensitive}
        />
        <CostsChart
          period={period}
          records={records}
          hideSensitive={hideSensitive}
        />
      </div>

      <StatsCards records={records} hideSensitive={hideSensitive} />

      <TransactionsTable records={records} hideSensitive={hideSensitive} />
    </div>
  );
};

export { Dashboard2 };
