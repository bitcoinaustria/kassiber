import type { AiChatToolCall } from "@/daemon/stream";
import type { ExpectedBookScope } from "@/daemon/client";

export type EvidenceAction = "connect_wallet" | "import_history" | "attach_evidence";
export interface EvidenceRequest extends ExpectedBookScope {
  schema_version: 1;
  request_id: string;
  input_version: number;
  action: EvidenceAction;
  explanation: string | null;
  cases: { case_id: string; transaction_id: string; reason: string; wallet_id?: string | null }[];
}
export type EvidenceRequestStatus = "idle" | "opening" | "received" | "partial" | "continuing" | "stale" | "error" | "unavailable";
export interface EvidenceRequestState { status: EvidenceRequestStatus; error?: string }
export interface HandoffStamp { generation: number; daemonSession: number; promptRevision: number }

export function sameHandoffContext(origin: HandoffStamp, current: HandoffStamp): boolean {
  return origin.generation === current.generation && origin.daemonSession === current.daemonSession;
}
export function canAutoContinueEvidence(origin: HandoffStamp, current: HandoffStamp,
  busy: boolean, hasDraft: boolean, queued: boolean): boolean {
  return sameHandoffContext(origin, current) && origin.promptRevision === current.promptRevision
    && !busy && !hasDraft && !queued;
}

interface PendingHandoffIdentity { request: { request_id: string }; origin: HandoffStamp; outcome?: string }
export function canStartEvidenceHandoff(requestId: string, pending: PendingHandoffIdentity | null, continued: ReadonlySet<string>): boolean {
  return pending === null && !continued.has(requestId);
}
export function canResumeEvidenceHandoff(requestId: string, pending: PendingHandoffIdentity | null, continued: ReadonlySet<string>): boolean {
  return !continued.has(requestId) && (!pending || pending.request.request_id === requestId && Boolean(pending.outcome));
}
/** Identity comparison drops a cancelled/replaced async callback even in one chat. */
export function isActiveEvidenceHandoff<T extends PendingHandoffIdentity>(pending: T | null, expected: T, current: HandoffStamp): boolean {
  return pending === expected && sameHandoffContext(expected.origin, current);
}

const record = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : null;

/** Only completed server packets can produce local UI actions. */
export function evidenceRequest(tool: AiChatToolCall): EvidenceRequest | null {
  if (tool.name !== "ui.review.request_input" || tool.status !== "done") return null;
  const envelope = record(tool.result);
  const data = record(envelope?.data);
  if (envelope?.kind !== "ui.review.request_input" || !data || data.schema_version !== 1
    || typeof data.request_id !== "string" || !/^[a-f0-9]{64}$/.test(data.request_id)
    || typeof data.workspace_id !== "string" || !data.workspace_id
    || typeof data.profile_id !== "string" || !data.profile_id
    || !Number.isSafeInteger(data.input_version) || Number(data.input_version) < 0
    || !["connect_wallet", "import_history", "attach_evidence"].includes(String(data.action))
    || !Array.isArray(data.cases) || !data.cases.length || data.cases.length > 20
    || (data.action === "attach_evidence" && data.cases.length !== 1)
    || !(data.explanation === null || typeof data.explanation === "string" && data.explanation.length <= 1000)
    || !data.cases.every((value) => { const item = record(value); return item
      && typeof item.case_id === "string" && item.case_id.startsWith("quarantine:")
      && typeof item.transaction_id === "string" && item.transaction_id.length > 0
      && typeof item.reason === "string"; })) return null;
  return data as unknown as EvidenceRequest;
}
