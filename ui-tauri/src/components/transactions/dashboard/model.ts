import { type ChartConfig } from "@/components/ui/chart";
import { type Currency } from "@/lib/currency";
import { transactionRecords } from "./demoRecords";
import { type Tx } from "@/mocks/seed";
import {
  type AttachmentItem,
  type JournalEventItem,
  SATS_PER_BTC,
  formatSignedDisplayMoney,
  transactionBtc,
  transactionFlow,
  type Transaction,
  type TransactionDirection,
  type TransactionFlow,
  type TransactionStatus,
} from "@/components/transactions";

type PeriodKey =
  | "ytd"
  | "30days"
  | "3months"
  | "1year"
  | "5years"
  | "10years"
  | "15years"
  | "all";
type FlowChartMetric = "amount" | "count";
type FlowChartMode = "external" | "all";
type FlowChartSegment = "incoming" | "outgoing" | "transfers" | "swaps";

type FlowChartPoint = {
  bucketKey: string;
  date: string;
  incoming: number;
  outgoing: number;
  transfers: number;
  swaps: number;
  stats: Record<FlowChartSegment, FlowChartSegmentStats>;
};

type FlowChartSegmentStats = {
  count: number;
  btc: number;
  eur: number;
  missingPrice: number;
  review: number;
  failed: number;
  largest?: {
    label: string;
    btc: number;
    eur: number;
  };
};

type FlowChartSelection = {
  id: string;
  period: PeriodKey;
  bucketKey: string | null;
  bucketLabel: string;
  segment: FlowChartSegment | null;
  mode: FlowChartMode;
};

type TableQuickFilter =
  | "external_flow"
  | "review_queue"
  | "no_explorer_id"
  | "missing_price"
  | "failed_import";
type BreakdownSelection = {
  dimension: "network" | "wallet";
  key: string;
  /**
   * How the wallet `key` matches transaction rows. The breakdown chart buckets
   * transfers by their full account string ("A → B"), so chart-driven
   * selections must match exactly to keep the table count equal to the clicked
   * bar's count. The Wallet dropdown and the "Show all" deep link instead match
   * by transfer leg (so "Cold Storage" also surfaces "Cold Storage → Vault").
   * Absent / "exact" = full-string match (chart, network); "leg" = leg-aware.
   */
  match?: "exact" | "leg";
};

type FlowChartClickData = {
  payload?: FlowChartPoint;
  activePayload?: Array<{ payload?: FlowChartPoint }>;
};

type FlowBucket = {
  key: string;
  label: string;
};

type SwapCandidate = {
  in: Transaction;
  out: Transaction;
  eur: number | null;
  btc: number;
};

export type SwapCandidateReference = {
  in_id: string;
  out_id: string;
  in_asset?: string;
  out_asset?: string;
  default_kind?: string;
  candidate_type?: "transfer" | "swap";
  conflict_set_id?: string;
  /** Matcher-stamped cluster size over the full candidate set. */
  conflict_size?: number;
};

const BITCOIN_LAYER_TRANSITION_PAIR_KINDS = new Set([
  "peg-in",
  "peg-out",
  "submarine-swap",
  "swap-refund",
]);

const flowColors: Record<TransactionFlow, string> = {
  incoming: "oklch(0.56 0.16 150)",
  outgoing: "var(--kb-accent)",
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


type Translator = (key: string, opts?: Record<string, unknown>) => string;

function toDashboardTransaction(
  tx: Tx,
  index: number,
  t?: Translator,
): Transaction {
  const tag = tx.tag || "";
  const displayAmountSat =
    tx.type === "Consolidation" && tx.amountSat === 0 && tx.feeSat
      ? tx.feeSat
      : tx.amountSat;
  const isSwap =
    tag.toLowerCase().includes("swap") ||
    tx.type === "Swap" ||
    tx.type === "Mint" ||
    tx.type === "Melt";
  const flow: TransactionFlow = isSwap
    ? "swap"
    : tx.internal ||
        tx.type === "Transfer" ||
        tx.type === "Consolidation" ||
        tx.type === "Rebalance"
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
    amount:
      tx.eur !== null
        ? Math.abs(tx.eur)
        : tx.rate !== null
          ? Math.abs((displayAmountSat / SATS_PER_BTC) * tx.rate)
          : null,
    amountBtc: Math.abs(displayAmountSat / SATS_PER_BTC),
    feeBtc: tx.feeSat ? Math.abs(tx.feeSat / SATS_PER_BTC) : 0,
    feeEur:
      tx.feeSat && tx.rate !== null
        ? Math.abs((tx.feeSat / SATS_PER_BTC) * tx.rate)
        : null,
    asset: "BTC",
    rate: tx.rate,
    fiatCurrency: tx.fiatCurrency,
    pricingSourceKind: tx.pricingSourceKind as Transaction["pricingSourceKind"],
    pricingQuality: tx.pricingQuality as Transaction["pricingQuality"],
    pricingExternalRef: tx.pricingExternalRef,
    pricingProvider: tx.pricingProvider,
    pricingPair: tx.pricingPair,
    pricingTimestamp: tx.pricingTimestamp,
    pricingFetchedAt: tx.pricingFetchedAt,
    pricingGranularity: tx.pricingGranularity,
    pricingMethod: tx.pricingMethod,
    reviewStatus: normalizeTransactionReviewStatus(tx.reviewStatus, status),
    taxable: tx.taxable,
    atRegime: tx.atRegime as Transaction["atRegime"],
    atCategory: tx.atCategory as Transaction["atCategory"],
    note: tx.note,
    tags: tx.tags,
    excluded: tx.excluded,
    quarantineReason: tx.quarantineReason ?? null,
    pair: tx.pair,
    // Empty = no counterparty recorded; surfaces fall back per context
    // (short txid in tables, hidden segment in the detail header).
    counterparty: tx.counter || "",
    counterpartyInitials: initials(tx.counter || tx.account || "TX"),
    direction,
    flow,
    wallet:
      tx.account ||
      (t ? t("transactions:fallback.unassignedWallet") : "Unassigned wallet"),
    tag,
    sourceType: tx.type,
    paymentMethod,
    date: tx.date,
    status: tag.toLowerCase().includes("review") ? "review" : status,
    confirmations: tx.conf,
  };
}

function normalizeTransactionReviewStatus(
  raw: string | null | undefined,
  fallback: TransactionStatus,
): TransactionStatus {
  const normalized = raw?.trim().toLowerCase().replaceAll("-", "_");
  if (!normalized) return fallback;
  if (normalized === "completed" || normalized === "complete") return "completed";
  if (normalized === "pending") return "pending";
  if (normalized === "failed" || normalized === "error") return "failed";
  if (
    normalized === "review" ||
    normalized === "needs_review" ||
    normalized === "blocked" ||
    normalized === "quarantined"
  ) {
    return "review";
  }
  return fallback;
}

function dashboardRecordsFromTxs(txs: Tx[], t?: Translator) {
  return txs.map((tx, index) => toDashboardTransaction(tx, index, t));
}

// Stable English flow labels for the redundancy comparison below. The stored
// `label` is a persisted code (English), so this must match against stable
// English, not the translated flow label.
const flowLabelStableEnglish: Record<TransactionFlow, string> = {
  incoming: "incoming",
  outgoing: "outgoing",
  transfer: "internal transfer",
  swap: "swap",
  "layer-transition": "Bitcoin swap",
};

function isRedundantTransactionLabel(label: string, flow: TransactionFlow) {
  const normalized = label.trim().toLowerCase();
  if (!normalized || normalized === "unlabeled") return true;
  return normalized === flowLabelStableEnglish[flow];
}

function pairRailLabel(txn: Transaction, t?: Translator) {
  const pair = txn.pair;
  if (!pair) return txn.paymentMethod;
  const outRail = railLabelForAssetOrWallet(pair.outAsset, pair.outWallet, t);
  const inRail = railLabelForAssetOrWallet(pair.inAsset, pair.inWallet, t);
  return outRail === inRail ? outRail : `${outRail} -> ${inRail}`;
}

// Network names stay English (Lightning/Liquid/on-chain); only the "Other"
// fallback is localized via the optional translator.
function railLabelForAssetOrWallet(
  asset?: string | null,
  wallet?: string | null,
  t?: Translator,
) {
  const text = `${asset ?? ""} ${wallet ?? ""}`.toLowerCase();
  if (text.includes("lightning") || text.includes("phoenix") || text.includes("ln")) {
    return "Lightning";
  }
  if (text.includes("lbtc") || text.includes("liquid")) {
    return "Liquid";
  }
  if (text.includes("btc") || text.includes("onchain") || text.includes("on-chain")) {
    return "On-chain";
  }
  return t ? t("transactions:classification.other") : "Other";
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

type AttachmentRecord = {
  id: string;
  attachment_type: "file" | "url";
  label?: string | null;
  display_label?: string;
  original_filename?: string;
  url?: string;
  media_type?: string;
  size_bytes?: number | null;
  sha256?: string;
  exists?: boolean | null;
  copied_from_attachment_id?: string;
  copied_from_transaction_id?: string;
};

type AttachmentsListData = {
  attachments: AttachmentRecord[];
};

type AttachmentsCopyData = {
  copied: number;
  attachments: AttachmentRecord[];
  source_transaction_id?: string;
  target_transaction_id?: string;
};

type AttachmentOpenData = {
  target_type: "file" | "url";
  path?: string;
  url?: string;
  attachment: AttachmentRecord;
};

type JournalEventsData = {
  events: JournalEventItem[];
};

function compactBytes(bytes: number | null | undefined) {
  if (bytes === null || bytes === undefined) return null;
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function urlAttachmentDetail(
  rawUrl: string | undefined,
  label: string,
  hasStoredLabel: boolean,
) {
  if (!rawUrl || rawUrl === label) return undefined;
  if (hasStoredLabel) return rawUrl;
  try {
    const parsed = new URL(rawUrl);
    const host = parsed.hostname.replace(/^www\./i, "");
    if (label === host || label.startsWith(`${host} - `)) {
      return undefined;
    }
  } catch {
    return rawUrl;
  }
  return rawUrl;
}

function attachmentRecordToItem(
  record: AttachmentRecord,
  t?: (key: string) => string,
): AttachmentItem {
  const kind = record.attachment_type === "url" ? "url" : "file";
  const size = compactBytes(record.size_bytes);
  const hash = record.sha256 ? `sha256 ${record.sha256.slice(0, 6)}...` : null;
  const fileBits = [
    record.media_type || null,
    size,
    hash,
    record.exists === false
      ? t
        ? t("transactions:attachment.missingFile")
        : "missing file"
      : null,
  ].filter(Boolean);
  const storedLabel = record.label?.trim();
  const fallbackLinkLabel = t
    ? t("transactions:attachment.linkLabel")
    : "Link attachment";
  const fallbackFileLabel = t
    ? t("transactions:attachment.fileLabel")
    : "File attachment";
  const label =
    kind === "url"
      ? record.display_label || storedLabel || fallbackLinkLabel
      : record.display_label ||
        storedLabel ||
        record.original_filename ||
        fallbackFileLabel;
  return {
    id: record.id,
    kind,
    label,
    detail:
      kind === "url"
        ? urlAttachmentDetail(record.url, label, Boolean(storedLabel))
        : fileBits.join(" · ") || record.original_filename || undefined,
    href: record.url || undefined,
    copiedFromAttachmentId: record.copied_from_attachment_id || undefined,
    copiedFromTransactionId: record.copied_from_transaction_id || undefined,
  };
}

function replaceAttachmentRecord(
  attachments: AttachmentRecord[],
  updated: AttachmentRecord,
) {
  let replaced = false;
  const next = attachments.map((attachment) => {
    if (attachment.id !== updated.id) return attachment;
    replaced = true;
    return updated;
  });
  return replaced ? next : attachments;
}

function upsertAttachmentRecords(
  attachments: AttachmentRecord[],
  updates: AttachmentRecord[],
) {
  if (!updates.length) return attachments;
  const updateById = new Map(
    updates.map((attachment) => [attachment.id, attachment]),
  );
  const seen = new Set<string>();
  const next = attachments.map((attachment) => {
    const updated = updateById.get(attachment.id);
    if (!updated) return attachment;
    seen.add(attachment.id);
    return updated;
  });
  const created = updates.filter((attachment) => !seen.has(attachment.id));
  return created.length ? [...created, ...next] : next;
}

function removeAttachmentRecord(
  attachments: AttachmentRecord[],
  attachmentId: string,
) {
  return attachments.filter((attachment) => attachment.id !== attachmentId);
}

function isAttachmentListQueryKeyForTransaction(
  queryKey: readonly unknown[],
  transactionId: string,
) {
  return (
    queryKey.includes("ui.attachments.list") &&
    queryKey.some((part) => {
      if (!part || typeof part !== "object" || Array.isArray(part)) {
        return false;
      }
      return (part as { transaction?: unknown }).transaction === transactionId;
    })
  );
}

// The following label maps hold i18n keys (resolved with t() at the call site).
const periodLabels = {
  ytd: "transactions:period.ytd",
  "1year": "transactions:period.1year",
  "3months": "transactions:period.3months",
  "30days": "transactions:period.30days",
  "5years": "transactions:period.5years",
  "10years": "transactions:period.10years",
  "15years": "transactions:period.15years",
  all: "transactions:period.all",
} as const satisfies Record<PeriodKey, string>;

const flowChartMetricLabels = {
  amount: "transactions:chartMetric.amount",
  count: "transactions:chartMetric.count",
} as const satisfies Record<FlowChartMetric, string>;

const flowChartModeLabels = {
  external: "transactions:chartMode.external",
  all: "transactions:chartMode.all",
} as const satisfies Record<FlowChartMode, string>;

const flowChartSegmentLabels = {
  incoming: "transactions:chartSegment.incoming",
  outgoing: "transactions:chartSegment.outgoing",
  transfers: "transactions:chartSegment.transfers",
  swaps: "transactions:chartSegment.swaps",
} as const satisfies Record<FlowChartSegment, string>;

const emptyFlowChartSegmentStats = (): FlowChartSegmentStats => ({
  count: 0,
  btc: 0,
  eur: 0,
  missingPrice: 0,
  review: 0,
  failed: 0,
});

const emptyFlowChartStats = (): Record<FlowChartSegment, FlowChartSegmentStats> => ({
  incoming: emptyFlowChartSegmentStats(),
  outgoing: emptyFlowChartSegmentStats(),
  transfers: emptyFlowChartSegmentStats(),
  swaps: emptyFlowChartSegmentStats(),
});

const periodKeys: PeriodKey[] = [
  "30days",
  "3months",
  "ytd",
  "1year",
  "5years",
  "10years",
  "15years",
  "all",
];

const basePeriodKeys: PeriodKey[] = ["30days", "3months", "ytd", "1year"];
const longHistoryPeriodKeys = [
  { key: "5years", years: 5 },
  { key: "10years", years: 10 },
  { key: "15years", years: 15 },
] as const satisfies ReadonlyArray<{ key: PeriodKey; years: number }>;
const MS_PER_YEAR = 365.2425 * 24 * 60 * 60 * 1000;

function isLongHistoryPeriod(period: PeriodKey) {
  return (
    period === "5years" ||
    period === "10years" ||
    period === "15years" ||
    period === "all"
  );
}

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
  if (
    normalized === "10years" ||
    normalized === "10year" ||
    normalized === "10yrs" ||
    normalized === "10yr" ||
    normalized === "10y"
  ) {
    return "10years";
  }
  if (
    normalized === "15years" ||
    normalized === "15year" ||
    normalized === "15yrs" ||
    normalized === "15yr" ||
    normalized === "15y"
  ) {
    return "15years";
  }
  if (normalized === "all" || normalized === "max") {
    return "all";
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
  if (period === "10years") return 90;
  if (period === "15years") return 120;
  if (period === "all") return Number.MAX_SAFE_INTEGER;
  return 30;
}

function sortTransactionsByDateDesc(records: Transaction[]) {
  return [...records].sort((a, b) => {
    const dateA = parseTransactionDate(a.date)?.getTime() ?? -Infinity;
    const dateB = parseTransactionDate(b.date)?.getTime() ?? -Infinity;
    return dateB - dateA;
  });
}

function recordsForPeriod(records: Transaction[], period: PeriodKey) {
  if (period === "all") return records;

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

function availablePeriodKeysForRecords(records: Transaction[]): PeriodKey[] {
  const dated = records
    .map((record) => parseTransactionDate(record.date))
    .filter((date): date is Date => date !== null);
  if (!dated.length) return [...basePeriodKeys, "all"];

  const end = periodAnchorDate(dated);
  const earliest = dated.reduce((min, date) => (date < min ? date : min), dated[0]);
  const historyYears = Math.max(
    0,
    (startOfLocalDay(end).getTime() - startOfLocalDay(earliest).getTime()) /
      MS_PER_YEAR,
  );

  return [
    ...basePeriodKeys,
    ...longHistoryPeriodKeys
      .filter((period) => historyYears >= period.years)
      .map((period) => period.key),
    "all",
  ];
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

function periodStartDate(end: Date, period: PeriodKey, earliest?: Date) {
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
  } else if (period === "10years") {
    start.setFullYear(start.getFullYear() - 10);
  } else if (period === "15years") {
    start.setFullYear(start.getFullYear() - 15);
  } else if (period === "all" && earliest) {
    return startOfLocalDay(earliest);
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
  } else if (isLongHistoryPeriod(period)) {
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
  if (isLongHistoryPeriod(period)) {
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
  const earliest = dated.reduce((min, date) => (date < min ? date : min), dated[0]);
  let cursor = periodStartDate(end, period, earliest);
  if (period === "3months") cursor = startOfIsoWeek(cursor);
  if (isLongHistoryPeriod(period)) {
    cursor.setMonth(Math.floor(cursor.getMonth() / 3) * 3, 1);
    cursor.setHours(0, 0, 0, 0);
  }

  while (cursor <= end) {
    const bucket = bucketTransactionDate(cursor, period);
    grouped.set(bucket.key, {
      bucketKey: bucket.key,
      date: bucket.label,
      incoming: 0,
      outgoing: 0,
      transfers: 0,
      swaps: 0,
      stats: emptyFlowChartStats(),
    });
    cursor = addBucketStep(cursor, period);
  }
  return grouped;
}

function flowBucketLabel(period: PeriodKey) {
  if (period === "30days") return "day";
  if (period === "3months") return "week";
  if (isLongHistoryPeriod(period)) return "quarter";
  return "month";
}

function sumByFlow(records: Transaction[], flow: TransactionFlow) {
  const rows = records.filter((txn) => transactionFlow(txn) === flow);
  return {
    count: rows.length,
    eur: rows.reduce((sum, txn) => sum + (txn.amount ?? 0), 0),
    btc: rows.reduce((sum, txn) => sum + transactionBtc(txn), 0),
  };
}

function buildSwapCandidates(
  records: Transaction[],
  candidateRefs?: SwapCandidateReference[],
): SwapCandidate[] {
  return buildPairingCandidates(records, candidateRefs, "swap");
}

function buildTransferCandidates(
  records: Transaction[],
  candidateRefs?: SwapCandidateReference[],
): SwapCandidate[] {
  return buildPairingCandidates(records, candidateRefs, "transfer");
}

function buildPairingCandidates(
  records: Transaction[],
  candidateRefs: SwapCandidateReference[] | undefined,
  reviewType: "transfer" | "swap",
): SwapCandidate[] {
  if (candidateRefs) {
    const recordsById = new Map(records.map((txn) => [txn.id, txn]));
    return nonConflictedCandidateRefs(candidateRefs)
      .filter((candidate) => candidateReferenceReviewType(candidate) === reviewType)
      .flatMap((candidate) => {
        const input = recordsById.get(candidate.in_id);
        const out = recordsById.get(candidate.out_id);
        if (!input || !out) return [];
        return [
          {
            in: input,
            out,
            eur:
              input.amount !== null && out.amount !== null
                ? Math.min(input.amount, out.amount)
                : null,
            btc: Math.min(transactionBtc(input), transactionBtc(out)),
          },
        ];
      });
  }

  if (reviewType === "transfer") return [];

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
      eur:
        best.txn.amount !== null && out.txn.amount !== null
          ? Math.min(best.txn.amount, out.txn.amount)
          : null,
      btc: Math.min(transactionBtc(best.txn), transactionBtc(out.txn)),
    });
  }

  return candidates;
}

function candidateReferenceReviewType(
  candidate: SwapCandidateReference,
): "transfer" | "swap" {
  if (candidate.candidate_type === "transfer" || candidate.candidate_type === "swap") {
    return candidate.candidate_type;
  }
  if (
    candidate.default_kind &&
    BITCOIN_LAYER_TRANSITION_PAIR_KINDS.has(candidate.default_kind)
  ) {
    return "transfer";
  }
  if (candidate.in_asset && candidate.out_asset) {
    return candidate.in_asset.toUpperCase() === candidate.out_asset.toUpperCase()
      ? "transfer"
      : "swap";
  }
  return "swap";
}

function nonConflictedCandidateRefs(
  candidateRefs: SwapCandidateReference[],
): SwapCandidateReference[] {
  // Prefer the matcher-stamped conflict_size (computed over the full
  // candidate set); the local recount only covers refs missing the field.
  const clusterSizes = new Map<string, number>();
  candidateRefs.forEach((candidate, index) => {
    const clusterId = candidate.conflict_set_id ?? `solo:${index}`;
    clusterSizes.set(clusterId, (clusterSizes.get(clusterId) ?? 0) + 1);
  });

  const usedLegs = new Set<string>();
  return candidateRefs.filter((candidate, index) => {
    const clusterId = candidate.conflict_set_id ?? `solo:${index}`;
    const clusterSize =
      candidate.conflict_size ?? clusterSizes.get(clusterId) ?? 0;
    if (clusterSize > 1) return false;
    if (usedLegs.has(candidate.in_id) || usedLegs.has(candidate.out_id)) {
      return false;
    }
    usedLegs.add(candidate.in_id);
    usedLegs.add(candidate.out_id);
    return true;
  });
}

function buildFlowChartRows(
  records: Transaction[],
  period: PeriodKey,
  currency: Currency,
  candidateFlowOverrides = new Map<string, TransactionFlow>(),
  metric: FlowChartMetric = "amount",
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
        bucketKey: bucket.key,
        date: bucket.label,
        incoming: 0,
        outgoing: 0,
        transfers: 0,
        swaps: 0,
        stats: emptyFlowChartStats(),
      };
    const value =
      metric === "count" ? 1 : currency === "btc" ? transactionBtc(txn) : (txn.amount ?? 0);
    const flow = candidateFlowOverrides.get(txn.id) ?? transactionFlow(txn);
    const segment = flowChartSegmentForFlow(flow);
    if (flow === "incoming") row.incoming += value;
    if (flow === "outgoing") row.outgoing -= value;
    if (flow === "transfer" || flow === "layer-transition") {
      row.transfers += value;
    }
    if (flow === "swap") row.swaps += value;
    if (segment) {
      addFlowChartSegmentStats(row.stats[segment], txn);
    }
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
    row.eur += txn.amount ?? 0;
    row.btc += transactionBtc(txn);
    rows.set(key, row);
  }
  return Array.from(rows.values()).sort((a, b) => b.eur - a.eur);
}

function flowChartSegmentFromDataKey(
  dataKey: string | number | undefined,
): FlowChartSegment | null {
  if (dataKey === "incoming" || dataKey === "outgoing") return dataKey;
  if (dataKey === "transfers" || dataKey === "swaps") return dataKey;
  return null;
}

function flowColorForSegment(segment: FlowChartSegment | null) {
  if (segment === "incoming") return flowColors.incoming;
  if (segment === "outgoing") return flowColors.outgoing;
  if (segment === "transfers") return flowColors.transfer;
  if (segment === "swaps") return flowColors.swap;
  return "currentColor";
}

function formatFlowTooltipValue(
  value: number,
  currency: Currency,
  metric: FlowChartMetric,
  t?: Translator,
) {
  if (metric === "count") {
    const count = Math.abs(value);
    if (t) {
      return value >= 0
        ? t("transactions:workbench.tooltip.txDeltaPositive", { count })
        : t("transactions:workbench.tooltip.txDeltaNegative", { count });
    }
    const prefix = value >= 0 ? "+ " : "− ";
    return `${prefix}${count} tx`;
  }
  return formatSignedDisplayMoney(value, value, currency);
}

function formatCountBarLabel(value: unknown) {
  const count = Math.abs(Number(value ?? 0));
  return count > 0 ? String(Math.round(count)) : "";
}

function flowPointEntries(row: FlowChartPoint) {
  return [
    ["incoming", row.incoming],
    ["outgoing", row.outgoing],
    ["transfer", row.transfers],
    ["swap", row.swaps],
  ] as const;
}

function flowPointSegmentValue(row: FlowChartPoint, segment: FlowChartSegment) {
  return segment === "incoming"
    ? row.incoming
    : segment === "outgoing"
      ? row.outgoing
      : segment === "transfers"
        ? row.transfers
        : row.swaps;
}

function flowChartSegmentForFlow(flow: TransactionFlow): FlowChartSegment | null {
  if (flow === "incoming" || flow === "outgoing") return flow;
  if (flow === "transfer" || flow === "layer-transition") return "transfers";
  if (flow === "swap") return "swaps";
  return null;
}

function addFlowChartSegmentStats(
  stats: FlowChartSegmentStats,
  txn: Transaction,
) {
  const btc = transactionBtc(txn);
  const eur = txn.amount ?? 0;
  stats.count += 1;
  stats.btc += btc;
  stats.eur += eur;
  if (!txn.rate) stats.missingPrice += 1;
  if (txn.status === "review" || txn.status === "pending") stats.review += 1;
  if (txn.status === "failed") stats.failed += 1;
  if (!stats.largest || btc > stats.largest.btc) {
    stats.largest = {
      label: txn.counterparty || txn.wallet || txn.txnId,
      btc,
      eur,
    };
  }
}

function flowPointTotal(row: FlowChartPoint) {
  return flowPointEntries(row).reduce((sum, [, value]) => sum + Math.abs(value), 0);
}

function flowAxisDomain(
  rows: FlowChartPoint[],
  metric: FlowChartMetric,
): [number, number] {
  const maxAbs = Math.max(
    metric === "count" ? 1 : 0,
    ...rows.flatMap((row) =>
      flowPointEntries(row).map(([, value]) => Math.abs(value)),
    ),
  );
  if (maxAbs === 0) return [-1, 1];
  return [-maxAbs * 1.12, maxAbs * 1.12];
}

type FeeFilter = "all" | "with-fees";

const filterChipClassName =
  "inline-flex h-5 cursor-pointer items-center gap-1 rounded-md bg-gray-50 px-2 text-[10px] font-medium text-gray-600 ring-1 ring-inset ring-gray-500/10 sm:h-6 sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20";

const detailTabValues = [
  "details",
  "classify",
  "pricing",
  "tax",
  "linked",
  "ledger",
] as const;

function readTransactionDetailParams(): {
  transactionId: string | null;
  tab: string;
  rowId?: string | null;
} {
  if (typeof window === "undefined") {
    return { transactionId: null, tab: "details" };
  }
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab");
  const rowId = params.get("qrow");
  const target = {
    transactionId:
      params.get("tx") ?? params.get("transaction") ?? params.get("transactionId"),
    tab: detailTabValues.includes(tab as (typeof detailTabValues)[number])
      ? tab ?? "details"
      : "details",
  };
  return rowId ? { ...target, rowId } : target;
}

const quickFilterValues: TableQuickFilter[] = [
  "external_flow",
  "review_queue",
  "no_explorer_id",
  "missing_price",
  "failed_import",
];

/**
 * Read the wallet/quick-filter scope from the URL. Used by Wallet Detail
 * deep links ("Show all" / "Needs review") that arrive at
 * `/transactions?wallet=<label>&quick=review_queue#transactions-table`.
 */
function readTransactionScopeParams(): {
  wallet: string | null;
  quick: TableQuickFilter | null;
} {
  if (typeof window === "undefined") return { wallet: null, quick: null };
  const params = new URLSearchParams(window.location.search);
  const wallet = params.get("wallet");
  const quick = params.get("quick");
  return {
    wallet: wallet && wallet.trim() ? wallet : null,
    quick: quickFilterValues.includes(quick as TableQuickFilter)
      ? (quick as TableQuickFilter)
      : null,
  };
}

function updateTransactionDetailParams(
  transactionId: string | null,
  tab = "details",
  rowId?: string | null,
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
    if (rowId) {
      params.set("qrow", rowId);
    } else {
      params.delete("qrow");
    }
  } else {
    params.delete("tx");
    params.delete("transaction");
    params.delete("transactionId");
    params.delete("tab");
    params.delete("qrow");
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

function flowChartSelectionLabel(
  selection: FlowChartSelection,
  t: Translator,
) {
  const segmentLabel = selection.segment
    ? t(flowChartSegmentLabels[selection.segment])
    : t("transactions:selectionLabel.allFlows");
  return t("transactions:selectionLabel.flowChart", {
    bucket: selection.bucketLabel,
    segment: segmentLabel,
    mode: t(flowChartModeLabels[selection.mode]),
  });
}

// Returns an i18n key (resolved with t() at the call site).
function quickFilterLabel(filter: TableQuickFilter) {
  if (filter === "external_flow") return "transactions:quickFilter.externalFlow";
  if (filter === "review_queue") return "transactions:quickFilter.reviewQueue";
  if (filter === "no_explorer_id") return "transactions:quickFilter.missingExplorerLink";
  if (filter === "missing_price") return "transactions:quickFilter.missingPrice";
  return "transactions:quickFilter.failedImport";
}

function breakdownSelectionLabel(
  selection: BreakdownSelection,
  t: Translator,
) {
  return selection.dimension === "network"
    ? t("transactions:selectionLabel.network", { key: selection.key })
    : t("transactions:selectionLabel.walletSource", { key: selection.key });
}

function matchesFlowChartSelection(
  txn: Transaction,
  selection: FlowChartSelection,
  displayFlow: (txn: Transaction) => TransactionFlow,
) {
  const parsedDate = parseTransactionDate(txn.date);
  if (selection.bucketKey !== null) {
    if (!parsedDate) return false;
    const bucket = bucketTransactionDate(parsedDate, selection.period);
    if (bucket.key !== selection.bucketKey) return false;
  }

  const flow = displayFlow(txn);
  if (
    selection.mode === "external" &&
    flow !== "incoming" &&
    flow !== "outgoing"
  ) {
    return false;
  }

  if (selection.segment === null) return true;

  if (selection.segment === "transfers") {
    return flow === "transfer" || flow === "layer-transition";
  }
  if (selection.segment === "swaps") return flow === "swap";
  return flow === selection.segment;
}


export {
  PAGE_SIZE_OPTIONS,
  addFlowChartSegmentStats,
  attachmentRecordToItem,
  availablePeriodKeysForRecords,
  breakdownSelectionLabel,
  bucketTransactionDate,
  buildBreakdown,
  buildFlowChartRows,
  buildSwapCandidates,
  buildTransferCandidates,
  candidateReferenceReviewType,
  dashboardRecordsFromTxs,
  filterChipClassName,
  flowAxisDomain,
  flowBucketLabel,
  flowChartConfig,
  flowChartMetricLabels,
  flowChartModeLabels,
  flowChartSegmentForFlow,
  flowChartSegmentFromDataKey,
  flowChartSegmentLabels,
  flowChartSelectionLabel,
  flowColorForSegment,
  flowColors,
  flowPointSegmentValue,
  flowPointTotal,
  formatCountBarLabel,
  formatFlowTooltipValue,
  initialPeriodFromUrl,
  isAttachmentListQueryKeyForTransaction,
  isRedundantTransactionLabel,
  matchesFlowChartSelection,
  matchesTransactionDeepLink,
  pairRailLabel,
  periodKeys,
  periodLabels,
  quickFilterLabel,
  readTransactionDetailParams,
  readTransactionScopeParams,
  recordsForPeriod,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  sortTransactionsByDateDesc,
  sumByFlow,
  toDashboardTransaction,
  upsertAttachmentRecords,
  transactionRecords,
  updateTransactionDetailParams,
};

export type {
  AttachmentsCopyData,
  AttachmentOpenData,
  AttachmentRecord,
  AttachmentsListData,
  BreakdownSelection,
  FeeFilter,
  FlowChartClickData,
  FlowChartMetric,
  FlowChartMode,
  FlowChartPoint,
  FlowChartSegment,
  FlowChartSegmentStats,
  FlowChartSelection,
  JournalEventsData,
  PeriodKey,
  SwapCandidate,
  TableQuickFilter,
};
