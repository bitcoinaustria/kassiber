// Source-of-funds domain model: payload types mirrored from the Python
// daemon envelopes, CLI-precise vocabularies, and pure row helpers shared
// by every stage of the workstation.

export type TransactionRow = {
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


export type SourceFundsRecipient = {
  id: string;
  label: string;
  kind: string;
  default_reveal_mode: string;
  notes?: string;
  active?: boolean;
  created_at?: string;
  updated_at?: string;
};


export type SourceFundsCoverageBucket = {
  amount: number;
  amount_msat: number;
  tx_count: number;
};


export type SourceFundsCoverageBuckets = {
  fully_traced: SourceFundsCoverageBucket;
  attested: SourceFundsCoverageBucket;
  in_review: SourceFundsCoverageBucket;
  untraced: SourceFundsCoverageBucket;
  not_classified: SourceFundsCoverageBucket;
};


export type SourceFundsCoverage = {
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


export type SourceFundsFindingNextStep = {
  headline?: string;
  action?: string;
  action_args?: Record<string, unknown>;
  doc_anchor?: string;
};


export type SourceFundsFinding = {
  code: string;
  severity?: string;
  message: string;
  ref?: string;
  amount?: number | null;
  amount_msat?: number | null;
  asset?: string;
  next_step?: SourceFundsFindingNextStep;
};


export const BULK_REVIEWABLE_METHODS = new Set([
  "same_external_id",
  "transaction_pair",
  "utxo_spend",
  "payment_hash",
  "provider_trade_id",
  "provider_order_id",
  "provider_payment_id",
  "provider_exchange_order_id",
  "provider_ledger_id",
]);


export type SourceFundsPreview = {
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
    provenance?: string;
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
        transaction_id?: string;
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
    assets?: string[];
    fiat_currency?: string;
    fiat_value_total?: number | null;
    nodes: {
      id: string;
      node_type: string;
      label: string;
      wallet?: string;
      source_type?: string;
      direction?: string;
      asset?: string;
      required_amount?: number | null;
      amount?: number | null;
      fee?: number | null;
      fee_msat?: number | null;
      fiat_currency?: string;
      fiat_value?: number | null;
      fiat_value_allocated?: number | null;
      occurred_at?: string;
      acquired_at?: string;
      external_id?: string;
      data_provenance?: string;
    }[];
  }[];
  data_provenance_summary?: {
    provenance: string;
    label: string;
    count: number;
    percent: number;
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
    wallets_named?: string[];
    ownership_note?: string;
    privacy_note: string;
    excluded: string[];
  };
};


export type SourceFundsSource = {
  id: string;
  source_type: string;
  label: string;
  asset: string;
  amount?: number | null;
  amount_msat?: number | null;
  description?: string;
  attachments?: EvidenceAttachment[];
};


export type SourceFundsLink = {
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


export type EvidenceAttachment = {
  id: string;
  label: string;
  attachment_type?: string;
  external_id?: string;
  wallet?: string;
  occurred_at?: string;
};


export const SOURCE_TYPES = [
  "fiat_purchase",
  "exchange_withdrawal",
  "mining",
  "income",
  "gift",
  "opening_balance_attestation",
  "missing_history",
  "unknown",
];


export const LINK_TYPES = [
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


export const CONFIDENCE_LEVELS = ["exact", "strong", "weak", "unknown"];


export const REVEAL_MODES = ["labels_only", "minimal", "standard", "full"];


export const NO_ATTACHMENT = "__none__";


export const NO_RECIPIENT = "__none__";


export type TxPickerFlow = "incoming" | "outgoing" | "transfer" | "swap";


export function transactionRows(data: unknown): TransactionRow[] {
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


export function txRef(row: TransactionRow): string {
  return row.id || row.transaction_id || row.external_id || "";
}


export function txLabel(row: TransactionRow): string {
  const date = row.occurred_at || row.date || "";
  const wallet = txWallet(row);
  const kind = row.direction || row.type || "transaction";
  const id = row.external_id || row.externalId || row.id || "";
  return [date.slice(0, 10), wallet, kind, id].filter(Boolean).join(" · ");
}


export function txAmount(row: TransactionRow): string {
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


export function txSignedAmount(row: TransactionRow): string {
  const asset = row.asset || "BTC";
  const flow = txFlow(row);
  const sign = flow === "incoming" ? "+" : flow === "outgoing" ? "-" : "";
  const format = (value: number) => `${sign}${Math.abs(value).toFixed(8)} ${asset}`;
  if (typeof row.amount === "number") return format(row.amount);
  if (typeof row.amountSat === "number") return format(row.amountSat / 100_000_000);
  if (typeof row.amount_msat === "number") return format(row.amount_msat / 100_000_000_000);
  return asset;
}


export function txDirection(row: TransactionRow): string {
  return String(row.direction || row.type || "transaction").toLowerCase();
}


export function txFlow(row: TransactionRow): TxPickerFlow {
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


export function txFlowLabel(row: TransactionRow): string {
  const flow = txFlow(row);
  if (flow === "incoming") return "Incoming";
  if (flow === "outgoing") return "Outgoing";
  if (flow === "swap") return "Swap";
  return "Transfer";
}


export function txWallet(row: TransactionRow): string {
  return row.wallet || row.account || "Wallet";
}


export function txStatus(row: TransactionRow): "confirmed" | "pending" | "review" {
  const tag = String(row.tag || "").toLowerCase();
  if (tag.includes("review")) return "review";
  return typeof row.conf === "number" && row.conf <= 0 ? "pending" : "confirmed";
}


export function txNetwork(row: TransactionRow): string {
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


export function txSearchText(row: TransactionRow): string {
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


export function txDate(row: TransactionRow): string {
  return (row.occurred_at || row.date || "").slice(0, 10) || "Unknown date";
}


export function txDateFilterValue(row: TransactionRow): string {
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

// Each range filter matches its own bucket plus the more-recent ones, so
// "Last 7 days" / "Last 30 days" include today and yesterday rather than
// excluding them. "today"/"yesterday"/"older" stay single buckets.


export const DATE_FILTER_BUCKETS: Record<string, ReadonlySet<string>> = {
  today: new Set(["today"]),
  yesterday: new Set(["yesterday"]),
  "7days": new Set(["today", "yesterday", "7days"]),
  "30days": new Set(["today", "yesterday", "7days", "30days"]),
  older: new Set(["older"]),
};


export function uniqueSorted(values: string[]) {
  return Array.from(new Set(values.filter(Boolean))).sort((a, b) =>
    a.localeCompare(b),
  );
}


export function formatBtc(value: number | null | undefined, asset = "BTC") {
  if (typeof value !== "number") return "-";
  return `${value.toFixed(8)} ${asset}`;
}


export function formatDateTime(value?: string | null) {
  if (!value) return "-";
  return value.replace("T", " ").replace("Z", "").slice(0, 16);
}


export function pretty(value: string) {
  return value.replaceAll("_", " ");
}


export function shortId(value?: string | null) {
  if (!value) return "-";
  return value.length > 18 ? `${value.slice(0, 10)}...${value.slice(-6)}` : value;
}


export function stringValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}


export function numberValue(value: unknown): number | null {
  return typeof value === "number" ? value : null;
}


export function isBulkReviewableLink(link: SourceFundsLink) {
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


export const COVERAGE_BUCKET_ORDER: (keyof SourceFundsCoverageBuckets)[] = [
  "fully_traced",
  "attested",
  "in_review",
  "untraced",
  "not_classified",
];


export const COVERAGE_BUCKET_LABELS: Record<keyof SourceFundsCoverageBuckets, string> = {
  fully_traced: "Fully traced",
  attested: "Attested",
  in_review: "In review",
  untraced: "Untraced",
  not_classified: "Not classified",
};


export const COVERAGE_BUCKET_TONES: Record<keyof SourceFundsCoverageBuckets, string> = {
  fully_traced: "text-emerald-700 dark:text-emerald-300",
  attested: "text-sky-700 dark:text-sky-300",
  in_review: "text-amber-700 dark:text-amber-300",
  untraced: "text-rose-700 dark:text-rose-300",
  not_classified: "text-muted-foreground",
};


export function coverageSummary(coverage?: SourceFundsCoverage) {
  if (!coverage || coverage.totals.tx_count === 0) {
    return "No inbound coverage snapshot";
  }
  const traced = coverage.totals.buckets.fully_traced.amount;
  const count = coverage.totals.tx_count;
  return `${traced.toFixed(8)} BTC fully traced across ${count} inbound transaction${count === 1 ? "" : "s"}`;
}


export const COVERAGE_BUCKET_BARS: Record<keyof SourceFundsCoverageBuckets, string> = {
  fully_traced: "bg-emerald-500",
  attested: "bg-sky-500",
  in_review: "bg-amber-500",
  untraced: "bg-rose-500",
  not_classified: "bg-muted-foreground/40",
};


export const GAP_ACTION_LABELS: Record<string, string> = {
  open_source_creator: "Document this gap",
  open_link_review: "Review this link",
  open_review_queue: "Open review queue",
  open_source: "Open sources",
  open_transaction: "Open transaction",
};


export const PROVENANCE_SHORT_LABELS: Record<string, string> = {
  chain_sync: "chain",
  platform_export: "platform",
  manual_import: "manual",
};
