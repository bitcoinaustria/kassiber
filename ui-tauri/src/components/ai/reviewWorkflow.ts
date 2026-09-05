import type { AiChatToolCall, AiReviewPreview } from "@/daemon/stream";

export type ReviewRecord = Record<string, unknown>;
export interface ReviewEffects extends ReviewRecord {
  entries_count: number;
  quarantine_count: number;
  report_ready: boolean;
  quarantines: unknown[];
}
export interface ReviewArtifact extends ReviewRecord {
  schema_version: number;
  workspace_id: string;
  profile_id: string;
  base_input_version: number;
  digest: string;
  operations: ReviewRecord[];
  before: ReviewEffects;
  after: ReviewEffects;
}
export interface ReviewReceipt extends ReviewRecord {
  id: string;
  status: "verified";
  artifact_digest: string;
  result_input_version: number;
  operations: unknown[];
  verification: ReviewEffects;
  before?: ReviewEffects;
  created_at: string;
}

export function reviewRecord(value: unknown): ReviewRecord | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as ReviewRecord : null;
}
export function reviewText(value: unknown): string {
  return typeof value === "string" || typeof value === "number" ? String(value) : "";
}
function count(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}
function effects(value: unknown): value is ReviewEffects {
  const row = reviewRecord(value);
  return Boolean(row && count(row.entries_count) && count(row.quarantine_count) &&
    typeof row.report_ready === "boolean" && Array.isArray(row.quarantines));
}
export function reviewArtifact(value: unknown): ReviewArtifact | null {
  const row = reviewRecord(value);
  return row?.schema_version === 1 && typeof row.workspace_id === "string" &&
    typeof row.profile_id === "string" && count(row.base_input_version) &&
    typeof row.digest === "string" && /^[a-f0-9]{64}$/.test(row.digest) &&
    Array.isArray(row.operations) && row.operations.length > 0 && row.operations.length <= 50 &&
    row.operations.every((operation) => reviewRecord(operation) &&
      ["price_override", "exclude", "custody_component"].includes(String(operation.type))) &&
    effects(row.before) && effects(row.after) ? row as ReviewArtifact : null;
}
export function reviewReceipt(value: unknown): ReviewReceipt | null {
  const row = reviewRecord(value);
  return row?.schema_version === 1 && row.status === "verified" &&
    typeof row.id === "string" && typeof row.artifact_digest === "string" &&
    typeof row.created_at === "string" && count(row.result_input_version) &&
    Array.isArray(row.operations) && effects(row.verification) &&
    (row.before === undefined || effects(row.before)) ? row as ReviewReceipt : null;
}

/** Approval depends only on the daemon's revalidated preview, never arguments. */
export function reviewApprovalAvailable(preview: AiReviewPreview | undefined): boolean {
  return preview?.status === "ready" ? reviewArtifact(preview.artifact) !== null
    : preview?.status === "applied" && reviewReceipt(preview.receipt) !== null;
}

export function reviewToolResult(toolCall: AiChatToolCall) {
  if (toolCall.status !== "done") return null;
  const envelope = reviewRecord(toolCall.result);
  if (envelope?.kind === "ui.review.plan") {
    const artifact = reviewArtifact(envelope.data);
    return artifact ? { kind: "plan" as const, artifact } : null;
  }
  if (envelope?.kind === "ui.review.apply" || envelope?.kind === "ui.review.receipt") {
    const receipt = reviewReceipt(envelope.data);
    return receipt ? { kind: "receipt" as const, receipt } : null;
  }
  return null;
}
