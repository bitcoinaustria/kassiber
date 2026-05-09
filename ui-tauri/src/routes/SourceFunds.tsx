import {
  AlertTriangle,
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  Check,
  FileCheck,
  FileDown,
  GitBranch,
  Link2,
  Plus,
  RefreshCw,
  Search,
  ShieldAlert,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
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

type SourceFundsFinding = {
  code: string;
  severity?: string;
  message: string;
  ref?: string;
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
    asset: string;
    required_amount: number;
    external_id?: string;
  };
  reveal_mode: string;
  graph: {
    nodes: Record<string, unknown>[];
    edges: Record<string, unknown>[];
  };
  source_mix: { source_type: string; amount: number; count: number }[];
  findings: SourceFundsFinding[];
  explain_gates: {
    exportable: boolean;
    blockers: SourceFundsFinding[];
    warnings: SourceFundsFinding[];
  };
  disclosure_preview: {
    txids: string[];
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
  chain_data_confirmed?: boolean;
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
  { id: "purpose", label: "Purpose" },
  { id: "anchor", label: "Funds" },
  { id: "review", label: "Evidence" },
  { id: "disclosure", label: "Disclosure" },
  { id: "export", label: "Export" },
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

function txFlowLabel(row: TransactionRow): string {
  const flow = txFlow(row);
  if (flow === "incoming") return "Incoming";
  if (flow === "outgoing") return "Outgoing";
  if (flow === "swap") return "Swap";
  return "Transfer";
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

function txDate(row: TransactionRow): string {
  return (row.occurred_at || row.date || "").slice(0, 10) || "Unknown date";
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

function pretty(value: string) {
  return value.replaceAll("_", " ");
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
  return (
    <>
      <Field label={amountLabel} htmlFor="sof-amount">
        <Input
          id="sof-amount"
          value={targetAmount}
          onChange={(event) => onAmountChange(event.target.value)}
          placeholder={selectedTx ? txAmount(selectedTx) : "0.00000000"}
        />
      </Field>
      <Field label="Reveal" htmlFor="sof-reveal">
        <select
          id="sof-reveal"
          className="h-10 w-full rounded-md border bg-background px-3 text-sm"
          value={revealMode}
          onChange={(event) => onRevealModeChange(event.target.value)}
        >
          {REVEAL_MODES.map((mode) => (
            <option key={mode} value={mode}>
              {pretty(mode)}
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
}: {
  row: TransactionRow;
  active: boolean;
  onSelect: () => void;
}) {
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
  const description = row.counter || row.description || row.note || txid || "Transaction";

  return (
    <button
      type="button"
      className={[
        "w-full rounded-md border px-3 py-2 text-left transition-colors",
        active ? "border-primary bg-primary/5" : "hover:bg-muted/45",
      ].join(" ")}
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
                <span className="md:hidden">{pretty(txDirection(row))}</span>
              )}
            </div>
          </div>
        </div>
        <div className={`font-mono text-sm tabular-nums md:text-right ${amountClassName}`}>
          {txSignedAmount(row)}
        </div>
        <div className="text-sm text-muted-foreground">
          <span className="md:hidden">Wallet: </span>
          {txWallet(row)}
        </div>
        <div className="flex flex-wrap items-center gap-2 md:justify-end">
          <span className={`inline-flex items-center gap-1 rounded-md border px-2 py-1 text-xs font-medium ${flowClassName}`}>
            <FlowIcon className="size-3.5" aria-hidden="true" />
            {txFlowLabel(row)}
          </span>
          <span className="text-xs text-muted-foreground">
            {txDate(row)}
          </span>
        </div>
      </div>
    </button>
  );
}

function TransactionTargetHeader() {
  return (
    <div className="hidden border-b bg-muted/35 px-5 py-2 text-xs font-medium text-muted-foreground md:grid md:grid-cols-[minmax(0,1fr)_140px_150px_130px]">
      <span>Transaction</span>
      <span className="text-right">Amount</span>
      <span>Wallet</span>
      <span className="text-right">Flow</span>
    </div>
  );
}

export function SourceFunds() {
  const addNotification = useUiStore((state) => state.addNotification);
  const [currentStep, setCurrentStep] = useState<WizardStep>("purpose");
  const [reportPurpose, setReportPurpose] = useState<
    "planned_exchange_sale" | "existing_transaction"
  >("planned_exchange_sale");
  const [target, setTarget] = useState("");
  const [targetAmount, setTargetAmount] = useState("");
  const [targetSearch, setTargetSearch] = useState("");
  const [targetDirectionFilter, setTargetDirectionFilter] = useState("all");
  const [targetDateFilter, setTargetDateFilter] = useState("all");
  const [targetStatusFilter, setTargetStatusFilter] = useState("all");
  const [targetNetworkFilter, setTargetNetworkFilter] = useState("all");
  const [targetAssetFilter, setTargetAssetFilter] = useState("all");
  const [targetWalletFilter, setTargetWalletFilter] = useState("all");
  const [plannedDestination, setPlannedDestination] = useState("");
  const [plannedNote, setPlannedNote] = useState("");
  const [revealMode, setRevealMode] = useState("standard");
  const [selectedLinkId, setSelectedLinkId] = useState("");
  const [linkForm, setLinkForm] = useState({
    link_type: "self_transfer",
    confidence: "strong",
    allocation_amount: "",
    from_allocation_amount: "",
    explanation: "",
    attachment_id: NO_ATTACHMENT,
  });
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
  const exportPdf = useDaemonMutation("ui.source_funds.export_pdf");

  const report = preview.data?.data;
  const links = linksQuery.data?.data?.links ?? [];
  const sources = sourcesQuery.data?.data?.sources ?? [];
  const evidence = evidenceQuery.data?.data?.attachments ?? [];
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
  const bulkReviewableSuggestions = links.filter(isBulkReviewableLink);
  const manualSuggestionCount = links.filter(
    (link) => link.state === "suggested" && !isBulkReviewableLink(link),
  ).length;
  const exportedPdf = exportPdf.data?.data as { filename?: string } | undefined;
  const planned = reportPurpose === "planned_exchange_sale";
  const showStepContext =
    currentStep === "review" ||
    currentStep === "disclosure" ||
    currentStep === "export";
  const targetLabel = planned ? "Funds history anchor" : "Completed transaction";
  const amountLabel = planned ? "Planned sale amount" : "Report amount";
  const stepIndex = WIZARD_STEPS.findIndex((step) => step.id === currentStep);
  const goBack = () => {
    const previous = WIZARD_STEPS[Math.max(0, stepIndex - 1)]?.id ?? "purpose";
    setCurrentStep(previous);
  };
  const goForward = () => {
    const next = WIZARD_STEPS[Math.min(WIZARD_STEPS.length - 1, stepIndex + 1)]?.id ?? "export";
    setCurrentStep(next);
    if (currentStep === "anchor" && next === "review" && selectedTarget) {
      void runSuggestions(false);
    }
  };

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
    if (!selectedLink) return;
    setSelectedLinkId(selectedLink.id);
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
  }, [selectedLink?.id]);

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
        title: showNotification ? "Suggestions updated" : "Evidence matched",
        body: `${inserted} new source-funds link${inserted === 1 ? "" : "s"}.`,
        tone: inserted > 0 ? "success" : "info",
      });
    }
  }

  const bulkReviewDeterministicLinks = async () => {
    const envelope = await bulkReviewLinks.mutateAsync({});
    const reviewed = envelope.data?.reviewed ?? 0;
    const skipped = envelope.data?.skipped ?? 0;
    addNotification({
      title: "Deterministic hops reviewed",
      body: `${reviewed} reviewed, ${skipped} left for manual review.`,
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
      title: state === "reviewed" ? "Link accepted" : "Link rejected",
      body: `${pretty(linkForm.link_type)} ${state}.`,
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
      title: "Manual link added",
      body: "The reviewed flow has been updated.",
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
          ? "Gap marked reviewed"
          : "Source linked",
      body: "The source-funds path has been updated.",
      tone: "success",
    });
  };

  return (
    <div className={screenShellClassName}>
      <div className="grid gap-4">
        <div className="space-y-4">
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2">
                <GitBranch className="size-4" aria-hidden="true" />
                Source of Funds
              </CardTitle>
              <CardDescription>
                Pick the purpose first, then review the evidence path.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 p-4">
              <WizardProgress currentStep={currentStep} onStep={setCurrentStep} />
              {currentStep === "purpose" && (
                <div className="grid gap-3 md:grid-cols-2">
                <PurposeButton
                  active={reportPurpose === "planned_exchange_sale"}
                  title="Planned exchange sale"
                  body="Prepare a bank or exchange disclosure before the deposit or sale happens."
                  onClick={() => setReportPurpose("planned_exchange_sale")}
                />
                <PurposeButton
                  active={reportPurpose === "existing_transaction"}
                  title="Already happened"
                  body="Explain a completed sale, exchange deposit, withdrawal, or transfer."
                  onClick={() => setReportPurpose("existing_transaction")}
                />
                </div>
              )}
              {currentStep === "purpose" && planned && (
                <div className="rounded-md border bg-muted/30 px-3 py-2 text-sm text-muted-foreground">
                  Planned reports prove the reviewed history of the bitcoin you
                  intend to sell. If those sats were originally bought on an
                  exchange, attach fiat-funds proof to that purchase source as a
                  separate evidence item.
                </div>
              )}
              {currentStep === "anchor" && (
              planned ? (
                <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_160px_150px]">
                  <Field label={targetLabel} htmlFor="sof-target">
                    <select
                      id="sof-target"
                      className="h-10 w-full rounded-md border bg-background px-3 text-sm"
                      value={selectedTarget}
                      onChange={(event) => setTarget(event.target.value)}
                    >
                      {rows.map((row) => (
                        <option key={txRef(row)} value={txRef(row)}>
                          {txLabel(row)}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <ReportControlFields
                    amountLabel={amountLabel}
                    targetAmount={targetAmount}
                    selectedTx={selectedTx}
                    revealMode={revealMode}
                    onAmountChange={setTargetAmount}
                    onRevealModeChange={setRevealMode}
                  />
                </div>
              ) : (
                <div className="space-y-3">
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
                        Selected: {txDate(selectedTx)} · {txWallet(selectedTx)} ·{" "}
                        {shortId(selectedTx.external_id || selectedTx.externalId || selectedTx.id)}
                      </div>
                    )}
                  </div>
                  <div className="rounded-md border">
                    <div className="flex flex-col gap-3 border-b p-3 lg:flex-row lg:items-center lg:justify-between">
                      <div>
                        <div className="text-sm font-medium">{targetLabel}</div>
                        <div className="text-xs text-muted-foreground">
                          {filteredTargetRows.length} of {rows.length} transactions
                        </div>
                      </div>
                      <div className="flex flex-wrap gap-2">
                        <div className="relative min-w-[220px] flex-1">
                          <Search
                            className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                            aria-hidden="true"
                          />
                          <Input
                            type="search"
                            value={targetSearch}
                            onChange={(event) => setTargetSearch(event.target.value)}
                            placeholder="Search txid, wallet, note..."
                            className="h-9 pl-9"
                          />
                        </div>
                        <select
                          className="h-9 rounded-md border bg-background px-3 text-sm"
                          value={targetDirectionFilter}
                          onChange={(event) => setTargetDirectionFilter(event.target.value)}
                          aria-label="Filter by direction"
                        >
                          <option value="all">All flows</option>
                          <option value="incoming">Incoming</option>
                          <option value="outgoing">Outgoing</option>
                          <option value="transfer">Transfer</option>
                          <option value="swap">Swap</option>
                        </select>
                        <select
                          className="h-9 rounded-md border bg-background px-3 text-sm"
                          value={targetDateFilter}
                          onChange={(event) => setTargetDateFilter(event.target.value)}
                          aria-label="Filter by date"
                        >
                          <option value="all">All dates</option>
                          <option value="today">Today</option>
                          <option value="yesterday">Yesterday</option>
                          <option value="7days">Last 7 days</option>
                          <option value="30days">Last 30 days</option>
                          <option value="older">Older</option>
                        </select>
                        <select
                          className="h-9 rounded-md border bg-background px-3 text-sm"
                          value={targetStatusFilter}
                          onChange={(event) => setTargetStatusFilter(event.target.value)}
                          aria-label="Filter by status"
                        >
                          <option value="all">All statuses</option>
                          <option value="confirmed">Confirmed</option>
                          <option value="pending">Pending</option>
                          <option value="review">Needs review</option>
                        </select>
                        <select
                          className="h-9 rounded-md border bg-background px-3 text-sm"
                          value={targetNetworkFilter}
                          onChange={(event) => setTargetNetworkFilter(event.target.value)}
                          aria-label="Filter by network"
                        >
                          <option value="all">All networks</option>
                          {targetNetworkOptions.map((network) => (
                            <option key={network} value={network}>
                              {network}
                            </option>
                          ))}
                        </select>
                        <select
                          className="h-9 rounded-md border bg-background px-3 text-sm"
                          value={targetAssetFilter}
                          onChange={(event) => setTargetAssetFilter(event.target.value)}
                          aria-label="Filter by asset"
                        >
                          <option value="all">All assets</option>
                          {targetAssetOptions.map((asset) => (
                            <option key={asset} value={asset}>
                              {asset}
                            </option>
                          ))}
                        </select>
                        <select
                          className="h-9 max-w-[190px] rounded-md border bg-background px-3 text-sm"
                          value={targetWalletFilter}
                          onChange={(event) => setTargetWalletFilter(event.target.value)}
                          aria-label="Filter by wallet"
                        >
                          <option value="all">All wallets</option>
                          {targetWalletOptions.map((wallet) => (
                            <option key={wallet} value={wallet}>
                              {wallet}
                            </option>
                          ))}
                        </select>
                        {(targetSearch ||
                          targetDirectionFilter !== "all" ||
                          targetDateFilter !== "all" ||
                          targetStatusFilter !== "all" ||
                          targetNetworkFilter !== "all" ||
                          targetAssetFilter !== "all" ||
                          targetWalletFilter !== "all") && (
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            className="h-9"
                            onClick={clearTargetFilters}
                          >
                            <X className="mr-2 size-4" aria-hidden="true" />
                            Clear
                          </Button>
                        )}
                      </div>
                    </div>
                    <TransactionTargetHeader />
                    <div className="max-h-[430px] overflow-y-auto p-2">
                      {filteredTargetRows.length === 0 ? (
                        <EmptyState text="No transactions match these filters." />
                      ) : (
                        <div className="space-y-2">
                          {filteredTargetRows.map((row) => (
                            <TransactionTargetRow
                              key={txRef(row)}
                              row={row}
                              active={txRef(row) === selectedTarget}
                              onSelect={() => setTarget(txRef(row))}
                            />
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )
              )}
              {currentStep === "anchor" && planned && (
                <div className="grid gap-3 md:grid-cols-[220px_minmax(0,1fr)]">
                  <Field label="Exchange or broker" htmlFor="planned-destination">
                    <Input
                      id="planned-destination"
                      value={plannedDestination}
                      onChange={(event) => setPlannedDestination(event.target.value)}
                      placeholder="Kraken, Bitpanda, OTC desk..."
                    />
                  </Field>
                  <Field label="Bank disclosure note" htmlFor="planned-note">
                    <Input
                      id="planned-note"
                      value={plannedNote}
                      onChange={(event) => setPlannedNote(event.target.value)}
                      placeholder="Expected EUR proceeds, bank contact, or internal case note"
                    />
                  </Field>
                </div>
              )}

              {currentStep === "review" && (
              <div className="grid gap-3 md:grid-cols-5">
                <Metric label="Nodes" value={report?.graph.nodes.length ?? 0} />
                <Metric label="Reviewed links" value={report?.graph.edges.length ?? 0} />
                <Metric
                  label="Batchable"
                  value={bulkReviewableSuggestions.length}
                />
                <Metric label="Sources" value={report?.source_mix.length ?? 0} />
                <Metric label="Blockers" value={blockers.length} />
              </div>
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
                  Find Links
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => void bulkReviewDeterministicLinks()}
                  disabled={
                    bulkReviewLinks.isPending ||
                    bulkReviewableSuggestions.length === 0
                  }
                >
                  <GitBranch className="mr-2 size-4" aria-hidden="true" />
                  Review Deterministic Hops
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() =>
                    setSourceForm((current) => ({
                      ...current,
                      source_type: "missing_history",
                      link_type: "missing_history",
                      label: current.label || "Reviewed missing history",
                      amount: current.amount || selectedTargetAmount,
                      description:
                        current.description ||
                        "Prior history is missing and has been reviewed as a disclosure gap.",
                    }))
                  }
                >
                  <AlertTriangle className="mr-2 size-4" aria-hidden="true" />
                  Mark Gap
                </Button>
                {manualSuggestionCount > 0 && (
                  <div className="basis-full rounded-md border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                    {manualSuggestionCount} weak or chain-observation suggestion
                    {manualSuggestionCount === 1 ? "" : "s"} still need manual review.
                  </div>
                )}
              </div>
              )}

              {currentStep === "disclosure" && (
                <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
                  Review what the report will expose before exporting. Change
                  reveal mode in the previous step if the disclosure is too broad.
                </div>
              )}

              {currentStep === "export" && (
                <div className="rounded-md border bg-muted/30 px-3 py-3 text-sm text-muted-foreground">
                  Export is available only when all blockers are cleared. The
                  PDF includes reviewed evidence only.
                </div>
              )}

              <div className="flex items-center justify-between border-t pt-4">
                <Button
                  type="button"
                  variant="outline"
                  onClick={goBack}
                  disabled={stepIndex === 0}
                >
                  Back
                </Button>
                {currentStep === "export" ? (
                  <Button
                    type="button"
                    disabled={!report?.explain_gates.exportable || exportPdf.isPending}
                    onClick={() =>
                      exportPdf.mutate({
                        target_transaction: selectedTarget,
                        target_amount: targetAmount || undefined,
                        report_purpose: reportPurpose,
                        planned_destination:
                          reportPurpose === "planned_exchange_sale"
                            ? plannedDestination || undefined
                            : undefined,
                        planned_note:
                          reportPurpose === "planned_exchange_sale"
                            ? plannedNote || undefined
                            : undefined,
                        reveal_mode: revealMode,
                      })
                    }
                  >
                    <FileDown className="mr-2 size-4" aria-hidden="true" />
                    Export PDF
                  </Button>
                ) : (
                  <Button type="button" onClick={goForward} disabled={!selectedTarget}>
                    Continue
                  </Button>
                )}
              </div>
            </CardContent>
          </Card>

          {currentStep === "review" && (
          <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_420px]">
            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Link2 className="size-4" aria-hidden="true" />
                  Review Queue
                </CardTitle>
                <CardDescription>
                  Matched links for the selected target, plus suggested upstream
                  hops that can extend the path.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                {reviewQueueLinks.length === 0 ? (
                  <EmptyState text="No matched links yet. Kassiber will look for same-id transfers, reviewed pairs, provider ids, and tight amount/time hints." />
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
                            ? "Path"
                            : link.to_transaction_id === selectedTxId
                              ? "Target"
                              : "Suggested"}
                        </span>
                        <span className="font-medium">{pretty(link.link_type)}</span>
                        <span className="text-muted-foreground">
                          {pretty(link.method)}
                        </span>
                      </div>
                      <div className="mt-2 grid gap-1 text-xs text-muted-foreground">
                        <span>
                          {link.from_source_id
                            ? sourceName(link.from_source_id)
                            : txName(link.from_transaction_id)}{" "}
                          {"->"} {txName(link.to_transaction_id)}
                        </span>
                        <span>
                          {formatBtc(link.allocation_amount ?? null, link.asset)} ·{" "}
                          {pretty(link.confidence)}
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
                  Link Review
                </CardTitle>
                <CardDescription>
                  Accept, reject, allocate, and attach evidence.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                {!selectedLink ? (
                  <EmptyState text="Select a link to review." />
                ) : (
                  <>
                    <div className="rounded-md border p-3 text-sm">
                      <div className="font-medium">
                        {selectedSource?.label ??
                          txName(selectedLink.from_transaction_id)}
                      </div>
                      <div className="text-muted-foreground">
                        to {txName(selectedLink.to_transaction_id)}
                      </div>
                    </div>
                    <div className="grid gap-3 sm:grid-cols-2">
                      <SelectField
                        id="review-link-type"
                        label="Type"
                        value={linkForm.link_type}
                        options={LINK_TYPES}
                        onChange={(value) =>
                          setLinkForm((current) => ({ ...current, link_type: value }))
                        }
                      />
                      <SelectField
                        id="review-confidence"
                        label="Confidence"
                        value={linkForm.confidence}
                        options={CONFIDENCE_LEVELS}
                        onChange={(value) =>
                          setLinkForm((current) => ({ ...current, confidence: value }))
                        }
                      />
                      <Field label="Allocation" htmlFor="review-allocation">
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
                      <Field label="From amount" htmlFor="review-from-allocation">
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
                    <Field label="Review note" htmlFor="review-note">
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
                        Accept
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        onClick={() => void reviewSelectedLink("rejected")}
                        disabled={reviewLink.isPending}
                      >
                        <X className="mr-2 size-4" aria-hidden="true" />
                        Reject
                      </Button>
                    </div>
                  </>
                )}
              </CardContent>
            </Card>
          </div>
          )}

          {currentStep === "review" && (
          <div className="grid gap-4 2xl:grid-cols-2">
            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Plus className="size-4" aria-hidden="true" />
                  Source Or Gap
                </CardTitle>
                <CardDescription>
                  Add a reviewed root source or explicit missing-history stop.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                <div className="grid gap-3 sm:grid-cols-2">
                  <SelectField
                    id="source-type"
                    label="Source type"
                    value={sourceForm.source_type}
                    options={SOURCE_TYPES}
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
                    label="Link type"
                    value={sourceForm.link_type}
                    options={LINK_TYPES}
                    onChange={(value) =>
                      setSourceForm((current) => ({ ...current, link_type: value }))
                    }
                  />
                  <Field label="Label" htmlFor="source-label">
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
                  <Field label="Amount" htmlFor="source-amount">
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
                  <Field label="Asset" htmlFor="source-asset">
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
                    label="Applies to"
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
                <Field label="Evidence note" htmlFor="source-description">
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
                  Create Source Link
                </Button>
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="border-b">
                <CardTitle className="flex items-center gap-2 text-base">
                  <Link2 className="size-4" aria-hidden="true" />
                  Manual Link
                </CardTitle>
                <CardDescription>
                  Connect two known transactions with explicit allocation.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3 p-4">
                <div className="grid gap-3 sm:grid-cols-2">
                  <TransactionSelect
                    id="manual-from"
                    label="From"
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
                    label="To"
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
                    label="Type"
                    value={manualLinkForm.link_type}
                    options={LINK_TYPES}
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        link_type: value,
                      }))
                    }
                  />
                  <SelectField
                    id="manual-confidence"
                    label="Confidence"
                    value={manualLinkForm.confidence}
                    options={CONFIDENCE_LEVELS}
                    onChange={(value) =>
                      setManualLinkForm((current) => ({
                        ...current,
                        confidence: value,
                      }))
                    }
                  />
                  <Field label="Allocation" htmlFor="manual-allocation">
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
                  <Field label="From amount" htmlFor="manual-from-amount">
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
                <Field label="Review note" htmlFor="manual-note">
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
                  Add Reviewed Link
                </Button>
              </CardContent>
            </Card>
          </div>
          )}
        </div>

        {showStepContext && (
        <div className="space-y-4">
          {(currentStep === "review" || currentStep === "export") && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2 text-base">
                <AlertTriangle className="size-4" aria-hidden="true" />
                Gates
              </CardTitle>
              <CardDescription>
                Blockers must clear before export.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-2 p-4">
              {preview.isLoading && <EmptyState text="Building reviewed flow..." />}
              {preview.isError && (
                <GateRow
                  finding={{
                    code: "preview_unavailable",
                    message: "No source-funds report can be built for this target yet.",
                  }}
                />
              )}
              {[...blockers, ...warnings].map((finding) => (
                <GateRow
                  key={`${finding.code}-${finding.ref ?? ""}-${finding.message}`}
                  finding={finding}
                />
              ))}
              {report && blockers.length === 0 && warnings.length === 0 && (
                <div className="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200">
                  Gates clear.
                </div>
              )}
            </CardContent>
          </Card>
          )}

          {(currentStep === "disclosure" || currentStep === "export") && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="flex items-center gap-2 text-base">
                <ShieldAlert className="size-4" aria-hidden="true" />
                Disclosure
              </CardTitle>
              <CardDescription>Exact report exposure.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 p-4 text-sm">
              <DisclosureNarrative report={report} />
              <DisclosureList
                label="Txids"
                values={report?.disclosure_preview.txids ?? []}
              />
              <DisclosureList
                label="Evidence"
                values={(report?.disclosure_preview.attachments ?? []).map(
                  (item) => item.label,
                )}
              />
              <DisclosureList
                label="Excluded"
                values={report?.disclosure_preview.excluded ?? []}
              />
              {report?.disclosure_preview.privacy_note && (
                <p className="rounded-md border px-3 py-2 text-xs text-muted-foreground">
                  {report.disclosure_preview.privacy_note}
                </p>
              )}
              <Button
                className="w-full"
                disabled={!report?.explain_gates.exportable || exportPdf.isPending}
                onClick={() =>
                  exportPdf.mutate({
                    target_transaction: selectedTarget,
                    target_amount: targetAmount || undefined,
                    report_purpose: reportPurpose,
                    planned_destination:
                      reportPurpose === "planned_exchange_sale"
                        ? plannedDestination || undefined
                        : undefined,
                    planned_note:
                      reportPurpose === "planned_exchange_sale"
                        ? plannedNote || undefined
                        : undefined,
                    reveal_mode: revealMode,
                  })
                }
              >
                <FileDown className="mr-2 size-4" aria-hidden="true" />
                Export PDF
              </Button>
              {exportedPdf && (
                <p className="text-xs text-muted-foreground">
                  {exportedPdf.filename}
                </p>
              )}
            </CardContent>
          </Card>
          )}

          {currentStep === "disclosure" && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">Source Mix</CardTitle>
            </CardHeader>
            <CardContent className="p-4">
              <div className="overflow-hidden rounded-md border">
                {(report?.source_mix ?? []).map((source) => (
                  <div
                    key={source.source_type}
                    className="flex items-center justify-between border-b px-3 py-2 text-sm last:border-b-0"
                  >
                    <span>{pretty(source.source_type)}</span>
                    <span className="font-mono tabular-nums">
                      {formatBtc(source.amount)}
                    </span>
                  </div>
                ))}
                {(report?.source_mix ?? []).length === 0 && (
                  <div className="px-3 py-2 text-sm text-muted-foreground">
                    No reviewed sources.
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
          )}

          {currentStep === "disclosure" && (
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">Reviewed Flow</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 p-4">
              {(report?.graph.edges ?? []).map((edge) => {
                const id = stringValue(edge.id) || JSON.stringify(edge);
                return (
                  <div key={id} className="rounded-md border px-3 py-2 text-sm">
                    <div className="font-medium">
                      {pretty(stringValue(edge.link_type) || "link")}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {formatBtc(numberValue(edge.allocation_amount), stringValue(edge.asset) || "BTC")}
                    </div>
                  </div>
                );
              })}
              {(report?.graph.edges ?? []).length === 0 && (
                <EmptyState text="No reviewed graph edges yet." />
              )}
            </CardContent>
          </Card>
          )}
        </div>
        )}
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
  const currentIndex = WIZARD_STEPS.findIndex((step) => step.id === currentStep);
  return (
    <div className="grid gap-2 md:grid-cols-5">
      {WIZARD_STEPS.map((step, index) => {
        const active = step.id === currentStep;
        const done = index < currentIndex;
        return (
          <button
            key={step.id}
            type="button"
            className={[
              "rounded-md border px-3 py-2 text-left text-sm transition-colors",
              active
                ? "border-primary bg-primary/5"
                : done
                  ? "bg-muted/60"
                  : "hover:bg-muted/40",
            ].join(" ")}
            onClick={() => onStep(step.id)}
          >
            <span className="block text-xs text-muted-foreground">
              Step {index + 1}
            </span>
            <span className="block font-medium">{step.label}</span>
          </button>
        );
      })}
    </div>
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
  onChange,
}: {
  id: string;
  label: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
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
            {pretty(option)}
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
  return (
    <Field label={label} htmlFor={id}>
      <select
        id={id}
        className="h-10 w-full rounded-md border bg-background px-3 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value="">Select transaction</option>
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
  return (
    <Field label="Evidence" htmlFor={id}>
      <select
        id={id}
        className="h-10 w-full rounded-md border bg-background px-3 text-sm"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value={NO_ATTACHMENT}>No attachment</option>
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
  const className =
    state === "reviewed"
      ? "border-emerald-200 bg-emerald-50 text-emerald-800 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200"
      : state === "rejected"
        ? "border-muted bg-muted text-muted-foreground"
        : "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200";
  return (
    <span className={`rounded-full border px-2 py-0.5 text-xs ${className}`}>
      {pretty(state)}
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

function GateRow({ finding }: { finding: SourceFundsFinding }) {
  const blocker = finding.severity === "blocker" || !finding.severity;
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

function DisclosureNarrative({ report }: { report?: SourceFundsPreview }) {
  const txidCount = report?.disclosure_preview.txids.length ?? 0;
  const evidenceCount = report?.disclosure_preview.attachments.length ?? 0;
  const hiddenCount = report?.disclosure_preview.excluded.length ?? 0;
  const sourceCount = report?.source_mix.length ?? 0;
  const sourceLabel = sourceCount === 1 ? "source category" : "source categories";
  const reviewedLinkCount = report?.graph.edges.length ?? 0;
  const walletLabels = uniqueSorted(
    (report?.graph.nodes ?? [])
      .map((node) => stringValue(node.wallet))
      .filter(Boolean),
  );
  const targetLabel = report?.target.label || "the selected target";
  const purposeLabel = report?.purpose?.label || "source-of-funds report";
  const revealMode = pretty(report?.reveal_mode || "standard");

  return (
    <section className="space-y-3 rounded-md border bg-muted/20 p-4">
      <div className="space-y-1">
        <h2 className="text-base font-semibold">Disclosure Summary</h2>
        <p className="text-sm text-muted-foreground">
          This {purposeLabel} will disclose the reviewed flow for {targetLabel}.
          It will expose {txidCount} txid{txidCount === 1 ? "" : "s"},{" "}
          {evidenceCount} evidence item{evidenceCount === 1 ? "" : "s"},{" "}
          {reviewedLinkCount} reviewed link{reviewedLinkCount === 1 ? "" : "s"},
          and {sourceCount} {sourceLabel}.
        </p>
      </div>

      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
        <DisclosureMetric label="Txids" value={txidCount} />
        <DisclosureMetric label="Evidence" value={evidenceCount} />
        <DisclosureMetric label="Reviewed links" value={reviewedLinkCount} />
        <DisclosureMetric label="Sources" value={sourceCount} />
        <DisclosureMetric label="Hidden" value={hiddenCount} />
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            Reveal mode
          </div>
          <div className="mt-1 font-medium">{revealMode}</div>
          <p className="mt-1 text-xs text-muted-foreground">
            {report?.disclosure_preview.privacy_note ||
              "No disclosure preview is available yet."}
          </p>
        </div>
        <div className="rounded-md border bg-background px-3 py-2">
          <div className="text-xs font-medium text-muted-foreground">
            Wallet labels
          </div>
          <div className="mt-1 text-sm">
            {walletLabels.length > 0 ? walletLabels.join(", ") : "None"}
          </div>
          <p className="mt-1 text-xs text-muted-foreground">
            Kassiber does not include descriptors, xpubs, wallet files, seeds,
            backend tokens, or unrelated wallet history in the PDF.
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

function DisclosureList({ label, values }: { label: string; values: string[] }) {
  return (
    <section className="space-y-2">
      <h2 className="text-sm font-semibold">{label}</h2>
      <div className="space-y-1">
        {values.length === 0 ? (
          <div className="rounded-md border px-3 py-2 text-muted-foreground">
            None
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
