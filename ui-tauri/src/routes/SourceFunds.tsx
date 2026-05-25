import {
  AlertTriangle,
  ArrowDownRight,
  ArrowLeftRight,
  ArrowRight,
  ArrowUpRight,
  ChevronDown,
  Check,
  ExternalLink,
  Eye,
  FileCheck,
  FileDown,
  GitBranch,
  Link2,
  Plus,
  RefreshCw,
  Search,
  ShieldAlert,
  SlidersHorizontal,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Trans, useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { Button } from "@/components/ui/button";
import { type Transaction } from "@/components/transactions";
import { TransactionDetailController } from "@/components/transactions/dashboard/TransactionDetailController";
import { toDashboardTransaction } from "@/components/transactions/dashboard/model";
import { useCurrency } from "@/lib/currency";
import { type Tx } from "@/mocks/seed";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { screenShellClassName } from "@/lib/screen-layout";
import { sourceFundsExportArgs } from "@/lib/sourceFundsExport";
import { useUiStore } from "@/store/ui";

type TransactionRow = {
  id?: string;
  transaction_id?: string;
  externalId?: string;
  external_id?: string;
  explorerId?: string;
  occurred_at?: string;
  date?: string;
  account?: string;
  wallet?: string;
  counter?: string;
  direction?: string;
  type?: string;
  flow?: string;
  asset?: string;
  amount?: number;
  amount_msat?: number;
  amountSat?: number;
  eur?: number;
  rate?: number;
  tag?: string;
  note?: string;
  conf?: number;
  internal?: boolean;
  description?: string;
};

type SourceFundsRecipient = {
  id: string;
  label: string;
  kind: string;
  default_reveal_mode: string;
  notes?: string;
  active?: boolean;
  created_at?: string;
  updated_at?: string;
};

type SourceFundsCoverageBucket = {
  amount: number;
  amount_msat: number;
  tx_count: number;
};

type SourceFundsCoverageBuckets = {
  fully_traced: SourceFundsCoverageBucket;
  attested: SourceFundsCoverageBucket;
  in_review: SourceFundsCoverageBucket;
  untraced: SourceFundsCoverageBucket;
  not_classified: SourceFundsCoverageBucket;
};

type SourceFundsCoverage = {
  by_wallet: {
    wallet_id: string;
    wallet_label: string;
    asset: string;
    buckets: SourceFundsCoverageBuckets;
    total_inbound: number;
    total_inbound_msat: number;
  }[];
  by_asset: {
    asset: string;
    buckets: SourceFundsCoverageBuckets;
    total_inbound: number;
    total_inbound_msat: number;
  }[];
  totals: {
    buckets: SourceFundsCoverageBuckets;
    tx_count: number;
    amount: number;
    amount_msat: number;
  };
  limits?: {
    max_depth?: number;
    max_transactions?: number;
  };
  truncation?: {
    truncated: boolean;
    inbound_total_count: number;
    inbound_total_msat: number;
    inbound_total: number;
    not_classified_count: number;
    not_classified_msat: number;
    not_classified: number;
  };
};

type SourceFundsFindingNextStep = {
  headline?: string;
  action?: string;
  action_args?: Record<string, unknown>;
  doc_anchor?: string;
};

type SourceFundsFinding = {
  code: string;
  severity?: string;
  message: string;
  ref?: string;
  next_step?: SourceFundsFindingNextStep;
};

const BULK_REVIEWABLE_METHODS = new Set([
  "same_external_id",
  "transaction_pair",
  "provider_trade_id",
  "provider_order_id",
  "provider_payment_id",
  "provider_exchange_order_id",
  "provider_ledger_id",
]);

type SourceFundsPreview = {
  purpose?: {
    type: "existing_transaction" | "planned_exchange_sale";
    label: string;
    anchor_role: string;
    planned_destination?: string;
    planned_note?: string;
    fiat_purchase_note?: string;
  };
  target: {
    transaction_id: string;
    label: string;
    wallet?: string;
    asset: string;
    required_amount: number;
    external_id?: string;
  };
  reveal_mode: string;
  graph: {
    nodes: Record<string, unknown>[];
    edges: Record<string, unknown>[];
  };
  source_mix: {
    source_type: string;
    amount: number;
    amount_msat?: number;
    count: number;
    percent_of_target?: number;
  }[];
  report_context?: {
    tax_country?: string;
    fiat_currency?: string;
    jurisdiction_label?: string;
    template_key?: string;
    report_title?: string;
    report_subtitle?: string;
  };
  overview?: {
    target_label?: string;
    target_asset?: string;
    target_amount?: number;
    target_fiat_value?: number | null;
    target_fiat_currency?: string;
    target_date?: string;
    target_wallet?: string;
    time_range?: {
      start?: string;
      end?: string;
    };
    transaction_count?: number;
    link_count?: number;
    root_source_count?: number;
    source_category_count?: number;
    data_source_count?: number;
    blocker_count?: number;
    warning_count?: number;
  };
  narrative?: {
    generated_by?: string;
    paragraphs?: string[];
  };
  data_sources?: {
    label: string;
    kind: string;
    transaction_count: number;
    source_count: number;
    assets: string[];
    first_seen?: string;
    last_seen?: string;
  }[];
  simplified_flow?: {
    note?: string;
    deferred_privacy_hops?: unknown[];
    levels: {
      level?: number;
      role?: string;
      distance_to_target?: number;
      nodes: {
        id: string;
        node_type?: string;
        kind?: string;
        label?: string;
        wallet?: string;
        asset?: string;
        amount?: number | null;
        occurred_at?: string;
        deferred_privacy_hop?: boolean;
      }[];
    }[];
    edges?: Record<string, unknown>[];
  };
  flow_levels?: {
    level: number;
    role: string;
    transaction_count: number;
    source_count: number;
    nodes: {
      id: string;
      node_type: string;
      label: string;
      wallet?: string;
      source_type?: string;
      asset?: string;
      required_amount?: number | null;
      amount?: number | null;
      occurred_at?: string;
      acquired_at?: string;
      external_id?: string;
    }[];
  }[];
  diagrams?: {
    flow_svg?: string;
    source_mix_ring_svg?: string;
    data_source_ring_svg?: string;
  };
  case?: {
    id: string;
    status: string;
    snapshot_hash: string;
  };
  findings: SourceFundsFinding[];
  explain_gates: {
    exportable: boolean;
    blockers: SourceFundsFinding[];
    warnings: SourceFundsFinding[];
  };
  disclosure_preview: {
    txids: string[];
    explorer_links?: {
      txid: string;
      asset?: string;
      chain?: string;
      network?: string;
      label: string;
      url: string;
    }[];
    attachments: { id: string; label: string; attachment_type?: string }[];
    privacy_note: string;
    excluded: string[];
  };
};

type SourceFundsSource = {
  id: string;
  source_type: string;
  label: string;
  asset: string;
  amount?: number | null;
  amount_msat?: number | null;
  description?: string;
  attachments?: EvidenceAttachment[];
};

type SourceFundsLink = {
  id: string;
  from_source_id?: string | null;
  from_transaction_id?: string | null;
  to_transaction_id: string;
  link_type: string;
  state: string;
  confidence: string;
  method: string;
  asset: string;
  allocation_amount?: number | null;
  from_allocation_amount?: number | null;
  allocation_policy: string;
  explanation?: string;
  uses_chain_observation?: boolean;
  attachments?: EvidenceAttachment[];
};

type EvidenceAttachment = {
  id: string;
  label: string;
  attachment_type?: string;
  external_id?: string;
  wallet?: string;
  occurred_at?: string;
};

const SOURCE_TYPES = [
  "fiat_purchase",
  "exchange_withdrawal",
  "mining",
  "income",
  "gift",
  "opening_balance_attestation",
  "missing_history",
  "unknown",
];

const LINK_TYPES = [
  "self_transfer",
  "exchange_transfer",
  "trade",
  "swap",
  "peg_in",
  "peg_out",
  "lightning_funding",
  "lightning_close",
  "lightning_routed",
  "lightning_swap",
  "coinjoin",
  "payjoin",
  "manual_source",
  "missing_history",
];

const CONFIDENCE_LEVELS = ["exact", "strong", "weak", "unknown"];
const REVEAL_MODES = ["labels_only", "minimal", "standard", "full"];
const NO_ATTACHMENT = "__none__";
const WIZARD_STEPS = [
  { id: "setup" },
  { id: "review" },
  { id: "export" },
] as const;

type WizardStep = (typeof WIZARD_STEPS)[number]["id"];
type TxPickerFlow = "incoming" | "outgoing" | "transfer" | "swap";

function transactionRows(data: unknown): TransactionRow[] {
  if (!data || typeof data !== "object") return [];
  const payload = data as {
    txs?: TransactionRow[];
    transactions?: TransactionRow[];
  };
  if (Array.isArray(payload.txs)) return payload.txs;
  if (Array.isArray(payload.transactions)) return payload.transactions;
  if (Array.isArray(data)) return data as TransactionRow[];
  return [];
}

function txRef(row: TransactionRow): string {
  return row.id || row.transaction_id || row.external_id || "";
}

function txLabel(row: TransactionRow): string {
  const date = row.occurred_at || row.date || "";
  const wallet = txWallet(row);
  const kind = row.direction || row.type || "transaction";
  const id = row.external_id || row.externalId || row.id || "";
  return [date.slice(0, 10), wallet, kind, id].filter(Boolean).join(" · ");
}

function txAmount(row: TransactionRow): string {
  const asset = row.asset || "BTC";
  if (typeof row.amount === "number") return `${row.amount.toFixed(8)} ${asset}`;
  if (typeof row.amountSat === "number") {
    return `${(Math.abs(row.amountSat) / 100_000_000).toFixed(8)} ${asset}`;
  }
  if (typeof row.amount_msat === "number") {
    return `${(Math.abs(row.amount_msat) / 100_000_000_000).toFixed(8)} ${asset}`;
  }
  return asset;
}

function txSignedAmount(row: TransactionRow): string {
  const asset = row.asset || "BTC";
  const flow = txFlow(row);
  const sign = flow === "incoming" ? "+" : flow === "outgoing" ? "-" : "";
  const format = (value: number) => `${sign}${Math.abs(value).toFixed(8)} ${asset}`;
  if (typeof row.amount === "number") return format(row.amount);
  if (typeof row.amountSat === "number") return format(row.amountSat / 100_000_000);
  if (typeof row.amount_msat === "number") return format(row.amount_msat / 100_000_000_000);
  return asset;
}

function txDirection(row: TransactionRow): string {
  return String(row.direction || row.type || "transaction").toLowerCase();
}

function txFlow(row: TransactionRow): TxPickerFlow {
  if (
    row.flow === "incoming" ||
    row.flow === "outgoing" ||
    row.flow === "transfer" ||
    row.flow === "swap"
  ) {
    return row.flow;
  }
  const type = String(row.type || "").toLowerCase();
  const tag = String(row.tag || "").toLowerCase();
  if (
    tag.includes("swap") ||
    type === "swap" ||
    type === "mint" ||
    type === "melt"
  ) {
    return "swap";
  }
  if (row.internal || type === "transfer" || type === "rebalance") {
    return "transfer";
  }
  if (typeof row.amountSat === "number") {
    return row.amountSat >= 0 ? "incoming" : "outgoing";
  }
  const direction = txDirection(row);
  if (direction.includes("inbound") || direction.includes("receive")) {
    return "incoming";
  }
  if (direction.includes("outbound") || direction.includes("send")) {
    return "outgoing";
  }
  if (typeof row.amount === "number" && row.amount < 0) return "outgoing";
  if (typeof row.amount === "number" && row.amount > 0) return "incoming";
  return "transfer";
}

function txFlowLabel(row: TransactionRow, t: SourceFundsTFunction): string {
  const flow = txFlow(row);
  if (flow === "incoming") return t("flow.incoming");
  if (flow === "outgoing") return t("flow.outgoing");
  if (flow === "swap") return t("flow.swap");
  return t("flow.transfer");
}

function txWallet(row: TransactionRow): string {
  return row.wallet || row.account || "Wallet";
}

function txStatus(row: TransactionRow): "confirmed" | "pending" | "review" {
  const tag = String(row.tag || "").toLowerCase();
  if (tag.includes("review")) return "review";
  return typeof row.conf === "number" && row.conf <= 0 ? "pending" : "confirmed";
}

function txNetwork(row: TransactionRow): string {
  const account = txWallet(row).toLowerCase();
  if (
    account.includes("lightning") ||
    account.includes(" ln") ||
    account.includes("ln ") ||
    account.includes("phoenix")
  ) {
    return "Lightning";
  }
  if (account.includes("liquid") || account.includes("lbtc")) return "Liquid";
  if (
    account.includes("exchange") ||
    account.includes("kraken") ||
    account.includes("bitpanda") ||
    account.includes("bitstamp") ||
    account.includes("coinbase") ||
    account.includes("river")
  ) {
    return "Exchange";
  }
  return "On-chain";
}

function txSearchText(row: TransactionRow): string {
  return [
    row.external_id,
    row.externalId,
    row.explorerId,
    row.transaction_id,
    row.id,
    row.wallet,
    row.account,
    row.counter,
    row.direction,
    row.type,
    row.tag,
    row.note,
    row.asset,
    row.description,
    row.occurred_at,
    row.date,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

function txDate(row: TransactionRow, t: SourceFundsTFunction): string {
  return (row.occurred_at || row.date || "").slice(0, 10) || t("fallback.unknownDate");
}

function txDateFilterValue(row: TransactionRow): string {
  const value = row.occurred_at || row.date || "";
  if (value.toLowerCase() === "today") return "today";
  if (value.toLowerCase() === "1 day ago") return "yesterday";
  const date = new Date(value.includes("T") ? value : value.replace(" ", "T"));
  if (Number.isNaN(date.getTime())) return "all";
  const now = new Date();
  const ageMs = now.getTime() - date.getTime();
  const ageDays = ageMs / 86_400_000;
  if (ageDays < 1 && date.toDateString() === now.toDateString()) return "today";
  if (ageDays <= 1.5) return "yesterday";
  if (ageDays <= 7) return "7days";
  if (ageDays <= 30) return "30days";
  return "older";
}

function uniqueSorted(values: string[]) {
  return Array.from(new Set(values.filter(Boolean))).sort((a, b) =>
    a.localeCompare(b),
  );
}

function formatBtc(value: number | null | undefined, asset = "BTC") {
  if (typeof value !== "number") return "-";
  return `${value.toFixed(8)} ${asset}`;
}

function formatDateTime(value?: string | null) {
  if (!value) return "-";
  return value.replace("T", " ").replace("Z", "").slice(0, 16);
}

function pretty(value: string) {
  return value.replaceAll("_", " ");
}

type SourceFundsTFunction = TFunction<"sourceFunds">;

// Enum -> display label. Known enum values resolve through the namespace; any
// value not enumerated in the bundle falls back to the underscore-stripped form.
function enumLabel(
  t: SourceFundsTFunction,
  group:
    | "sourceType"
    | "linkType"
    | "confidence"
    | "reveal"
    | "linkState"
    | "method",
  value?: string | null,
): string {
  if (!value) return pretty(value ?? "");
  const translated = t(`${group}.${value}`, { defaultValue: "" });
  return translated || pretty(value);
}

function shortId(value?: string | null) {
  if (!value) return "-";
  return value.length > 18 ? `${value.slice(0, 10)}...${value.slice(-6)}` : value;
}

function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function numberValue(value: unknown): number | null {
  return typeof value === "number" ? value : null;
}

function ReportControlFields({
  amountLabel,
  targetAmount,
  selectedTx,
  revealMode,
  onAmountChange,
  onRevealModeChange,
}: {
  amountLabel: string;
  targetAmount: string;
  selectedTx?: TransactionRow;
  revealMode: string;
  onAmountChange: (value: string) => void;
  onRevealModeChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <>
      <Field label={amountLabel} htmlFor="sof-amount">
        <Input
          id="sof-amount"
          value={targetAmount}
          onChange={(event) => onAmountChange(event.target.value)}
          placeholder={selectedTx ? txAmount(selectedTx) : t("controls.amountPlaceholder")}
        />
      </Field>
      <Field label={t("controls.revealLabel")} htmlFor="sof-reveal">
        <select
          id="sof-reveal"
          className="h-10 w-full rounded-md border bg-background px-3 text-sm"
          value={revealMode}
          onChange={(event) => onRevealModeChange(event.target.value)}
        >
          {REVEAL_MODES.map((mode) => (
            <option key={mode} value={mode}>
              {enumLabel(t, "reveal", mode)}
            </option>
          ))}
        </select>
      </Field>
    </>
  );
}

function TransactionTargetRow({
  row,
  active,
  onSelect,
  onOpenDetails,
}: {
  row: TransactionRow;
  active: boolean;
  onSelect: () => void;
  onOpenDetails: () => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const flow = txFlow(row);
  const FlowIcon =
    flow === "incoming"
      ? ArrowDownRight
      : flow === "outgoing"
        ? ArrowUpRight
        : ArrowLeftRight;
  const flowClassName =
    flow === "incoming"
      ? "border-emerald-600/20 bg-emerald-50 text-emerald-700 dark:bg-emerald-900/25 dark:text-emerald-300"
      : flow === "outgoing"
        ? "border-red-600/20 bg-red-50 text-red-700 dark:bg-red-900/25 dark:text-red-300"
        : "border-zinc-500/20 bg-zinc-50 text-zinc-700 dark:bg-zinc-800/70 dark:text-zinc-300";
  const amountClassName =
    flow === "incoming"
      ? "text-emerald-700 dark:text-emerald-300"
      : flow === "outgoing"
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";
  const txid = row.external_id || row.externalId || row.id;
  const description =
    row.counter || row.description || row.note || txid || t("transactionRow.fallbackDescription");

  return (
    <div
      className={[
        "flex items-stretch gap-1 rounded-md border transition-colors",
        active ? "border-primary bg-primary/5" : "hover:bg-muted/45",
      ].join(" ")}
    >
      <button
        type="button"
        className="min-w-0 flex-1 px-3 py-2 text-left"
        onClick={onSelect}
      >
      <div className="grid min-w-0 gap-3 md:grid-cols-[minmax(0,1fr)_140px_150px_130px] md:items-center">
        <div className="flex min-w-0 items-start gap-3">
          <span
            className={`mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border ${flowClassName}`}
            aria-hidden="true"
          >
            <FlowIcon className="size-4" />
          </span>
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-foreground">
              {description}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
              <span>{row.asset || "BTC"}</span>
              <span className="break-all font-mono">
                {shortId(txid)}
              </span>
              {row.direction && (
                <span className="md:hidden">{txFlowLabel(row, t)}</span>
              )}
            </div>
          </div>
        </div>
        <div className={`font-mono text-sm tabular-nums md:text-right ${amountClassName}`}>
          {txSignedAmount(row)}
        </div>
        <div className="text-sm text-muted-foreground">
          <span className="md:hidden">{t("transactionRow.walletMobile")}</span>
          {txWallet(row)}
        </div>
        <div className="flex flex-wrap items-center gap-2 md:justify-end">
          <span className={`inline-flex items-center gap-1 rounded-md border px-2 py-1 text-xs font-medium ${flowClassName}`}>
            <FlowIcon className="size-3.5" aria-hidden="true" />
            {txFlowLabel(row, t)}
          </span>
          <span className="text-xs text-muted-foreground">
            {txDate(row, t)}
          </span>
        </div>
      </div>
      </button>
      <button
        type="button"
        className="flex shrink-0 items-center border-l px-2.5 text-muted-foreground transition-colors hover:text-foreground"
        onClick={onOpenDetails}
        aria-label="View transaction details"
        title="View details"
      >
        <Eye className="size-4" aria-hidden="true" />
      </button>
    </div>
  );
}

function TransactionTargetHeader() {
  const { t } = useTranslation("sourceFunds");
  return (
    <div className="hidden border-b bg-muted/35 px-5 py-2 text-xs font-medium text-muted-foreground md:grid md:grid-cols-[minmax(0,1fr)_140px_150px_130px]">
      <span>{t("transactionRow.header.transaction")}</span>
      <span className="text-right">{t("transactionRow.header.amount")}</span>
      <span>{t("transactionRow.header.wallet")}</span>
      <span className="text-right">{t("transactionRow.header.flow")}</span>
    </div>
  );
}

export function SourceFunds() {
  const { t } = useTranslation(["sourceFunds", "common"]);
  const addNotification = useUiStore((state) => state.addNotification);
  const profileKey = useUiStore(
    (state) => state.identity?.profile ?? "default",
  );
  const persistedDraft = useUiStore(
    (state) => state.sourceFundsDrafts[profileKey] ?? null,
  );
  const setSourceFundsDraft = useUiStore((state) => state.setSourceFundsDraft);
  const [currentStep, setCurrentStep] = useState<WizardStep>(
    persistedDraft?.currentStep ?? "setup",
  );
  const [reportPurpose, setReportPurpose] = useState<
    "planned_exchange_sale" | "existing_transaction"
  >(persistedDraft?.reportPurpose ?? "planned_exchange_sale");
  const [target, setTarget] = useState(persistedDraft?.target ?? "");
  const [detailTransaction, setDetailTransaction] = useState<Transaction | null>(
    null,
  );
  const currency = useCurrency();
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const explorerSettings = useUiStore((state) => state.explorerSettings);
  const [targetAmount, setTargetAmount] = useState(
    persistedDraft?.targetAmount ?? "",
  );
  const [targetSearch, setTargetSearch] = useState("");
  const [targetDirectionFilter, setTargetDirectionFilter] = useState("all");
  const [targetDateFilter, setTargetDateFilter] = useState("all");
  const [targetStatusFilter, setTargetStatusFilter] = useState("all");
  const [targetNetworkFilter, setTargetNetworkFilter] = useState("all");
  const [targetAssetFilter, setTargetAssetFilter] = useState("all");
  const [targetWalletFilter, setTargetWalletFilter] = useState("all");
  const [plannedDestination, setPlannedDestination] = useState(
    persistedDraft?.plannedDestination ?? "",
  );
  const [plannedNote, setPlannedNote] = useState(
    persistedDraft?.plannedNote ?? "",
  );
  const [revealMode, setRevealMode] = useState(
    persistedDraft?.revealMode ?? "standard",
  );
  const [diagramDetail, setDiagramDetail] = useState<"summary" | "detailed">(
    persistedDraft?.diagramDetail ?? "summary",
  );
  const [amountPrecision, setAmountPrecision] = useState<"btc" | "sats">("btc");
  const [maskRecipient, setMaskRecipient] = useState(false);
  const [omitSections, setOmitSections] = useState<string[]>([]);
  const [revealOverrides, setRevealOverrides] = useState<
    Record<string, "show" | "hide">
  >({});
  const [selectedRecipientId, setSelectedRecipientId] = useState<string>(
    persistedDraft?.selectedRecipientId ?? "",
  );
  const [selectedLinkId, setSelectedLinkId] = useState("");
  const [linkForm, setLinkForm] = useState({
    link_type: "self_transfer",
    confidence: "strong",
    allocation_amount: "",
    from_allocation_amount: "",
    explanation: "",
    attachment_id: NO_ATTACHMENT,
  });
  const [linkFormSourceId, setLinkFormSourceId] = useState("");
  const [sourceForm, setSourceForm] = useState({
    source_type: "fiat_purchase",
    label: "",
    asset: "BTC",
    amount: "",
    description: "",
    attachment_id: NO_ATTACHMENT,
    to_transaction: "",
    link_type: "manual_source",
  });
  const [manualLinkForm, setManualLinkForm] = useState({
    from_transaction: "",
    to_transaction: "",
    link_type: "self_transfer",
    allocation_amount: "",
    from_allocation_amount: "",
    confidence: "strong",
    explanation: "",
    attachment_id: NO_ATTACHMENT,
  });
  const [showCoverage, setShowCoverage] = useState(false);
  const [showAdvancedTargetFilters, setShowAdvancedTargetFilters] =
    useState(false);
  const [showAdvancedReview, setShowAdvancedReview] = useState(false);

  const transactions = useDaemon<unknown>("ui.transactions.list", { limit: 500 });
  const rows = useMemo(
    () => transactionRows(transactions.data?.data),
    [transactions.data],
  );
  const targetAssetOptions = useMemo(
    () => uniqueSorted(rows.map((row) => row.asset || "BTC")),
    [rows],
  );
  const targetWalletOptions = useMemo(
    () => uniqueSorted(rows.map(txWallet)),
    [rows],
  );
  const targetNetworkOptions = useMemo(
    () => uniqueSorted(rows.map(txNetwork)),
    [rows],
  );
  const filteredTargetRows = useMemo(() => {
    const query = targetSearch.trim().toLowerCase();
    return rows.filter((row) => {
      const matchesSearch = !query || txSearchText(row).includes(query);
      const matchesDirection =
        targetDirectionFilter === "all" || txFlow(row) === targetDirectionFilter;
      const matchesDate =
        targetDateFilter === "all" ||
        txDateFilterValue(row) === targetDateFilter ||
        (targetDateFilter === "30days" && txDateFilterValue(row) === "7days");
      const matchesStatus =
        targetStatusFilter === "all" || txStatus(row) === targetStatusFilter;
      const matchesNetwork =
        targetNetworkFilter === "all" || txNetwork(row) === targetNetworkFilter;
      const matchesAsset =
        targetAssetFilter === "all" || (row.asset || "BTC") === targetAssetFilter;
      const matchesWallet =
        targetWalletFilter === "all" || txWallet(row) === targetWalletFilter;
      return (
        matchesSearch &&
        matchesDirection &&
        matchesDate &&
        matchesStatus &&
        matchesNetwork &&
        matchesAsset &&
        matchesWallet
      );
    });
  }, [
    rows,
    targetSearch,
    targetDirectionFilter,
    targetDateFilter,
    targetStatusFilter,
    targetNetworkFilter,
    targetAssetFilter,
    targetWalletFilter,
  ]);
  const clearTargetFilters = () => {
    setTargetSearch("");
    setTargetDirectionFilter("all");
    setTargetDateFilter("all");
    setTargetStatusFilter("all");
    setTargetNetworkFilter("all");
    setTargetAssetFilter("all");
    setTargetWalletFilter("all");
  };
  const targetFiltersActive =
    Boolean(targetSearch) ||
    targetDirectionFilter !== "all" ||
    targetDateFilter !== "all" ||
    targetStatusFilter !== "all" ||
    targetNetworkFilter !== "all" ||
    targetAssetFilter !== "all" ||
    targetWalletFilter !== "all";
  const selectedTarget = target || txRef(rows[0] ?? {});
  const selectedTx = rows.find((row) => txRef(row) === selectedTarget) ?? rows[0];
  const selectedTxId = selectedTx?.id || selectedTx?.transaction_id || "";
  const selectedTargetAmount =
    targetAmount ||
    (typeof selectedTx?.amount === "number" ? selectedTx.amount.toFixed(8) : "");
  const txById = useMemo(() => {
    const mapping = new Map<string, TransactionRow>();
    rows.forEach((row) => {
      if (row.id) mapping.set(row.id, row);
      if (row.transaction_id) mapping.set(row.transaction_id, row);
    });
    return mapping;
  }, [rows]);
  // Open the shared transaction detail panel for a transaction id (used by the
  // picker's details affordance, the review gates, and the flow path nodes).
  const openTxDetailById = (txId: string) => {
    if (!txId) return;
    const index = rows.findIndex(
      (row) => row.id === txId || row.transaction_id === txId || txRef(row) === txId,
    );
    if (index >= 0) {
      setDetailTransaction(toDashboardTransaction(rows[index] as unknown as Tx, index));
    }
  };

  const previewArgs = {
    target_transaction: selectedTarget,
    target_amount: targetAmount || undefined,
    report_purpose: reportPurpose,
    planned_destination:
      reportPurpose === "planned_exchange_sale"
        ? plannedDestination || undefined
        : undefined,
    planned_note:
      reportPurpose === "planned_exchange_sale" ? plannedNote || undefined : undefined,
    reveal_mode: revealMode,
    recipient: selectedRecipientId || undefined,
    report_options: {
      diagram_detail: diagramDetail,
      amount_precision: amountPrecision,
      mask_recipient: maskRecipient,
      omit_sections: omitSections,
      reveal_overrides: revealOverrides,
    },
  };
  const preview = useDaemon<SourceFundsPreview>(
    "ui.source_funds.preview",
    previewArgs,
    { enabled: Boolean(selectedTarget) },
  );
  const sourcesQuery = useDaemon<{ sources: SourceFundsSource[] }>(
    "ui.source_funds.sources.list",
  );
  const linksQuery = useDaemon<{ links: SourceFundsLink[] }>(
    "ui.source_funds.links.list",
  );
  const evidenceQuery = useDaemon<{ attachments: EvidenceAttachment[] }>(
    "ui.source_funds.evidence.list",
  );
  const coverageQuery = useDaemon<SourceFundsCoverage>(
    "ui.source_funds.coverage",
  );
  const recipientsQuery = useDaemon<{ recipients: SourceFundsRecipient[] }>(
    "ui.source_funds.recipients.list",
    { include_inactive: true },
  );
  const selectedRecipient = useMemo<SourceFundsRecipient | null>(() => {
    const all = recipientsQuery.data?.data?.recipients ?? [];
    return all.find((item) => item.id === selectedRecipientId) ?? null;
  }, [recipientsQuery.data, selectedRecipientId]);
  const suggestLinks = useDaemonMutation<{ inserted: number }>(
    "ui.source_funds.suggest",
  );
  const bulkReviewLinks = useDaemonMutation<{
    reviewed: number;
    skipped: number;
  }>("ui.source_funds.links.bulk_review");
  const reviewLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.review",
  );
  const attachLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.attach",
  );
  const createLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.create",
  );
  const createSource = useDaemonMutation<SourceFundsSource>(
    "ui.source_funds.sources.create",
  );
  const casesSave = useDaemonMutation<SourceFundsPreview>(
    "ui.source_funds.cases.save",
  );
  const exportPdf = useDaemonMutation("ui.source_funds.export_pdf");

  const report = preview.data?.data;
  const savedCase = casesSave.data?.data?.case ?? null;
  const handleExportPdf = async () => {
    if (!report?.explain_gates.exportable) return;
    if (casesSave.isPending || exportPdf.isPending) return;
    const saved = await casesSave.mutateAsync(previewArgs);
    const args = sourceFundsExportArgs(saved.data);
    if (!args) return;
    exportPdf.mutate(args);
  };
  const links = useMemo(
    () => linksQuery.data?.data?.links ?? [],
    [linksQuery.data],
  );
  const sources = useMemo(
    () => sourcesQuery.data?.data?.sources ?? [],
    [sourcesQuery.data],
  );
  const evidence = useMemo(
    () => evidenceQuery.data?.data?.attachments ?? [],
    [evidenceQuery.data],
  );
  const blockers = report?.explain_gates.blockers ?? [];
  const warnings = report?.explain_gates.warnings ?? [];
  const reachableLinkIds = useMemo(() => {
    const found = new Set<string>();
    if (!selectedTxId) return found;
    const byTo = new Map<string, SourceFundsLink[]>();
    links.forEach((link) => {
      const rowsForTarget = byTo.get(link.to_transaction_id) ?? [];
      rowsForTarget.push(link);
      byTo.set(link.to_transaction_id, rowsForTarget);
    });
    const queue = [selectedTxId];
    const visited = new Set<string>();
    while (queue.length > 0) {
      const txId = queue.shift();
      if (!txId || visited.has(txId)) continue;
      visited.add(txId);
      for (const link of byTo.get(txId) ?? []) {
        if (link.state === "rejected") continue;
        found.add(link.id);
        if (link.from_transaction_id) queue.push(link.from_transaction_id);
      }
    }
    return found;
  }, [links, selectedTxId]);
  const reviewQueueLinks = useMemo(() => {
    const rowsForReview = links.filter(
      (link) =>
        reachableLinkIds.has(link.id) ||
        link.to_transaction_id === selectedTxId ||
        link.state === "suggested",
    );
    const rows = rowsForReview.length > 0 ? rowsForReview : links;
    return [...rows].sort((a, b) => {
      const score = (link: SourceFundsLink) => {
        if (reachableLinkIds.has(link.id)) return 0;
        if (link.to_transaction_id === selectedTxId) return 1;
        if (link.state === "suggested") return 2;
        if (link.state === "reviewed") return 3;
        return 4;
      };
      const scoreDelta = score(a) - score(b);
      if (scoreDelta !== 0) return scoreDelta;
      return Number(a.state === "rejected") - Number(b.state === "rejected");
    });
  }, [links, reachableLinkIds, selectedTxId]);
  const selectedLink =
    reviewQueueLinks.find((link) => link.id === selectedLinkId) ??
    reviewQueueLinks.find((link) => link.state === "suggested") ??
    reviewQueueLinks[0] ??
    links[0];
  const selectedSource = sources.find(
    (source) => source.id === selectedLink?.from_source_id,
  );
  const bulkReviewableSuggestions = links.filter(
    (link) => reachableLinkIds.has(link.id) && isBulkReviewableLink(link),
  );
  const manualSuggestionCount = links.filter(
    (link) =>
      reachableLinkIds.has(link.id) &&
      link.state === "suggested" &&
      !isBulkReviewableLink(link),
  ).length;
  const exportedPdf = exportPdf.data?.data as { filename?: string } | undefined;
  const planned = reportPurpose === "planned_exchange_sale";
  const showStepContext =
    currentStep === "review" || currentStep === "export";
  const targetLabel = planned
    ? t("setup.targetLabelPlanned")
    : t("setup.targetLabelExisting");
  const amountLabel = planned
    ? t("setup.amountLabelPlanned")
    : t("setup.amountLabelExisting");
  const stepIndex = WIZARD_STEPS.findIndex((step) => step.id === currentStep);
  const goBack = () => {
    const previous = WIZARD_STEPS[Math.max(0, stepIndex - 1)]?.id ?? "setup";
    setCurrentStep(previous);
  };
  const goForward = () => {
    const next = WIZARD_STEPS[Math.min(WIZARD_STEPS.length - 1, stepIndex + 1)]?.id ?? "export";
    setCurrentStep(next);
    if (currentStep === "setup" && next === "review" && selectedTarget) {
      void runSuggestions(false);
    }
  };

  useEffect(() => {
    setSourceFundsDraft(profileKey, {
      target,
      targetAmount,
      reportPurpose,
      plannedDestination,
      plannedNote,
      revealMode,
      diagramDetail,
      selectedRecipientId,
      currentStep,
    });
  }, [
    profileKey,
    setSourceFundsDraft,
    target,
    targetAmount,
    reportPurpose,
    plannedDestination,
    plannedNote,
    revealMode,
    diagramDetail,
    selectedRecipientId,
    currentStep,
  ]);

  useEffect(() => {
    if (!selectedTarget) return;
    setSourceForm((current) =>
      current.to_transaction === selectedTarget
        ? current
        : { ...current, to_transaction: selectedTarget },
    );
    setManualLinkForm((current) =>
      current.to_transaction === selectedTarget
        ? current
        : { ...current, to_transaction: selectedTarget },
    );
  }, [selectedTarget]);

  useEffect(() => {
    if (!selectedLink) {
      if (linkFormSourceId) {
        setLinkFormSourceId("");
      }
      return;
    }
    if (selectedLink.id === linkFormSourceId) {
      return;
    }
    setSelectedLinkId(selectedLink.id);
    setLinkFormSourceId(selectedLink.id);
    setLinkForm({
      link_type: selectedLink.link_type,
      confidence: selectedLink.confidence,
      allocation_amount:
        typeof selectedLink.allocation_amount === "number"
          ? selectedLink.allocation_amount.toFixed(8)
          : "",
      from_allocation_amount:
        typeof selectedLink.from_allocation_amount === "number"
          ? selectedLink.from_allocation_amount.toFixed(8)
          : "",
      explanation: selectedLink.explanation ?? "",
      attachment_id: NO_ATTACHMENT,
    });
  }, [selectedLink, linkFormSourceId]);

  const txName = (id?: string | null) => {
    const row = id ? txById.get(id) : undefined;
    return row ? txLabel(row) : shortId(id);
  };
  const sourceName = (id?: string | null) =>
    sources.find((source) => source.id === id)?.label ?? shortId(id);

  async function runSuggestions(showNotification = true) {
    if (!selectedTarget) return;
    const envelope = await suggestLinks.mutateAsync({
      target_transaction: selectedTarget,
    });
    const inserted = envelope.data?.inserted ?? 0;
    if (showNotification || inserted > 0) {
      addNotification({
        title: showNotification
          ? t("toast.suggestionsUpdated")
          : t("toast.evidenceMatched"),
        body: t("toast.linksFound", { count: inserted }),
        tone: inserted > 0 ? "success" : "info",
      });
    }
  }

  const bulkReviewDeterministicLinks = async () => {
    if (!selectedTarget) return;
    const envelope = await bulkReviewLinks.mutateAsync({
      target_transaction: selectedTarget,
    });
    const reviewed = envelope.data?.reviewed ?? 0;
    const skipped = envelope.data?.skipped ?? 0;
    addNotification({
      title: t("toast.deterministicReviewed"),
      body: t("toast.deterministicBody", { reviewed, skipped }),
      tone: reviewed > 0 ? "success" : "info",
    });
  };

  const reviewSelectedLink = async (state: "reviewed" | "rejected") => {
    if (!selectedLink) return;
    await reviewLink.mutateAsync({
      link: selectedLink.id,
      state,
      link_type: linkForm.link_type,
      confidence: linkForm.confidence,
      allocation_amount: linkForm.allocation_amount || undefined,
      from_allocation_amount: linkForm.from_allocation_amount || undefined,
      allocation_policy: state === "reviewed" ? "explicit" : undefined,
      explanation: linkForm.explanation,
    });
    if (state === "reviewed" && linkForm.attachment_id !== NO_ATTACHMENT) {
      await attachLink.mutateAsync({
        link: selectedLink.id,
        attachment_id: linkForm.attachment_id,
      });
    }
    addNotification({
      title: state === "reviewed" ? t("toast.linkAccepted") : t("toast.linkRejected"),
      body:
        state === "reviewed"
          ? t("toast.linkReviewedBody", {
              type: enumLabel(t, "linkType", linkForm.link_type),
            })
          : t("toast.linkRejectedBody", {
              type: enumLabel(t, "linkType", linkForm.link_type),
            }),
      tone: state === "reviewed" ? "success" : "info",
    });
  };

  const createManualLink = async () => {
    await createLink.mutateAsync({
      from_transaction: manualLinkForm.from_transaction,
      to_transaction: manualLinkForm.to_transaction || selectedTarget,
      link_type: manualLinkForm.link_type,
      state: "reviewed",
      confidence: manualLinkForm.confidence,
      method: "manual",
      allocation_amount: manualLinkForm.allocation_amount,
      from_allocation_amount: manualLinkForm.from_allocation_amount || undefined,
      allocation_policy: "explicit",
      explanation: manualLinkForm.explanation,
      attachment_id:
        manualLinkForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : manualLinkForm.attachment_id,
    });
    setManualLinkForm((current) => ({
      ...current,
      allocation_amount: "",
      from_allocation_amount: "",
      explanation: "",
      attachment_id: NO_ATTACHMENT,
    }));
    addNotification({
      title: t("toast.manualLinkAdded"),
      body: t("toast.manualLinkBody"),
      tone: "success",
    });
  };

  const createSourceLink = async () => {
    const sourceEnvelope = await createSource.mutateAsync({
      source_type: sourceForm.source_type,
      label: sourceForm.label,
      asset: sourceForm.asset,
      amount: sourceForm.amount,
      description: sourceForm.description,
      attachment_id:
        sourceForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : sourceForm.attachment_id,
    });
    if (!sourceEnvelope.data?.id) return;
    await createLink.mutateAsync({
      from_source: sourceEnvelope.data.id,
      to_transaction: sourceForm.to_transaction || selectedTarget,
      link_type: sourceForm.link_type,
      state: "reviewed",
      confidence:
        sourceForm.source_type === "missing_history" ? "unknown" : "strong",
      method: "manual",
      allocation_amount: sourceForm.amount,
      allocation_policy: "explicit",
      explanation: sourceForm.description,
      attachment_id:
        sourceForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : sourceForm.attachment_id,
    });
    setSourceForm((current) => ({
      ...current,
      label: "",
      amount: "",
      description: "",
      attachment_id: NO_ATTACHMENT,
    }));
    addNotification({
      title:
        sourceForm.source_type === "missing_history"
          ? t("toast.gapMarked")
          : t("toast.sourceLinked"),
      body: t("toast.sourcePathBody"),
      tone: "success",
    });
  };

  return (
    <div className={screenShellClassName}>
      <div className="grid gap-4">
        <div className="space-y-4">
          <TracedCoverageHero coverage={coverageQuery.data?.data} />
          <OptionalSection
            open={showCoverage}
            onOpenChange={setShowCoverage}
            icon={<GitBranch className="size-4" aria-hidden="true" />}
            title={t("coverage.sectionTitle")}
            summary={coverageSummary(coverageQuery.data?.data, t)}
          >
            <CoveragePanel
              coverage={coverageQuery.data?.data}
              loading={coverageQuery.isLoading}
            />
          </OptionalSection>
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2">
                <GitBranch className="size-4" aria-hidden="true" />
                {t("header.title")}
              </CardTitle>
              <CardDescription>
                {t("header.description")}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 p-4">
              <WizardProgress currentStep={currentStep} onStep={setCurrentStep} />
              {currentStep === "setup" && (
                <div className="grid gap-3 md:grid-cols-2">
                <PurposeButton
                  active={reportPurpose === "planned_exchange_sale"}
                  title={t("purpose.planned.title")}
                  body={t("purpose.planned.body")}
                  onClick={() => setReportPurpose("planned_exchange_sale")}
                />
                <PurposeButton
                  active={reportPurpose === "existing_transaction"}
                  title={t("purpose.existing.title")}
                  body={t("purpose.existing.body")}
                  onClick={() => setReportPurpose("existing_transaction")}
                />
                </div>
              )}
              {currentStep === "setup" && planned && (
                <div className="rounded-md border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                  {t("purpose.plannedHint")}
                </div>
              )}
              {currentStep === "setup" && (
                <div className="space-y-3">
                  {planned && (
                    <p className="text-xs text-muted-foreground">
                      The sale hasn't happened yet, so pick the existing
                      transaction that currently holds the coins you intend to
                      sell. The report traces history backward from here.
                    </p>
                  )}
                  <div className="grid gap-3 lg:grid-cols-[180px_150px_minmax(0,1fr)]">
                    <ReportControlFields
                      amountLabel={amountLabel}
                      targetAmount={targetAmount}
                      selectedTx={selectedTx}
                      revealMode={revealMode}
                      onAmountChange={setTargetAmount}
                      onRevealModeChange={setRevealMode}
                    />
                    {selectedTx && (
                      <div className="self-end rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                        {t("setup.selected", {
                          date: txDate(selectedTx, t),
                          wallet: txWallet(selectedTx),
                          txid: shortId(
                            selectedTx.external_id ||
                              selectedTx.externalId ||
                              selectedTx.id,
                          ),
                        })}
                      </div>
                    )}
                  </div>
                  <div className="rounded-md border">
                    <div className="flex flex-col gap-3 border-b p-3 lg:flex-row lg:items-center lg:justify-between">
                      <div>
                        <div className="text-sm font-medium">{targetLabel}</div>
                        <div className="text-xs text-muted-foreground">
                          {t("setup.transactionCount", {
                            shown: filteredTargetRows.length,
                            total: rows.length,
                          })}
                        </div>
                      </div>
                      <div className="space-y-3">
                        <div className="flex flex-wrap gap-2">
                          <div className="relative min-w-[220px] flex-1">
                            <Search
                              className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                              aria-hidden="true"
                            />
                            <Input
                              type="search"
                              value={targetSearch}
                              onChange={(event) =>
                                setTargetSearch(event.target.value)
                              }
                              placeholder={t("setup.searchPlaceholder")}
                              className="h-9 pl-9"
                            />
                          </div>
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-9"
                            onClick={() =>
                              setShowAdvancedTargetFilters((open) => !open)
                            }
                            aria-expanded={showAdvancedTargetFilters}
                          >
                            <SlidersHorizontal
                              className="mr-2 size-4"
                              aria-hidden="true"
                            />
                            {t("filters.title")}
                          </Button>
                          {targetFiltersActive && (
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              className="h-9"
                              onClick={clearTargetFilters}
                            >
                              <X className="mr-2 size-4" aria-hidden="true" />
                              {t("filters.clear")}
                            </Button>
                          )}
                        </div>
                        <Collapsible
                          open={showAdvancedTargetFilters}
                          onOpenChange={setShowAdvancedTargetFilters}
                        >
                          <CollapsibleContent className="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm"
                              value={targetDirectionFilter}
                              onChange={(event) =>
                                setTargetDirectionFilter(event.target.value)
                              }
                              aria-label={t("filters.direction.ariaLabel")}
                            >
                              <option value="all">{t("filters.direction.all")}</option>
                              <option value="incoming">{t("flow.incoming")}</option>
                              <option value="outgoing">{t("flow.outgoing")}</option>
                              <option value="transfer">{t("flow.transfer")}</option>
                              <option value="swap">{t("flow.swap")}</option>
                            </select>
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm"
                              value={targetDateFilter}
                              onChange={(event) =>
                                setTargetDateFilter(event.target.value)
                              }
                              aria-label={t("filters.date.ariaLabel")}
                            >
                              <option value="all">{t("filters.date.all")}</option>
                              <option value="today">{t("filters.date.today")}</option>
                              <option value="yesterday">{t("filters.date.yesterday")}</option>
                              <option value="7days">{t("filters.date.last7days")}</option>
                              <option value="30days">{t("filters.date.last30days")}</option>
                              <option value="older">{t("filters.date.older")}</option>
                            </select>
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm"
                              value={targetStatusFilter}
                              onChange={(event) =>
                                setTargetStatusFilter(event.target.value)
                              }
                              aria-label={t("filters.status.ariaLabel")}
                            >
                              <option value="all">{t("filters.status.all")}</option>
                              <option value="confirmed">{t("filters.status.confirmed")}</option>
                              <option value="pending">{t("filters.status.pending")}</option>
                              <option value="review">{t("filters.status.review")}</option>
                            </select>
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm"
                              value={targetNetworkFilter}
                              onChange={(event) =>
                                setTargetNetworkFilter(event.target.value)
                              }
                              aria-label={t("filters.network.ariaLabel")}
                            >
                              <option value="all">{t("filters.network.all")}</option>
                              {targetNetworkOptions.map((network) => (
                                <option key={network} value={network}>
                                  {network}
                                </option>
                              ))}
                            </select>
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm"
                              value={targetAssetFilter}
                              onChange={(event) =>
                                setTargetAssetFilter(event.target.value)
                              }
                              aria-label={t("filters.asset.ariaLabel")}
                            >
                              <option value="all">{t("filters.asset.all")}</option>
                              {targetAssetOptions.map((asset) => (
                                <option key={asset} value={asset}>
                                  {asset}
                                </option>
                              ))}
                            </select>
                            <select
                              className="h-9 rounded-md border bg-background px-3 text-sm xl:col-span-2"
                              value={targetWalletFilter}
                              onChange={(event) =>
                                setTargetWalletFilter(event.target.value)
                              }
                              aria-label={t("filters.wallet.ariaLabel")}
                            >
                              <option value="all">{t("filters.wallet.all")}</option>
                              {targetWalletOptions.map((wallet) => (
                                <option key={wallet} value={wallet}>
                                  {wallet}
                                </option>
                              ))}
                            </select>
                          </CollapsibleContent>
                        </Collapsible>
                      </div>
                    </div>
                    <TransactionTargetHeader />
                    <div className="max-h-[430px] overflow-y-auto p-2">
                      {filteredTargetRows.length === 0 ? (
                        <EmptyState text={t("setup.noMatches")} />
                      ) : (
                        <div className="space-y-2">
                          {filteredTargetRows.map((row) => (
                            <TransactionTargetRow
                              key={txRef(row)}
                              row={row}
                              active={txRef(row) === selectedTarget}
                              onSelect={() => setTarget(txRef(row))}
                              onOpenDetails={() => {
                                setTarget(txRef(row));
                                openTxDetailById(txRef(row));
                              }}
                            />
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}
              {currentStep === "setup" && planned && (
                <div className="grid gap-3 md:grid-cols-[220px_minmax(0,1fr)]">
                  <Field label={t("setup.exchangeBroker.label")} htmlFor="planned-destination">
                    <Input
                      id="planned-destination"
                      value={plannedDestination}
                      onChange={(event) => setPlannedDestination(event.target.value)}
                      placeholder={t("setup.exchangeBroker.placeholder")}
                    />
                  </Field>
                  <Field label={t("setup.bankNote.label")} htmlFor="planned-note">
                    <Input
                      id="planned-note"
                      value={plannedNote}
                      onChange={(event) => setPlannedNote(event.target.value)}
                      placeholder={t("setup.bankNote.placeholder")}
                    />
                  </Field>
                </div>
              )}

              {currentStep === "review" && (
                <CaseBrief
                  report={report}
                  bulkReviewable={bulkReviewableSuggestions.length}
                  manualReview={manualSuggestionCount}
                  onOpenTransaction={openTxDetailById}
                />
              )}

              {currentStep === "review" && (
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void runSuggestions()}
                  disabled={!selectedTarget || suggestLinks.isPending}
                >
                  <RefreshCw className="mr-2 size-4" aria-hidden="true" />
                  {t("actionsBar.findLinks")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void bulkReviewDeterministicLinks()}
                  disabled={
                    !selectedTarget ||
                    bulkReviewLinks.isPending ||
                    bulkReviewableSuggestions.length === 0
                  }
                >
                  <GitBranch className="mr-2 size-4" aria-hidden="true" />
                  {t("actionsBar.reviewDeterministic")}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    setSourceForm((current) => ({
                      ...current,
                      source_type: "missing_history",
                      link_type: "missing_history",
                      label: current.label || t("gapDefaults.label"),
                      amount: current.amount || selectedTargetAmount,
                      description:
                        current.description || t("gapDefaults.description"),
                    }));
                    setShowAdvancedReview(true);
                  }}
                >
                  <AlertTriangle className="mr-2 size-4" aria-hidden="true" />
                  {t("actionsBar.markGap")}
                </Button>
                {manualSuggestionCount > 0 && (
                  <div className="basis-full rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                    {t("actionsBar.manualReviewHint", {
                      count: manualSuggestionCount,
                    })}
                  </div>
                )}
              </div>
              )}

              {currentStep === "export" && (
                <div className="space-y-3">
                  <CaseBrief
                    report={report}
                    bulkReviewable={bulkReviewableSuggestions.length}
                    manualReview={manualSuggestionCount}
                    onOpenTransaction={openTxDetailById}
                  />
                  <RecipientPicker
                    recipients={recipientsQuery.data?.data?.recipients ?? []}
                    selectedRecipientId={selectedRecipientId}
                    onSelectRecipient={(recipient) => {
                      setSelectedRecipientId(recipient?.id ?? "");
                    }}
                  />
                  <RecipientPreferenceAdvisory
                    recipient={selectedRecipient}
                    currentRevealMode={revealMode}
                    onApply={(mode) => setRevealMode(mode)}
                  />
                  <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
                    {t("export.intro")}
                  </div>
                </div>
              )}

              <div className="flex items-center justify-between border-t pt-4">
                <Button
                  type="button"
                  variant="outline"
                  onClick={goBack}
                  disabled={stepIndex === 0}
                >
                  {t("common:actions.back")}
                </Button>
                {currentStep === "export" ? (
                  <Button
                    type="button"
                    disabled={
                      !report?.explain_gates.exportable ||
                      casesSave.isPending ||
                      exportPdf.isPending
                    }
                    onClick={() => {
                      void handleExportPdf();
                    }}
                  >
                    <FileDown className="mr-2 size-4" aria-hidden="true" />
                    {casesSave.isPending
                      ? t("export.savingCase")
                      : t("export.saveAndExport")}
                  </Button>
                ) : (
                  <Button type="button" onClick={goForward} disabled={!selectedTarget}>
                    {t("common:actions.continue")}
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>

          {currentStep === "review" && (
          <OptionalSection
            open={showAdvancedReview}
            onOpenChange={setShowAdvancedReview}
            icon={<SlidersHorizontal className="size-4" aria-hidden="true" />}
            title={t("advancedReview.title")}
            summary={t("advancedReview.summary", {
              links: reviewQueueLinks.length,
              sources: sources.length,
              evidence: evidence.length,
            })}
          >
          <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_420px]">
            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Link2 className="size-4" aria-hidden="true" />
                  {t("reviewQueue.title")}
                </CardTitle>
                <CardDescription>
                  {t("reviewQueue.description")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                {reviewQueueLinks.length === 0 ? (
                  <EmptyState text={t("reviewQueue.empty")} />
                ) : (
                  reviewQueueLinks.map((link) => (
                    <button
                      key={link.id}
                      type="button"
                      className={[
                        "w-full rounded-md border px-3 py-2 text-left text-sm transition-colors",
                        link.id === selectedLink?.id
                          ? "border-primary bg-primary/5"
                          : "hover:bg-muted/60",
                      ].join(" ")}
                      onClick={() => setSelectedLinkId(link.id)}
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <StatusPill state={link.state} />
                        <span className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                          {reachableLinkIds.has(link.id)
                            ? t("reviewQueue.badge.path")
                            : link.to_transaction_id === selectedTxId
                              ? t("reviewQueue.badge.target")
                              : t("reviewQueue.badge.suggested")}
                        </span>
                        <span className="font-medium">
                          {enumLabel(t, "linkType", link.link_type)}
                        </span>
                        <span className="text-muted-foreground">
                          {enumLabel(t, "method", link.method)}
                        </span>
                      </div>
                      <div className="mt-2 grid gap-1 text-xs text-muted-foreground">
                        <span>
                          {t("reviewQueue.arrow", {
                            from: link.from_source_id
                              ? sourceName(link.from_source_id)
                              : txName(link.from_transaction_id),
                            to: txName(link.to_transaction_id),
                          })}
                        </span>
                        <span>
                          {t("reviewQueue.amountConfidence", {
                            amount: formatBtc(link.allocation_amount ?? null, link.asset),
                            confidence: enumLabel(t, "confidence", link.confidence),
                          })}
                        </span>
                      </div>
                    </button>
                  ))
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <FileCheck className="size-4" aria-hidden="true" />
                  {t("linkReview.title")}
                </CardTitle>
                <CardDescription>
                  {t("linkReview.description")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                {!selectedLink ? (
                  <EmptyState text={t("linkReview.empty")} />
                ) : (
                  <>
                    <div className="rounded-md border p-3 text-sm">
                      <div className="font-medium">
                        {selectedSource?.label ??
                          txName(selectedLink.from_transaction_id)}
                      </div>
                      <div className="text-muted-foreground">
                        {t("linkReview.to", {
                          target: txName(selectedLink.to_transaction_id),
                        })}
                      </div>
                    </div>
                    <div className="grid gap-3 sm:grid-cols-2">
                      <SelectField
                        id="review-link-type"
                        label={t("linkReview.type")}
                        value={linkForm.link_type}
                        options={LINK_TYPES}
                        group="linkType"
                        onChange={(value) =>
                          setLinkForm((current) => ({ ...current, link_type: value }))
                        }
                      />
                      <SelectField
                        id="review-confidence"
                        label={t("linkReview.confidence")}
                        value={linkForm.confidence}
                        options={CONFIDENCE_LEVELS}
                        group="confidence"
                        onChange={(value) =>
                          setLinkForm((current) => ({ ...current, confidence: value }))
                        }
                      />
                      <Field label={t("linkReview.allocation")} htmlFor="review-allocation">
                        <Input
                          id="review-allocation"
                          value={linkForm.allocation_amount}
                          onChange={(event) =>
                            setLinkForm((current) => ({
                              ...current,
                              allocation_amount: event.target.value,
                            }))
                          }
                        />
                      </Field>
                      <Field label={t("linkReview.fromAmount")} htmlFor="review-from-allocation">
                        <Input
                          id="review-from-allocation"
                          value={linkForm.from_allocation_amount}
                          onChange={(event) =>
                            setLinkForm((current) => ({
                              ...current,
                              from_allocation_amount: event.target.value,
                            }))
                          }
                        />
                      </Field>
                    </div>
                    <EvidenceSelect
                      id="review-evidence"
                      value={linkForm.attachment_id}
                      evidence={evidence}
                      onChange={(value) =>
                        setLinkForm((current) => ({ ...current, attachment_id: value }))
                      }
                    />
                    <Field label={t("linkReview.reviewNote")} htmlFor="review-note">
                      <Textarea
                        id="review-note"
                        value={linkForm.explanation}
                        onChange={(event) =>
                          setLinkForm((current) => ({
                            ...current,
                            explanation: event.target.value,
                          }))
                        }
                      />
                    </Field>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <Button
                        type="button"
                        onClick={() => void reviewSelectedLink("reviewed")}
                        disabled={reviewLink.isPending || attachLink.isPending}
                      >
                        <Check className="mr-2 size-4" aria-hidden="true" />
                        {t("linkReview.accept")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void reviewSelectedLink("rejected")}
                        disabled={reviewLink.isPending}
                      >
                        <X className="mr-2 size-4" aria-hidden="true" />
                        {t("linkReview.reject")}
                      </Button>
                    </div>
                  </>
                )}
              </CardContent>
            </Card>
          </div>
          <div className="grid gap-4 2xl:grid-cols-2">
            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Plus className="size-4" aria-hidden="true" />
                  {t("sourceOrGap.title")}
                </CardTitle>
                <CardDescription>
                  {t("sourceOrGap.description")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                <div className="grid gap-3 sm:grid-cols-2">
                  <SelectField
                    id="source-type"
                    label={t("sourceOrGap.sourceType")}
                    value={sourceForm.source_type}
                    options={SOURCE_TYPES}
                    group="sourceType"
                    onChange={(value) =>
                      setSourceForm((current) => ({
                        ...current,
                        source_type: value,
                        link_type:
                          value === "missing_history"
                            ? "missing_history"
                            : current.link_type === "missing_history"
                              ? "manual_source"
                              : current.link_type,
                      }))
                    }
                  />
                  <SelectField
                    id="source-link-type"
                    label={t("sourceOrGap.linkType")}
                    value={sourceForm.link_type}
                    options={LINK_TYPES}
                    group="linkType"
                    onChange={(value) =>
                      setSourceForm((current) => ({ ...current, link_type: value }))
                    }
                  />
                  <Field label={t("sourceOrGap.label")} htmlFor="source-label">
                    <Input
                      id="source-label"
                      value={sourceForm.label}
                      onChange={(event) =>
                        setSourceForm((current) => ({
                          ...current,
                          label: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label={t("sourceOrGap.amount")} htmlFor="source-amount">
                    <Input
                      id="source-amount"
                      value={sourceForm.amount}
                      onChange={(event) =>
                        setSourceForm((current) => ({
                          ...current,
                          amount: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label={t("sourceOrGap.asset")} htmlFor="source-asset">
                    <Input
                      id="source-asset"
                      value={sourceForm.asset}
                      onChange={(event) =>
                        setSourceForm((current) => ({
                          ...current,
                          asset: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <TransactionSelect
                    id="source-to"
                    label={t("sourceOrGap.appliesTo")}
                    rows={rows}
                    value={sourceForm.to_transaction || selectedTarget}
                    onChange={(value) =>
                      setSourceForm((current) => ({
                        ...current,
                        to_transaction: value,
                      }))
                    }
                  />
                </div>
                <EvidenceSelect
                  id="source-evidence"
                  value={sourceForm.attachment_id}
                  evidence={evidence}
                  onChange={(value) =>
                    setSourceForm((current) => ({
                      ...current,
                      attachment_id: value,
                    }))
                  }
                />
                <Field label={t("sourceOrGap.evidenceNote")} htmlFor="source-description">
                  <Textarea
                    id="source-description"
                    value={sourceForm.description}
                    onChange={(event) =>
                      setSourceForm((current) => ({
                        ...current,
                        description: event.target.value,
                      }))
                    }
                  />
                </Field>
                <Button
                  type="button"
                  className="w-full"
                  onClick={() => void createSourceLink()}
                  disabled={
                    createSource.isPending ||
                    createLink.isPending ||
                    !sourceForm.label.trim() ||
                    !sourceForm.amount.trim()
                  }
                >
                  <Plus className="mr-2 size-4" aria-hidden="true" />
                  {t("sourceOrGap.create")}
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Link2 className="size-4" aria-hidden="true" />
                  {t("manualLink.title")}
                </CardTitle>
                <CardDescription>
                  {t("manualLink.description")}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                <div className="grid gap-3 sm:grid-cols-2">
                  <TransactionSelect
                    id="manual-from"
                    label={t("manualLink.from")}
                    rows={rows}
                    value={manualLinkForm.from_transaction}
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        from_transaction: value,
                      }))
                    }
                  />
                  <TransactionSelect
                    id="manual-to"
                    label={t("manualLink.to")}
                    rows={rows}
                    value={manualLinkForm.to_transaction || selectedTarget}
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        to_transaction: value,
                      }))
                    }
                  />
                  <SelectField
                    id="manual-type"
                    label={t("manualLink.type")}
                    value={manualLinkForm.link_type}
                    options={LINK_TYPES}
                    group="linkType"
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        link_type: value,
                      }))
                    }
                  />
                  <SelectField
                    id="manual-confidence"
                    label={t("manualLink.confidence")}
                    value={manualLinkForm.confidence}
                    options={CONFIDENCE_LEVELS}
                    group="confidence"
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        confidence: value,
                      }))
                    }
                  />
                  <Field label={t("manualLink.allocation")} htmlFor="manual-allocation">
                    <Input
                      id="manual-allocation"
                      value={manualLinkForm.allocation_amount}
                      onChange={(event) =>
                        setManualLinkForm((current) => ({
                          ...current,
                          allocation_amount: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label={t("manualLink.fromAmount")} htmlFor="manual-from-amount">
                    <Input
                      id="manual-from-amount"
                      value={manualLinkForm.from_allocation_amount}
                      onChange={(event) =>
                        setManualLinkForm((current) => ({
                          ...current,
                          from_allocation_amount: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
                <EvidenceSelect
                  id="manual-evidence"
                  value={manualLinkForm.attachment_id}
                  evidence={evidence}
                  onChange={(value) =>
                    setManualLinkForm((current) => ({
                      ...current,
                      attachment_id: value,
                    }))
                  }
                />
                <Field label={t("manualLink.reviewNote")} htmlFor="manual-note">
                  <Textarea
                    id="manual-note"
                    value={manualLinkForm.explanation}
                    onChange={(event) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        explanation: event.target.value,
                      }))
                    }
                  />
                </Field>
                <Button
                  type="button"
                  className="w-full"
                  onClick={() => void createManualLink()}
                  disabled={
                    createLink.isPending ||
                    !manualLinkForm.from_transaction ||
                    !manualLinkForm.allocation_amount.trim()
                  }
                >
                  <Plus className="mr-2 size-4" aria-hidden="true" />
                  {t("manualLink.add")}
                </Button>
              </CardContent>
            </Card>
          </div>
          </OptionalSection>
          )}
        </div>

        {showStepContext && (
        <div className="space-y-4">
          {(currentStep === "review" || currentStep === "export") && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2 text-base">
                <AlertTriangle className="size-4" aria-hidden="true" />
                {t("gates.title")}
              </CardTitle>
              <CardDescription>
                {t("gates.description")}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-2 p-4">
              {preview.isLoading && <EmptyState text={t("gates.building")} />}
              {preview.isError && (
                <GateRow
                  finding={{
                    code: "preview_unavailable",
                    message: t("gates.previewUnavailable"),
                  }}
                />
              )}
              {[...blockers, ...warnings].map((finding) => (
                <GateRow
                  key={`${finding.code}-${finding.ref ?? ""}-${finding.message}`}
                  finding={finding}
                  onOpenTransaction={
                    finding.ref && txById.has(finding.ref)
                      ? () => openTxDetailById(finding.ref as string)
                      : undefined
                  }
                />
              ))}
              {report && blockers.length === 0 && warnings.length === 0 && (
                <div className="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200">
                  {t("gates.clear")}
                </div>
              )}
            </CardContent>
          </Card>
          )}

          {currentStep === "export" && report?.diagrams?.flow_svg && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">Report visuals</CardTitle>
              <CardDescription>
                Rendered on this device — identical to the exported PDF.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 p-4">
              <Field label="Diagram detail" htmlFor="sof-diagram-detail">
                <select
                  id="sof-diagram-detail"
                  className="h-10 w-full rounded-md border bg-background px-3 text-sm"
                  value={diagramDetail}
                  onChange={(event) =>
                    setDiagramDetail(event.target.value === "detailed" ? "detailed" : "summary")
                  }
                >
                  <option value="summary">Summary — cluster long paths (default)</option>
                  <option value="detailed">Detailed — show more hops before clustering</option>
                </select>
              </Field>
              <Field label="Amount precision" htmlFor="sof-amount-precision">
                <select
                  id="sof-amount-precision"
                  className="h-10 w-full rounded-md border bg-background px-3 text-sm"
                  value={amountPrecision}
                  onChange={(event) =>
                    setAmountPrecision(event.target.value === "sats" ? "sats" : "btc")
                  }
                >
                  <option value="btc">BTC (8 decimals)</option>
                  <option value="sats">Sats (whole numbers)</option>
                </select>
              </Field>
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  className="size-4 rounded border"
                  checked={maskRecipient}
                  onChange={(event) => setMaskRecipient(event.target.checked)}
                />
                Mask recipient label in the report
              </label>
              <div className="space-y-1">
                <div className="text-sm font-medium">Omit sections (leaner report)</div>
                {(
                  [
                    ["flow_levels", "Flow diagram data"],
                    ["transaction_details", "Transaction details"],
                    ["flow_links", "Reviewed flow links"],
                    ["graph_nodes", "Disclosure graph nodes"],
                  ] as const
                ).map(([key, label]) => (
                  <label
                    key={key}
                    className="flex items-center gap-2 text-sm text-muted-foreground"
                  >
                    <input
                      type="checkbox"
                      className="size-4 rounded border"
                      checked={omitSections.includes(key)}
                      onChange={(event) =>
                        setOmitSections((current) =>
                          event.target.checked
                            ? [...current, key]
                            : current.filter((section) => section !== key),
                        )
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
              <ReportDiagram svg={report.diagrams.flow_svg} label="Simplified flow path" />
              <ReportDiagram
                svg={report.diagrams.source_mix_ring_svg}
                label="Source mix"
              />
              <ReportDiagram
                svg={report.diagrams.data_source_ring_svg}
                label="Data sources"
              />
            </CardContent>
          </Card>
          )}

          {currentStep === "export" && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2 text-base">
                <ShieldAlert className="size-4" aria-hidden="true" />
                {t("disclosure.title")}
              </CardTitle>
              <CardDescription>{t("disclosure.description")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 p-4 text-sm">
              <DisclosureNarrative report={report} />
              <DisclosureTxidList report={report} />
              <DisclosureNodeOverrides
                report={report}
                overrides={revealOverrides}
                onChange={(id, decision) =>
                  setRevealOverrides((current) => {
                    const next = { ...current };
                    if (decision) {
                      next[id] = decision;
                    } else {
                      delete next[id];
                    }
                    return next;
                  })
                }
              />
              <DisclosureList
                label={t("disclosure.evidence")}
                values={(report?.disclosure_preview.attachments ?? []).map(
                  (item) => item.label,
                )}
              />
              <DisclosureList
                label={t("disclosure.excluded")}
                values={report?.disclosure_preview.excluded ?? []}
              />
              {report?.disclosure_preview.privacy_note && (
                <p className="rounded-md border px-3 py-2 text-xs text-muted-foreground">
                  {report.disclosure_preview.privacy_note}
                </p>
              )}
              <Button
                className="w-full"
                disabled={
                  !report?.explain_gates.exportable ||
                  casesSave.isPending ||
                  exportPdf.isPending
                }
                onClick={() => {
                  void handleExportPdf();
                }}
              >
                <FileDown className="mr-2 size-4" aria-hidden="true" />
                {casesSave.isPending
                  ? t("export.savingCase")
                  : t("export.saveAndExport")}
              </Button>
              {savedCase && (
                <p className="text-xs text-muted-foreground">
                  {t("export.savedCase", {
                    id: savedCase.id,
                    status: savedCase.status,
                  })}
                </p>
              )}
              {exportedPdf && (
                <p className="text-xs text-muted-foreground">
                  {exportedPdf.filename}
                </p>
              )}
            </CardContent>
          </Card>
          )}

          {currentStep === "export" && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">{t("sourceMix.title")}</CardTitle>
            </CardHeader>
            <CardContent className="p-4">
              <div className="overflow-hidden rounded-md border">
                {(report?.source_mix ?? []).map((source) => (
                  <div
                    key={source.source_type}
                    className="flex items-center justify-between border-b px-3 py-2 text-sm last:border-b-0"
                  >
                    <span>{enumLabel(t, "sourceType", source.source_type)}</span>
                    <span className="font-mono tabular-nums">
                      {formatBtc(source.amount)}
                    </span>
                  </div>
                ))}
                {(report?.source_mix ?? []).length === 0 && (
                  <div className="px-3 py-2 text-sm text-muted-foreground">
                    {t("sourceMix.empty")}
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
          )}

          {currentStep === "export" && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">{t("reviewedFlow.title")}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 p-4">
              {(report?.graph.edges ?? []).map((edge) => {
                const id = stringValue(edge.id) || JSON.stringify(edge);
                const linkType = stringValue(edge.link_type);
                return (
                  <div key={id} className="rounded-md border px-3 py-2 text-sm">
                    <div className="font-medium">
                      {linkType
                        ? enumLabel(t, "linkType", linkType)
                        : t("reviewedFlow.fallbackLinkType")}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatBtc(numberValue(edge.allocation_amount), stringValue(edge.asset) || "BTC")}
                    </div>
                  </div>
                );
              })}
              {(report?.graph.edges ?? []).length === 0 && (
                <EmptyState text={t("reviewedFlow.empty")} />
              )}
            </CardContent>
          </Card>
          )}
        </div>
        )}
      </div>
      <TransactionDetailController
        transaction={detailTransaction}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        onOpenChange={(open) => {
          if (!open) setDetailTransaction(null);
        }}
      />
    </div>
  );
}

function OptionalSection({
  open,
  onOpenChange,
  icon,
  title,
  summary,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  icon: ReactNode;
  title: string;
  summary?: string;
  children: ReactNode;
}) {
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <section className="rounded-md border bg-card">
        <CollapsibleTrigger asChild>
          <button
            type="button"
            className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left"
          >
            <span className="flex min-w-0 items-center gap-2">
              {icon}
              <span className="min-w-0">
                <span className="block text-sm font-semibold">{title}</span>
                {summary && (
                  <span className="block truncate text-xs text-muted-foreground">
                    {summary}
                  </span>
                )}
              </span>
            </span>
            <ChevronDown
              className={[
                "size-4 shrink-0 text-muted-foreground transition-transform",
                open ? "rotate-180" : "",
              ].join(" ")}
              aria-hidden="true"
            />
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="border-t p-4">{children}</div>
        </CollapsibleContent>
      </section>
    </Collapsible>
  );
}

function CaseBrief({
  report,
  bulkReviewable,
  manualReview,
  onOpenTransaction,
}: {
  report?: SourceFundsPreview;
  bulkReviewable: number;
  manualReview: number;
  onOpenTransaction?: (txId: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const overview = report?.overview;
  const targetAsset = overview?.target_asset || report?.target.asset || "BTC";
  const paragraphs = report?.narrative?.paragraphs ?? [];
  const sources = report?.source_mix ?? [];
  const dataSources = report?.data_sources ?? [];
  const context = report?.report_context;
  const jurisdiction = context?.jurisdiction_label;
  const fiatCurrency = context?.fiat_currency;
  return (
    <section className="space-y-4 rounded-md border bg-muted/20 p-4">
      <div className="flex flex-col gap-2 lg:flex-row lg:items-start lg:justify-between">
        <div>
          <h2 className="text-base font-semibold">
            {overview?.target_label || report?.target.label || t("caseBrief.fallbackTarget")}
          </h2>
          <p className="text-sm text-muted-foreground">
            {formatDateTime(overview?.target_date)} ·{" "}
            {overview?.target_wallet || report?.target.wallet || t("caseBrief.fallbackWallet")} ·{" "}
            {formatBtc(overview?.target_amount ?? report?.target.required_amount, targetAsset)}
            {(jurisdiction || fiatCurrency) && (
              <>
                {" "}
                · {[jurisdiction, fiatCurrency].filter(Boolean).join(" / ")}
              </>
            )}
          </p>
        </div>
        <StatusPill
          state={report?.explain_gates.exportable ? "reviewed" : "suggested"}
        />
      </div>
      <div className="grid gap-3 md:grid-cols-5">
        <Metric label={t("caseBrief.metric.transactions")} value={overview?.transaction_count ?? 0} />
        <Metric label={t("caseBrief.metric.reviewedLinks")} value={overview?.link_count ?? 0} />
        <Metric label={t("caseBrief.metric.sources")} value={overview?.source_category_count ?? 0} />
        <Metric label={t("caseBrief.metric.blockers")} value={overview?.blocker_count ?? 0} />
        <Metric label={t("caseBrief.metric.batchable")} value={bulkReviewable} />
      </div>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_340px]">
        <div className="space-y-2">
          {paragraphs.length > 0 ? (
            paragraphs.slice(0, 3).map((paragraph) => (
              <p key={paragraph} className="text-sm text-muted-foreground">
                {paragraph}
              </p>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">
              {t("caseBrief.noPreview")}
            </p>
          )}
          {manualReview > 0 && (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {t("caseBrief.manualReview", { count: manualReview })}
            </p>
          )}
        </div>
        <div className="space-y-2">
          <div className="rounded-md border bg-background">
            {(sources.length > 0 ? sources : [{ source_type: "unresolved", amount: 0, count: 0 }]).map(
              (source) => (
                <div
                  key={source.source_type}
                  className="flex items-center justify-between gap-3 border-b px-3 py-2 text-sm last:border-b-0"
                >
                  <span className="truncate">
                    {enumLabel(t, "sourceType", source.source_type)}
                  </span>
                  <span className="font-mono text-xs tabular-nums">
                    {formatBtc(source.amount, targetAsset)}
                  </span>
                </div>
              ),
            )}
          </div>
          <div className="flex flex-wrap gap-2">
            {dataSources.slice(0, 4).map((source) => (
              <span
                key={`${source.kind}-${source.label}`}
                className="rounded-full border bg-background px-2 py-1 text-xs text-muted-foreground"
              >
                {source.label} · {source.transaction_count + source.source_count}
              </span>
            ))}
            {dataSources.length > 4 && (
              <span className="rounded-full border bg-background px-2 py-1 text-xs text-muted-foreground">
                +{dataSources.length - 4}
              </span>
            )}
          </div>
        </div>
      </div>
      <FlowPathPreview
        flow={report?.simplified_flow}
        onOpenTransaction={onOpenTransaction}
      />
    </section>
  );
}

// Maps the on-device (light, print-matching) diagram palette to a dark-mode
// palette. The frozen SVG stays light so it matches the exported PDF; the app
// recolours it for the dark theme on screen only.
const DARK_SVG_SUBS: ReadonlyArray<readonly [RegExp, string]> = [
  [/#222222/gi, "#e5e7eb"], // ink / text
  [/#666666/gi, "#9ca3af"], // muted text
  [/#d9d9d9/gi, "#3f3f46"], // hairlines
  [/#ffffff/gi, "#09090b"], // surfaces / donut hole / neutral fills
  [/#f7f7f7/gi, "#18181b"], // soft surface
  [/#ecfdf5/gi, "#06281f"], // root-source fill
  [/#16a34a/gi, "#34d399"], // root-source / income
  [/#fffbeb/gi, "#2a1d07"], // attestation fill
  [/#d97706/gi, "#fbbf24"], // attestation / manual
  [/#fff7ed/gi, "#2a1607"], // privacy fill
  [/#ea580c/gi, "#fb923c"], // privacy stroke / edge
  [/#e3000f/gi, "#f87171"], // target / accent
  [/#2563eb/gi, "#60a5fa"], // swap edge / fiat purchase / wallet
  [/#dbeafe/gi, "#1e3a5f"], // swap legend chip
  [/#0ea5e9/gi, "#38bdf8"], // exchange
  [/#65a30d/gi, "#a3e635"], // mining
  [/#a855f7/gi, "#c084fc"], // gift
  [/#0891b2/gi, "#22d3ee"], // blockchain
  [/#6b7280/gi, "#9ca3af"], // unknown
  [/#dc2626/gi, "#f87171"], // fallback red
];

function toDarkSvg(svg: string): string {
  return DARK_SVG_SUBS.reduce((acc, [pattern, color]) => acc.replace(pattern, color), svg);
}

function useIsDark(): boolean {
  const [dark, setDark] = useState(
    () =>
      typeof document !== "undefined" &&
      document.documentElement.classList.contains("dark"),
  );
  useEffect(() => {
    const root = document.documentElement;
    const sync = () => setDark(root.classList.contains("dark"));
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);
  return dark;
}

function ReportDiagram({ svg, label }: { svg?: string; label: string }) {
  const dark = useIsDark();
  if (!svg) {
    return null;
  }
  // Rendered on-device; embedded as a sandboxed <img> so any user-supplied
  // label text in the SVG can never execute as markup. Recoloured for dark mode.
  const themed = dark ? toDarkSvg(svg) : svg;
  const src = `data:image/svg+xml;utf8,${encodeURIComponent(themed)}`;
  return (
    <figure className="space-y-1">
      <img
        src={src}
        alt={label}
        className="w-full rounded-md border bg-white dark:bg-zinc-950"
      />
      <figcaption className="text-xs text-muted-foreground">{label}</figcaption>
    </figure>
  );
}

function FlowPathPreview({
  flow,
  onOpenTransaction,
}: {
  flow?: SourceFundsPreview["simplified_flow"];
  onOpenTransaction?: (txId: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const levels = flow?.levels ?? [];
  if (levels.length === 0) {
    return null;
  }
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <h3 className="text-sm font-semibold">{t("flowPath.title")}</h3>
        {flow?.deferred_privacy_hops?.length ? (
          <span className="rounded-full border border-amber-200 bg-amber-50 px-2 py-0.5 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
            {t("flowPath.privacyHopDeferred")}
          </span>
        ) : null}
      </div>
      {flow?.note && (
        <p className="text-xs text-muted-foreground">{flow.note}</p>
      )}
      <div className="overflow-x-auto pb-1">
        <div className="flex min-w-max items-stretch gap-2">
          {levels.map((level, levelIndex) => {
            const nodes = level.nodes.slice(0, 3);
            const hidden = Math.max(0, level.nodes.length - nodes.length);
            return (
              <div
                key={`${level.role ?? "level"}-${levelIndex}`}
                className="flex items-center gap-2"
              >
                <div className="w-44 rounded-md border bg-background p-2">
                  <div className="mb-2 text-[10px] font-semibold uppercase text-muted-foreground">
                    {level.role ? pretty(level.role) : t("flowPath.fallbackRole")}
                  </div>
                  <div className="space-y-1">
                    {nodes.map((node) => {
                      const clickable =
                        node.node_type === "transaction" &&
                        Boolean(onOpenTransaction);
                      const nodeClassName = [
                        "block w-full rounded border px-2 py-1 text-left",
                        node.deferred_privacy_hop
                          ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100"
                          : level.role === "target"
                            ? "border-primary/35 bg-primary/5"
                            : "bg-muted/25",
                        clickable
                          ? "cursor-pointer transition-colors hover:border-primary/50"
                          : "",
                      ].join(" ");
                      const nodeContent = (
                        <>
                          <div className="truncate text-xs font-medium">
                            {node.label || node.id}
                          </div>
                          <div className="mt-0.5 truncate text-[11px] text-muted-foreground">
                            {pretty(node.kind || node.node_type || "")}
                            {node.amount != null
                              ? ` · ${formatBtc(node.amount, node.asset || "BTC")}`
                              : ""}
                          </div>
                        </>
                      );
                      return clickable ? (
                        <button
                          key={node.id}
                          type="button"
                          className={nodeClassName}
                          onClick={() => onOpenTransaction?.(node.id)}
                          title="Open transaction details"
                        >
                          {nodeContent}
                        </button>
                      ) : (
                        <div key={node.id} className={nodeClassName}>
                          {nodeContent}
                        </div>
                      );
                    })}
                    {hidden > 0 && (
                      <div className="rounded border border-dashed px-2 py-1 text-xs text-muted-foreground">
                        {t("flowPath.more", { count: hidden })}
                      </div>
                    )}
                  </div>
                </div>
                {levelIndex < levels.length - 1 && (
                  <ArrowRight
                    className="size-4 shrink-0 text-muted-foreground"
                    aria-hidden="true"
                  />
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 font-mono text-lg font-semibold tabular-nums">
        {value.toLocaleString("en-US")}
      </div>
    </div>
  );
}

function WizardProgress({
  currentStep,
  onStep,
}: {
  currentStep: WizardStep;
  onStep: (step: WizardStep) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const currentIndex = WIZARD_STEPS.findIndex((step) => step.id === currentStep);
  return (
    <nav className="flex items-center" aria-label="Progress">
      {WIZARD_STEPS.map((step, index) => {
        const active = step.id === currentStep;
        const done = index < currentIndex;
        const isLast = index === WIZARD_STEPS.length - 1;
        return (
          <div
            key={step.id}
            className={isLast ? "flex items-center" : "flex flex-1 items-center"}
          >
            <button
              type="button"
              onClick={() => onStep(step.id)}
              aria-current={active ? "step" : undefined}
              className="flex items-center gap-2 rounded-full text-left"
            >
              <span
                className={[
                  "flex size-7 shrink-0 items-center justify-center rounded-full border text-xs font-semibold transition-colors",
                  active
                    ? "border-primary bg-primary text-primary-foreground"
                    : done
                      ? "border-primary/40 bg-primary/15 text-primary"
                      : "border-border bg-muted text-muted-foreground",
                ].join(" ")}
              >
                {index + 1}
              </span>
              <span
                className={[
                  "text-sm font-medium transition-colors",
                  active ? "text-foreground" : "text-muted-foreground",
                ].join(" ")}
              >
                {t(`wizard.steps.${step.id}`)}
              </span>
            </button>
            {!isLast && (
              <span
                aria-hidden="true"
                className={[
                  "mx-3 h-px flex-1 transition-colors",
                  index < currentIndex ? "bg-primary/40" : "bg-border",
                ].join(" ")}
              />
            )}
          </div>
        );
      })}
    </nav>
  );
}

function PurposeButton({
  active,
  title,
  body,
  onClick,
}: {
  active: boolean;
  title: string;
  body: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      className={[
        "rounded-md border px-3 py-3 text-left transition-colors",
        active ? "border-primary bg-primary/5" : "hover:bg-muted/60",
      ].join(" ")}
      onClick={onClick}
    >
      <span className="block text-sm font-semibold">{title}</span>
      <span className="mt-1 block text-xs text-muted-foreground">{body}</span>
    </button>
  );
}

function Field({
  label,
  htmlFor,
  children,
}: {
  label: string;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={htmlFor}>{label}</Label>
      {children}
    </div>
  );
}

function SelectField({
  id,
  label,
  value,
  options,
  group,
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  options: string[];
  group?: "sourceType" | "linkType" | "confidence" | "reveal";
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <Field label={label} htmlFor={id}>
      <select
        id={id}
        className="h-10 w-full rounded-md border bg-background px-3 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option} value={option}>
            {group ? enumLabel(t, group, option) : pretty(option)}
          </option>
        ))}
      </select>
    </Field>
  );
}

function TransactionSelect({
  id,
  label,
  rows,
  value,
  onChange,
}: {
  id: string;
  label: string;
  rows: TransactionRow[];
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <Field label={label} htmlFor={id}>
      <select
        id={id}
        className="h-10 w-full rounded-md border bg-background px-3 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value="">{t("select.transaction")}</option>
        {rows.map((row) => (
          <option key={txRef(row)} value={txRef(row)}>
            {txLabel(row)}
          </option>
        ))}
      </select>
    </Field>
  );
}

function EvidenceSelect({
  id,
  value,
  evidence,
  onChange,
}: {
  id: string;
  value: string;
  evidence: EvidenceAttachment[];
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  return (
    <Field label={t("evidence.label")} htmlFor={id}>
      <select
        id={id}
        className="h-10 w-full rounded-md border bg-background px-3 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value={NO_ATTACHMENT}>{t("select.noAttachment")}</option>
        {evidence.map((item) => (
          <option key={item.id} value={item.id}>
            {[item.label, item.wallet, item.external_id].filter(Boolean).join(" · ")}
          </option>
        ))}
      </select>
    </Field>
  );
}

function StatusPill({ state }: { state: string }) {
  const { t } = useTranslation("sourceFunds");
  const className =
    state === "reviewed"
      ? "border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200"
      : state === "rejected"
        ? "border-muted bg-muted text-muted-foreground"
        : "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200";
  return (
    <span className={`rounded-full border px-2 py-0.5 text-xs ${className}`}>
      {enumLabel(t, "linkState", state)}
    </span>
  );
}

function isBulkReviewableLink(link: SourceFundsLink) {
  const method = link.method || "";
  const deterministic = BULK_REVIEWABLE_METHODS.has(method);
  return (
    link.state === "suggested" &&
    deterministic &&
    (link.confidence === "exact" || link.confidence === "strong") &&
    typeof link.allocation_amount === "number" &&
    !link.uses_chain_observation
  );
}

const COVERAGE_BUCKET_ORDER: (keyof SourceFundsCoverageBuckets)[] = [
  "fully_traced",
  "attested",
  "in_review",
  "untraced",
  "not_classified",
];

const COVERAGE_BUCKET_LABEL_KEYS: Record<
  keyof SourceFundsCoverageBuckets,
  | "coverage.bucket.fullyTraced"
  | "coverage.bucket.attested"
  | "coverage.bucket.inReview"
  | "coverage.bucket.untraced"
  | "coverage.bucket.notClassified"
> = {
  fully_traced: "coverage.bucket.fullyTraced",
  attested: "coverage.bucket.attested",
  in_review: "coverage.bucket.inReview",
  untraced: "coverage.bucket.untraced",
  not_classified: "coverage.bucket.notClassified",
};

const COVERAGE_BUCKET_TONES: Record<keyof SourceFundsCoverageBuckets, string> = {
  fully_traced: "text-emerald-700 dark:text-emerald-300",
  attested: "text-sky-700 dark:text-sky-300",
  in_review: "text-amber-700 dark:text-amber-300",
  untraced: "text-rose-700 dark:text-rose-300",
  not_classified: "text-muted-foreground",
};

function coverageSummary(
  coverage: SourceFundsCoverage | undefined,
  t: SourceFundsTFunction,
) {
  if (!coverage || coverage.totals.tx_count === 0) {
    return t("coverage.summaryEmpty");
  }
  const traced = coverage.totals.buckets.fully_traced.amount;
  return t("coverage.summary", {
    amount: traced.toFixed(8),
    count: coverage.totals.tx_count,
  });
}

const COVERAGE_BUCKET_BARS: Record<keyof SourceFundsCoverageBuckets, string> = {
  fully_traced: "bg-emerald-500",
  attested: "bg-sky-500",
  in_review: "bg-amber-500",
  untraced: "bg-rose-500",
  not_classified: "bg-muted-foreground/40",
};

function TracedCoverageHero({ coverage }: { coverage?: SourceFundsCoverage }) {
  const totals = coverage?.totals;
  const total = totals?.amount ?? 0;
  const txCount = totals?.tx_count ?? 0;
  const buckets = totals?.buckets;
  if (!coverage || txCount === 0) {
    return null;
  }
  const pct = (name: keyof SourceFundsCoverageBuckets) =>
    total > 0 ? ((buckets?.[name]?.amount ?? 0) / total) * 100 : 0;
  return (
    <Card>
      <CardContent className="p-4">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <div className="text-xs uppercase tracking-wide text-muted-foreground">
              Inbound history traced
            </div>
            <div className="mt-0.5 flex items-baseline gap-2">
              <span className="font-mono text-3xl font-semibold tabular-nums text-emerald-700 dark:text-emerald-300">
                {pct("fully_traced").toFixed(1)}%
              </span>
              <span className="text-sm text-muted-foreground">
                fully traced · {txCount} inbound tx
              </span>
            </div>
          </div>
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
            {COVERAGE_BUCKET_ORDER.filter(
              (name) => name === "fully_traced" || (buckets?.[name]?.amount ?? 0) > 0,
            ).map((name) => (
              <span key={name} className="inline-flex items-center gap-1.5">
                <span className={`size-2.5 rounded-sm ${COVERAGE_BUCKET_BARS[name]}`} />
                <span className="text-muted-foreground">
                  {COVERAGE_BUCKET_LABELS[name]}
                </span>
                <span className={`font-medium ${COVERAGE_BUCKET_TONES[name]}`}>
                  {pct(name).toFixed(1)}%
                </span>
              </span>
            ))}
          </div>
        </div>
        <div className="mt-3 flex h-2.5 w-full overflow-hidden rounded-full bg-muted">
          {COVERAGE_BUCKET_ORDER.map((name) => {
            const percent = pct(name);
            return percent > 0 ? (
              <div
                key={name}
                className={COVERAGE_BUCKET_BARS[name]}
                style={{ width: `${percent}%` }}
                title={`${COVERAGE_BUCKET_LABELS[name]}: ${percent.toFixed(1)}%`}
              />
            ) : null;
          })}
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          Attested ({pct("attested").toFixed(1)}%) is prior-history attestation,
          shown separately — not counted as fully traced.
        </p>
        {coverage.truncation?.truncated && (
          <p className="mt-1 text-xs text-amber-700 dark:text-amber-300">
            Partial: {txCount} of {coverage.truncation.inbound_total_count} inbound
            transactions classified.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function CoveragePanel({
  coverage,
  loading,
}: {
  coverage?: SourceFundsCoverage;
  loading?: boolean;
}) {
  const { t } = useTranslation("sourceFunds");
  const totals = coverage?.totals;
  const totalAmount = totals?.amount ?? 0;
  const totalTxCount = totals?.tx_count ?? 0;
  const buckets = totals?.buckets;
  const denominator = totalAmount > 0 ? totalAmount : 0;
  return (
    <section>
        {loading && !coverage ? (
          <EmptyState text={t("coverage.computing")} />
        ) : !coverage || totalTxCount === 0 ? (
          <EmptyState text={t("coverage.noInbound")} />
        ) : (
          <>
            {coverage.truncation?.truncated && (
              <div className="mb-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100">
                <Trans
                  t={t}
                  i18nKey="coverage.truncated"
                  values={{
                    shown: totalTxCount,
                    total: coverage.truncation.inbound_total_count,
                    notClassified: coverage.truncation.not_classified_count,
                  }}
                  components={[<code className="text-[10px]" />]}
                />
              </div>
            )}
            <div className="grid gap-4 md:grid-cols-5">
              {COVERAGE_BUCKET_ORDER.map((name) => {
                const bucket = buckets?.[name];
                const amount = bucket?.amount ?? 0;
                const txCount = bucket?.tx_count ?? 0;
                const percent = denominator > 0 ? (amount / denominator) * 100 : 0;
                return (
                  <div key={name} className="space-y-1">
                    <div className="text-xs uppercase tracking-wide opacity-70">
                      {t(COVERAGE_BUCKET_LABEL_KEYS[name])}
                    </div>
                    <div className={`text-lg font-semibold ${COVERAGE_BUCKET_TONES[name]}`}>
                      {amount.toFixed(8)}
                    </div>
                    <div className="text-xs opacity-80">
                      {t("coverage.bucketStats", {
                        count: txCount,
                        percent: percent.toFixed(1),
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}
    </section>
  );
}

function RecipientPicker({
  recipients,
  selectedRecipientId,
  onSelectRecipient,
}: {
  recipients: SourceFundsRecipient[];
  selectedRecipientId: string;
  onSelectRecipient: (recipient: SourceFundsRecipient | null) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  if (recipients.length === 0) {
    return (
      <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
        <Trans
          t={t}
          i18nKey="recipient.empty"
          components={[<code className="text-xs" />]}
        />
      </div>
    );
  }
  const selected = recipients.find((r) => r.id === selectedRecipientId) ?? null;
  return (
    <div className="rounded-md border px-3 py-3 text-sm">
      <div className="mb-1 font-medium">{t("recipient.title")}</div>
      <div className="mb-2 text-xs text-muted-foreground">
        {t("recipient.hint")}
      </div>
      <select
        className="h-9 w-full rounded-md border bg-background px-3 text-sm"
        value={selectedRecipientId}
        onChange={(event) => {
          const next = recipients.find((r) => r.id === event.target.value) ?? null;
          if (next && next.active === false) return;
          onSelectRecipient(next);
        }}
        aria-label={t("recipient.ariaLabel")}
      >
        <option value="">{t("recipient.none")}</option>
        {recipients.map((recipient) => {
          const inactive = recipient.active === false;
          return (
            <option key={recipient.id} value={recipient.id} disabled={inactive}>
              {t("recipient.option", {
                label: recipient.label,
                kind: pretty(recipient.kind),
                reveal: enumLabel(t, "reveal", recipient.default_reveal_mode),
              })}
              {inactive ? t("recipient.inactiveSuffix") : ""}
            </option>
          );
        })}
      </select>
      {selected && selected.notes && (
        <div className="mt-2 text-xs opacity-80">{selected.notes}</div>
      )}
    </div>
  );
}

function RecipientPreferenceAdvisory({
  recipient,
  currentRevealMode,
  onApply,
}: {
  recipient: SourceFundsRecipient | null;
  currentRevealMode: string;
  onApply: (mode: string) => void;
}) {
  const { t } = useTranslation("sourceFunds");
  if (!recipient) return null;
  const preferred = recipient.default_reveal_mode;
  if (!preferred || preferred === currentRevealMode) return null;
  return (
    <div className="flex items-center justify-between gap-3 rounded-md border bg-muted/30 px-3 py-2 text-xs">
      <span className="text-muted-foreground">
        <Trans
          t={t}
          i18nKey="recipient.advisory"
          values={{
            label: recipient.label,
            preferred: enumLabel(t, "reveal", preferred),
            current: enumLabel(t, "reveal", currentRevealMode),
          }}
          components={[
            <span className="font-medium text-foreground" />,
            <span className="font-medium text-foreground" />,
          ]}
        />
      </span>
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => onApply(preferred)}
      >
        {t("recipient.applyPreference")}
      </Button>
    </div>
  );
}

function GateRow({
  finding,
  onOpenTransaction,
}: {
  finding: SourceFundsFinding;
  onOpenTransaction?: () => void;
}) {
  const { t } = useTranslation("sourceFunds");
  const blocker = finding.severity === "blocker";
  const headline = finding.next_step?.headline?.trim();
  const docAnchor = finding.next_step?.doc_anchor?.trim();
  return (
    <div
      className={[
        "rounded-md border px-3 py-2 text-sm",
        blocker
          ? "border-amber-200 bg-amber-50 text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100"
          : "",
      ].join(" ")}
    >
      <div className="font-medium">{pretty(finding.code)}</div>
      <div className="mt-1 text-xs opacity-80">{finding.message}</div>
      {headline && (
        <div className="mt-2 text-xs font-medium opacity-90">
          {t("gates.nextStep", { headline })}
          {docAnchor && (
            <span className="ml-1 opacity-70">
              {t("gates.seeDocs", { anchor: docAnchor })}
            </span>
          )}
        </div>
      )}
      {onOpenTransaction && (
        <button
          type="button"
          className="mt-2 inline-flex items-center gap-1 text-xs font-medium text-[var(--color-accent)] hover:underline"
          onClick={onOpenTransaction}
        >
          <Eye className="size-3.5" aria-hidden="true" />
          Open transaction to fix
        </button>
      )}
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="rounded-md border px-3 py-6 text-center text-sm text-muted-foreground">
      {text}
    </div>
  );
}

function DisclosureNodeOverrides({
  report,
  overrides,
  onChange,
}: {
  report?: SourceFundsPreview;
  overrides: Record<string, "show" | "hide">;
  onChange: (id: string, decision: "show" | "hide" | undefined) => void;
}) {
  const nodes = (report?.graph.nodes ?? []).filter(
    (node) => stringValue(node.node_type) === "transaction",
  );
  if (nodes.length === 0) {
    return null;
  }
  const buttonClass = (active: boolean, tone: "show" | "hide") =>
    [
      "rounded border px-2 py-0.5 text-[11px] transition-colors",
      active
        ? tone === "show"
          ? "border-emerald-500 bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300"
          : "border-rose-500 bg-rose-50 text-rose-700 dark:bg-rose-950/40 dark:text-rose-300"
        : "text-muted-foreground hover:bg-muted/50",
    ].join(" ");
  return (
    <div className="space-y-1">
      <div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        Per-transaction disclosure
      </div>
      <p className="text-[11px] text-muted-foreground">
        Override the reveal mode for individual transactions. Changes update the
        preview live and freeze into the exported case.
      </p>
      <div className="space-y-1">
        {nodes.map((node) => {
          const id = stringValue(node.transaction_id);
          if (!id) {
            return null;
          }
          const external = stringValue(node.external_id);
          const decision = overrides[id];
          return (
            <div
              key={id}
              className="flex items-center justify-between gap-2 rounded-md border px-2 py-1"
            >
              <div className="min-w-0">
                <div className="truncate text-xs">
                  {stringValue(node.label) || id}
                </div>
                <div className="truncate font-mono text-[10px] text-muted-foreground">
                  {external ? shortId(external) : "(redacted)"}
                </div>
              </div>
              <div className="flex shrink-0 gap-1">
                <button
                  type="button"
                  className={buttonClass(decision === "show", "show")}
                  onClick={() =>
                    onChange(id, decision === "show" ? undefined : "show")
                  }
                >
                  Show
                </button>
                <button
                  type="button"
                  className={buttonClass(decision === "hide", "hide")}
                  onClick={() =>
                    onChange(id, decision === "hide" ? undefined : "hide")
                  }
                >
                  Hide
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DisclosureNarrative({ report }: { report?: SourceFundsPreview }) {
  const { t } = useTranslation("sourceFunds");
  const txidCount = report?.disclosure_preview.txids.length ?? 0;
  const evidenceCount = report?.disclosure_preview.attachments.length ?? 0;
  const hiddenCount = report?.disclosure_preview.excluded.length ?? 0;
  const sourceCount = report?.source_mix.length ?? 0;
  const reviewedLinkCount = report?.graph.edges.length ?? 0;
  const walletLabels = uniqueSorted(
    (report?.graph.nodes ?? [])
      .map((node) => stringValue(node.wallet))
      .filter(Boolean),
  );
  const targetLabel = report?.target.label || t("disclosure.narrativeTarget");
  const purposeLabel = report?.purpose?.label || t("disclosure.narrativePurpose");
  const revealMode = enumLabel(t, "reveal", report?.reveal_mode || "standard");

  return (
    <section className="space-y-3 rounded-md border bg-muted/20 p-4">
      <div className="space-y-1">
        <h2 className="text-base font-semibold">{t("disclosure.summaryTitle")}</h2>
        <p className="text-sm text-muted-foreground">
          {t("disclosure.narrative", {
            purpose: purposeLabel,
            target: targetLabel,
            txids: t("disclosure.txidCount", { count: txidCount }),
            evidence: t("disclosure.evidenceCount", { count: evidenceCount }),
            links: t("disclosure.linkCount", { count: reviewedLinkCount }),
            sources: t("disclosure.sourceCount", { count: sourceCount }),
          })}
        </p>
      </div>

      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
        <DisclosureMetric label={t("disclosure.metric.txids")} value={txidCount} />
        <DisclosureMetric label={t("disclosure.metric.evidence")} value={evidenceCount} />
        <DisclosureMetric label={t("disclosure.metric.reviewedLinks")} value={reviewedLinkCount} />
        <DisclosureMetric label={t("disclosure.metric.sources")} value={sourceCount} />
        <DisclosureMetric label={t("disclosure.metric.hidden")} value={hiddenCount} />
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("disclosure.revealMode")}
          </div>
          <div className="mt-1 font-medium">{revealMode}</div>
          <p className="mt-1 text-xs text-muted-foreground">
            {report?.disclosure_preview.privacy_note || t("disclosure.noPreview")}
          </p>
        </div>
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            {t("disclosure.walletLabels")}
          </div>
          <div className="mt-1 text-sm">
            {walletLabels.length > 0
              ? walletLabels.join(", ")
              : t("disclosure.walletLabelsNone")}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("disclosure.walletPrivacyNote")}
          </p>
        </div>
      </div>
    </section>
  );
}

function DisclosureMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-md border bg-background px-3 py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className="mt-1 font-mono text-lg font-semibold tabular-nums">
        {value.toLocaleString("en-US")}
      </div>
    </div>
  );
}

function DisclosureTxidList({ report }: { report?: SourceFundsPreview }) {
  const { t } = useTranslation("sourceFunds");
  const [openingTxid, setOpeningTxid] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const txids = report?.disclosure_preview.txids ?? [];
  const links = useMemo(
    () =>
      new Map(
        (report?.disclosure_preview.explorer_links ?? []).map((link) => [
          link.txid,
          link,
        ]),
      ),
    [report?.disclosure_preview.explorer_links],
  );
  const onOpen = async (txid: string, url: string) => {
    setOpenError(null);
    setOpeningTxid(txid);
    try {
      await openExternalUrl(url);
    } catch (error) {
      setOpenError(
        error instanceof Error && error.message
          ? error.message
          : t("disclosure.openError"),
      );
    } finally {
      setOpeningTxid(null);
    }
  };
  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold">{t("disclosure.txids")}</h2>
      <div className="space-y-1">
        {txids.length === 0 ? (
          <div className="rounded-md border px-3 py-2 text-muted-foreground">
            {t("disclosure.none")}
          </div>
        ) : (
          txids.map((txid) => {
            const link = links.get(txid);
            return (
              <div
                key={txid}
                className="flex flex-col gap-2 rounded-md border px-3 py-2 text-xs sm:flex-row sm:items-center sm:justify-between"
              >
                <span className="break-all font-mono">{txid}</span>
                {link ? (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    className="h-8 shrink-0"
                    disabled={openingTxid === txid}
                    onClick={() => void onOpen(txid, link.url)}
                    title={t("disclosure.openTitle", { txid, label: link.label })}
                  >
                    <ExternalLink className="mr-2 size-3.5" aria-hidden="true" />
                    {openingTxid === txid ? t("disclosure.opening") : link.label}
                  </Button>
                ) : (
                  <span className="text-muted-foreground">
                    {t("disclosure.noExplorerLink")}
                  </span>
                )}
              </div>
            );
          })
        )}
      </div>
      {openError && (
        <p
          role="alert"
          className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {openError}
        </p>
      )}
    </section>
  );
}

function DisclosureList({ label, values }: { label: string; values: string[] }) {
  const { t } = useTranslation("sourceFunds");
  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold">{label}</h2>
      <div className="space-y-1">
        {values.length === 0 ? (
          <div className="rounded-md border px-3 py-2 text-muted-foreground">
            {t("disclosure.none")}
          </div>
        ) : (
          values.map((value) => (
            <div
              key={value}
              className="break-all rounded-md border px-3 py-2 font-mono text-xs"
            >
              {value}
            </div>
          ))
        )}
      </div>
    </section>
  );
}
