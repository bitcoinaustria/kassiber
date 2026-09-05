import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import "@/i18n";
import type { AiChatMessage, AiToolConsentRequest } from "@/daemon/stream";
import { ChatMessage } from "./ChatMessage";
import { ReviewConsentBody, ToolConsentDialog } from "./ToolConsentDialog";
import { reviewApprovalAvailable, reviewToolResult } from "./reviewWorkflow";
vi.mock("@tanstack/react-router", () => ({ Link: ({ children, to }: ComponentProps<"a"> & { to: string }) => <a href={to}>{children}</a> }));
vi.mock("@/components/ui/dialog", () => {
  const Wrapper = ({ children, id, role }: ComponentProps<"div">) => <div id={id} role={role}>{children}</div>;
  return { Dialog: Wrapper, DialogContent: Wrapper, DialogHeader: Wrapper, DialogTitle: Wrapper, DialogDescription: Wrapper };
});

const before = { entries_count: 3, quarantine_count: 2, report_ready: false,
  quarantines: [{ transaction_id: "held-1", reason: "missing_price" }], wallet_holdings: [], journal_digest: "before" };
const after = { ...before, entries_count: 5, quarantine_count: 1,
  quarantines: [{ transaction_id: "held-2", reason: "missing_basis" }], journal_digest: "after" };
const artifact = { schema_version: 1, workspace_id: "workspace-1", profile_id: "book-1", base_input_version: 8,
  digest: "a".repeat(64), before, after,
  operations: [{ type: "price_override", transaction_id: "held-1", fiat_rate: "63000.00000001", reason: "Reviewed invoice" }] };
const receipt = { schema_version: 1, id: "receipt-1", status: "verified", artifact_digest: artifact.digest,
  result_input_version: 9, operations: [{ type: "price_override", result: {} }],
  before, verification: after, created_at: "2026-09-05T10:00:00Z" };
function request(reviewPreview?: AiToolConsentRequest["reviewPreview"]): AiToolConsentRequest {
  return { targetRequestId: "chat-1", callId: "apply-1", name: "ui.review.apply", summary: "Apply",
    argumentsPreview: { artifact: { ...artifact, operations: [{ type: "exclude", transaction_id: "FORGED", reason: "Model claim" }] } }, reviewPreview };
}

describe("review workflow chat integration", () => {
  it("shows server plan and remaining issues outside collapsed tool details", () => {
    const message: AiChatMessage = { id: "assistant-1", role: "assistant", content: "", status: "done", toolCalls: [{
      callId: "plan-1", name: "ui.review.plan", arguments: { secretArgument: true },
      kindClass: "read_only", needsConsent: false, status: "done", result: { kind: "ui.review.plan", data: artifact },
    }] };
    const markup = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(markup).toContain('aria-label="Proposed corrections"');
    expect(markup).toContain("63000.00000001");
    expect(markup).toContain("1 quarantine issue remains");
    expect(markup).toContain("Computed by Kassiber");
    expect(markup).toContain("Proposed interpretation");
    expect(markup).not.toContain("secretArgument");
  });

  it("blocks missing, unavailable and malformed previews without trusting model arguments", () => {
    for (const preview of [undefined, { status: "unavailable" as const, code: "review_plan_stale" }, { status: "ready" as const, artifact: {} }]) {
      expect(reviewApprovalAvailable(preview)).toBe(false);
      const markup = renderToStaticMarkup(<ReviewConsentBody request={request(preview)} />);
      expect(markup).toContain("could not verify this plan");
      expect(markup).not.toContain("FORGED");
      expect(markup).not.toContain("63000.00000001");
    }
  });

  it("reviews the authoritative ready preview and permits an idempotent receipt response", () => {
    expect(reviewApprovalAvailable({ status: "ready", artifact })).toBe(true);
    const markup = renderToStaticMarkup(<ReviewConsentBody request={request({ status: "ready", artifact })} />);
    expect(markup).toContain("held-1");
    expect(markup).not.toContain("FORGED");
    expect(reviewApprovalAvailable({ status: "applied", receipt })).toBe(true);
    expect(renderToStaticMarkup(<ReviewConsentBody request={request({ status: "applied", receipt })} />))
      .toContain("without making new changes");
  });

  it("offers one critical decision and disables apply when the server preview is unavailable", () => {
    const unavailable = renderToStaticMarkup(<ToolConsentDialog request={request()} onDecision={() => {}} />);
    expect(unavailable).toContain('role="alertdialog"');
    expect(unavailable).toMatch(/<button[^>]*disabled=""[^>]*>Apply once<\/button>/);
    expect(unavailable).not.toContain("Allow this session");
    const ready = renderToStaticMarkup(<ToolConsentDialog request={request({ status: "ready", artifact })} onDecision={() => {}} />);
    expect(ready).not.toMatch(/<button[^>]*disabled=""[^>]*>Apply once<\/button>/);
    expect(ready.match(/>Apply once<\/button>/g)).toHaveLength(1);
  });

  it("shows one verified receipt for repeated retrieval and preserves its blocked report status", () => {
    const message: AiChatMessage = { id: "assistant-1", role: "assistant", content: "", status: "done", toolCalls:
      ["ui.review.apply", "ui.review.receipt"].map((kind, index) => ({
        callId: `receipt-${index}`, name: kind, arguments: {}, kindClass: "read_only", needsConsent: false,
        status: "done", result: { kind, data: receipt },
      })) };
    const markup = renderToStaticMarkup(<ChatMessage message={message} />);
    expect(markup.match(/aria-label="Verified review receipt"/g)).toHaveLength(1);
    expect(markup).toContain("Blocked");
    expect(markup).toContain("1 quarantine issue remains");
  });

  it("does not treat a model-supplied artifact or a failed tool envelope as a successful plan", () => {
    const call = { callId: "bad", name: "ui.review.plan", arguments: { artifact }, kindClass: "read_only" as const,
      needsConsent: false, status: "error" as const, result: { kind: "ui.review.plan", data: artifact } };
    expect(reviewToolResult(call)).toBeNull();
    expect(reviewToolResult({ ...call, status: "done", result: undefined })).toBeNull();
  });

  it("does not guess BTC when a custody leg inherits its asset from a transaction", () => {
    const custodyArtifact = { ...artifact, operations: [{ type: "custody_component", request: { action: "create",
      components: [{ component_type: "manual_bridge", legs: [{ role: "source", amount_msat: "123456", transaction: "liquid-tx" }] }] } }] };
    const markup = renderToStaticMarkup(<ReviewConsentBody request={request({ status: "ready", artifact: custodyArtifact })} />);
    expect(markup).toContain("123456 msat (asset resolved from reference)");
    expect(markup).not.toContain("123456 BTC");
  });
});
