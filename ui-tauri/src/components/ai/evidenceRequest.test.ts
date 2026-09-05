import { describe, expect, it } from "vitest";
import { canAutoContinueEvidence, evidenceRequest, evidenceRevalidationRequest, evidenceAttachmentOptions, evidenceContinuationScreenContext, sameHandoffContext, canStartEvidenceHandoff, canResumeEvidenceHandoff, isActiveEvidenceHandoff } from "./evidenceRequest";
import type { AiChatToolCall } from "@/daemon/stream";

export const requestPacket = {
  schema_version: 1, request_id: "a".repeat(64), workspace_id: "workspace", profile_id: "profile",
  input_version: 4, action: "connect_wallet", explanation: "The intermediate wallet is missing.",
  cases: [{ case_id: "quarantine:tx-1", transaction_id: "tx-1", reason: "custody_gap", wallet_id: "wallet" }],
};
export const requestCall: AiChatToolCall = {
  callId: "call-1", name: "ui.review.request_input", arguments: {}, kindClass: "read_only",
  status: "done", needsConsent: false,
  result: { kind: "ui.review.request_input", schema_version: 1, data: requestPacket },
};

describe("evidence handoff boundary", () => {
  it("revalidates source-of-funds recipe and carries its fingerprint through native staging", () => {
    const data = { ...requestPacket, domain: "source_funds", action: "attach_evidence",
      target_transaction: "tx-1", recipe: { target_amount: "0.01", reveal_mode: "standard" },
      review_fingerprint: "b".repeat(64), cases: [{ ...requestPacket.cases[0],
        case_id: "source_funds:tx-1", reason: "evidence_review" }] };
    const call = { ...requestCall, name: "ui.source_funds.request_input",
      result: { kind: "ui.source_funds.request_input", data } };
    const parsed = evidenceRequest(call)!;
    expect(parsed).not.toBeNull();
    expect(evidenceRevalidationRequest(parsed)).toEqual({ kind: "ui.source_funds.request_input", args: {
      action: "attach_evidence", target_transaction: "tx-1", recipe: data.recipe,
      expected_review_fingerprint: data.review_fingerprint, explanation: data.explanation,
      expected_scope: { workspace_id: "workspace", profile_id: "profile" },
    } });
    expect(evidenceAttachmentOptions(parsed)).toMatchObject({ review_case_id: "source_funds:tx-1",
      review_recipe: data.recipe, expected_review_fingerprint: data.review_fingerprint });
    expect(evidenceContinuationScreenContext(parsed)).toEqual({ route: "/source-of-funds",
      capabilities: ["source_funds"], entityType: "transaction", entityId: "tx-1",
      filters: { source_funds_recipe: data.recipe } });
    expect(evidenceContinuationScreenContext(evidenceRequest(requestCall)!)).toBeNull();
    for (const patch of [{ domain: undefined }, { target_transaction: "tx-2" },
      { review_fingerprint: "bad" }, { recipe: null }, { cases: requestPacket.cases }]) {
      expect(evidenceRequest({ ...call, result: { ...call.result, data: { ...data, ...patch } } })).toBeNull();
    }
    expect(evidenceRequest({ ...requestCall, result: { kind: "ui.review.request_input", data } })).toBeNull();
  });
  it("accepts only a completed, typed server input request", () => {
    expect(evidenceRequest(requestCall)?.action).toBe("connect_wallet");
    for (const patch of [
      { name: "ui.transactions.list" }, { status: "running" as const },
      { result: { kind: "ui.review.request_input", data: { ...requestPacket, action: "open_url" } } },
      { result: { kind: "ui.review.request_input", data: { ...requestPacket, request_id: "invented" } } },
      { result: { kind: "ui.review.request_input", data: { ...requestPacket, cases: [] } } },
    ]) expect(evidenceRequest({ ...requestCall, ...patch })).toBeNull();
  });

  it("automatically continues actual success only in the original idle conversation", () => {
    const origin = { generation: 2, daemonSession: 10, promptRevision: 3 };
    expect(canAutoContinueEvidence(origin, { ...origin }, false, false, false)).toBe(true);
    // A different book/profile advances daemonSession; reset/branch/edit/resume
    // advances generation even when both chats have sessionId=null.
    for (const current of [
      { ...origin, daemonSession: 11 }, { ...origin, generation: 3 },
      { ...origin, promptRevision: 4 },
    ]) expect(canAutoContinueEvidence(origin, current, false, false, false)).toBe(false);
    for (const busy of [[true, false, false], [false, true, false], [false, false, true]]) {
      expect(canAutoContinueEvidence(origin, origin, ...busy as [boolean, boolean, boolean])).toBe(false);
    }
    expect(sameHandoffContext(origin, { ...origin, promptRevision: 4 })).toBe(true);
  });
  it("drops cancelled or replaced picker completions and prevents duplicate launches", () => {
    const origin = { generation: 1, daemonSession: 2, promptRevision: 3 };
    const pending = { request: { request_id: "A" }, origin };
    expect(canStartEvidenceHandoff("A", null, new Set())).toBe(true);
    expect(canStartEvidenceHandoff("A", pending, new Set())).toBe(false);
    expect(canStartEvidenceHandoff("B", pending, new Set())).toBe(false);
    expect(isActiveEvidenceHandoff(pending, pending, origin)).toBe(true);
    expect(isActiveEvidenceHandoff(null, pending, origin)).toBe(false); // cancelled picker
    expect(isActiveEvidenceHandoff({ ...pending }, pending, origin)).toBe(false); // reopened picker
    expect(isActiveEvidenceHandoff(pending, pending, { ...origin, generation: 2 })).toBe(false);
    expect(isActiveEvidenceHandoff(pending, pending, { ...origin, daemonSession: 3 })).toBe(false);
    expect(canStartEvidenceHandoff("A", null, new Set(["A"]))).toBe(false);
  });

  it("cannot consume another card's pending outcome or attachment", () => {
    const pending = { request: { request_id: "B" },
      origin: { generation: 1, daemonSession: 2, promptRevision: 3 }, outcome: "attached" };
    expect(canResumeEvidenceHandoff("A", pending, new Set())).toBe(false);
    expect(canResumeEvidenceHandoff("B", pending, new Set())).toBe(true);
    expect(canResumeEvidenceHandoff("B", { ...pending, outcome: undefined }, new Set())).toBe(false);
    expect(canResumeEvidenceHandoff("B", pending, new Set(["B"]))).toBe(false);
  });

});
