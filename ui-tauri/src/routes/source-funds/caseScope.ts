import type { ExpectedBookScope } from "@/daemon/client";
import type { EvidenceAttachment, SourceFundsLink, SourceFundsPreview, SourceFundsSource } from "./model";

export interface SourceFundsReviewContext {
  schema_version: number;
  workspace_id: string;
  profile_id: string;
  input_version: number;
  review_fingerprint: string;
  case_id: string;
  target: { transaction_id: string; wallet_id: string; direction: string; asset: string; occurred_at: string };
  recipe: Record<string, unknown>;
  report: SourceFundsPreview;
  links: SourceFundsLink[];
  sources: SourceFundsSource[];
  evidence: EvidenceAttachment[];
  scope_truncated?: boolean;
}

/** A database path alone does not distinguish profiles inside an imported book. */
export function sourceFundsDraftKey(databaseIdentity: string, scope: ExpectedBookScope): string {
  return JSON.stringify(["source-funds-v2", databaseIdentity, scope.workspace_id, scope.profile_id]);
}

export function targetQueryArgs(filters: { query: string; flow: string; date: string; status: string; network: string; asset: string; wallet: string }, now = new Date()): Record<string, unknown> {
  const args: Record<string, unknown> = { limit: 100, sort: "occurred-at", order: "desc" };
  if (filters.query.trim()) args.query = filters.query.trim();
  for (const key of ["flow", "status", "network", "asset", "wallet"] as const) {
    if (filters[key] !== "all") args[key] = filters[key];
  }
  const start = new Date(now); start.setHours(0, 0, 0, 0);
  const days = filters.date === "yesterday" ? 1 : filters.date === "7days" ? 7 : filters.date === "30days" || filters.date === "older" ? 30 : 0;
  start.setDate(start.getDate() - days);
  if (filters.date === "older") args.until = new Date(start.getTime() - 1).toISOString();
  else if (filters.date !== "all") {
    args.since = start.toISOString();
    if (filters.date === "yesterday") { const end = new Date(start); end.setDate(end.getDate() + 1); args.until = new Date(end.getTime() - 1).toISOString(); }
  }
  return args;
}
