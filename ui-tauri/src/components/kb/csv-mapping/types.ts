/** Daemon response shapes for the CSV mapping kinds. */

export interface CsvInspectResult {
  delimiter: string;
  encoding: string;
  headers: string[];
  sample_rows: Record<string, string>[];
  row_count_estimate: number;
}

export interface PreviewProblem {
  row: number;
  kind: "error" | "filtered";
  column: string | null;
  reason: string;
  detail: string | null;
}

export interface PreviewRecord {
  txid: string;
  occurred_at: string;
  direction: string;
  asset: string;
  amount: string;
  fee?: string;
  description?: string;
  counterparty?: string;
  kind?: string;
  fiat_currency?: string;
  fiat_rate?: string;
  fiat_value?: string;
}

export interface DetectedColumn {
  column: string;
  field: string;
}

export interface CsvPreviewResult {
  mapping_name?: string;
  rows_read: number;
  mapped: number;
  errors: number;
  filtered: number;
  problems: PreviewProblem[];
  preview: PreviewRecord[];
  truncated: boolean;
  headers: string[];
  /** Auto-detect outputs (absent when an explicit mapping was supplied). */
  confident?: boolean;
  detected?: DetectedColumn[] | null;
  mapping?: Record<string, unknown>;
}

export interface CsvExampleResult {
  csv: string;
  headers: string[];
  file?: string;
}

export interface ImportMappedResult {
  imported?: number;
  skipped?: number;
  mapped: number;
  errors: number;
  filtered: number;
  dry_run: boolean;
  input_format: string;
  wallet_id?: string;
  problems?: PreviewProblem[];
}

export interface WalletListItem {
  id: string;
  label: string;
  kind: string;
}

export interface WalletsListResult {
  wallets: WalletListItem[];
}

/** Map the UI language to a BCP-47 locale for number/date rendering. */
export function localeFor(lang: string): string {
  return lang.startsWith("de") ? "de-AT" : "en-US";
}
