export type TransactionHistorySource = "cli" | "gui" | "ai_tool";
export type TransactionHistoryFamily = "metadata" | "tax" | "pricing";

export type TransactionHistoryField = {
  id: string;
  field: string;
  label: string;
  family: TransactionHistoryFamily;
  before_value: unknown;
  after_value: unknown;
  before_label: string;
  after_label: string;
  diff?: {
    added?: string[];
    removed?: string[];
    before?: string[];
    after?: string[];
  };
  redacted?: boolean;
};

export type TransactionHistoryEvent = {
  id: string;
  transaction_id: string;
  transaction_external_id?: string;
  transaction_occurred_at?: string;
  wallet_id?: string;
  wallet_label?: string;
  source: TransactionHistorySource;
  source_label: string;
  reason?: string;
  changed_at: string;
  summary: string;
  families: TransactionHistoryFamily[];
  fields: TransactionHistoryField[];
  report_anchor?: {
    stale_for_reports?: boolean;
    journal_input_version_after?: number;
    last_processed_at?: string | null;
  };
  transaction?: {
    id: string;
    external_id?: string;
    occurred_at?: string;
    direction?: string;
    asset?: string;
    amount?: number | null;
    amount_msat?: number | null;
    fee?: number | null;
    fee_msat?: number | null;
    counterparty?: string;
  };
};

export type TransactionHistoryList = {
  events: TransactionHistoryEvent[];
  next_cursor?: string | null;
  has_more: boolean;
  limit: number;
  stale?: TransactionHistoryStaleSummary;
};

export type TransactionHistoryStaleSummary = {
  edit_count: number;
  latest_changed_at?: string | null;
  source_counts?: Record<string, number>;
  family_counts?: Record<string, number>;
  field_counts?: Record<string, number>;
  last_processed_at?: string | null;
  last_processed_input_version?: number;
};

export type HistoryRevertTarget = {
  event: TransactionHistoryEvent;
  field?: TransactionHistoryField;
};

export function formatHistoryDate(value: string | null | undefined) {
  if (!value) return "Unknown time";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function formatHistoryRelative(value: string | null | undefined) {
  if (!value) return "Unknown";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const deltaMs = date.getTime() - Date.now();
  const absMs = Math.abs(deltaMs);
  const units: Array<[Intl.RelativeTimeFormatUnit, number]> = [
    ["day", 86_400_000],
    ["hour", 3_600_000],
    ["minute", 60_000],
  ];
  const formatter = new Intl.RelativeTimeFormat(currentUiLocale(), {
    numeric: "auto",
  });
  for (const [unit, size] of units) {
    if (absMs >= size) {
      return formatter.format(Math.round(deltaMs / size), unit);
    }
  }
  return "just now";
}

export function transactionHistorySourceClass(source: string) {
  if (source === "ai_tool") {
    return "border-sky-300 bg-sky-50 text-sky-700 dark:border-sky-800 dark:bg-sky-950/50 dark:text-sky-300";
  }
  if (source === "gui") {
    return "border-emerald-300 bg-emerald-50 text-emerald-700 dark:border-emerald-800 dark:bg-emerald-950/50 dark:text-emerald-300";
  }
  return "border-zinc-300 bg-zinc-50 text-zinc-700 dark:border-zinc-700 dark:bg-zinc-900 dark:text-zinc-300";
}

export function transactionHistoryFamilyClass(family: string) {
  if (family === "pricing") {
    return "border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-300";
  }
  if (family === "tax") {
    return "border-violet-300 bg-violet-50 text-violet-700 dark:border-violet-800 dark:bg-violet-950/40 dark:text-violet-300";
  }
  return "border-slate-300 bg-slate-50 text-slate-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300";
}

export function historyTransactionLabel(event: TransactionHistoryEvent) {
  return (
    event.transaction_external_id ||
    event.transaction?.external_id ||
    event.transaction_id
  );
}
import { currentUiLocale } from "@/lib/localeFormat";
