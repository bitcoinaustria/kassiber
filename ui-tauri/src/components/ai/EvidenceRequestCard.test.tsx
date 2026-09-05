import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { AssistantSessionContext, type AssistantSessionContextValue } from "./assistantSession";
import { ChatMessage } from "./ChatMessage";
import { EvidenceRequestCard } from "./EvidenceRequestCard";
import type { EvidenceRequest, EvidenceRequestStatus } from "./evidenceRequest";

const request: EvidenceRequest = { schema_version: 1, request_id: "a".repeat(64),
  workspace_id: "workspace", profile_id: "profile", input_version: 4,
  action: "attach_evidence", explanation: null,
  cases: [{ case_id: "quarantine:tx", transaction_id: "tx", reason: "custody_gap" }] };
function render(status: EvidenceRequestStatus, otherActive = false) {
  const session = { evidenceRequests: { [request.request_id]: { status },
    ...(otherActive ? { other: { status: "opening" } } : {}) }, openEvidenceRequest: vi.fn(),
    continueEvidenceRequest: vi.fn(), isStreaming: false } as unknown as AssistantSessionContextValue;
  return renderToStaticMarkup(<AssistantSessionContext.Provider value={session}>
    <EvidenceRequestCard request={request} />
  </AssistantSessionContext.Provider>);
}
describe("EvidenceRequestCard", () => {
  it("names one action and explains that analysis does not import or resolve", () => {
    const html = render("idle");
    expect(html).toContain("Choose evidence file");
    expect(html).toContain("I don&#x27;t have this");
    expect(html).toContain("This does not import transactions or resolve the case");
    expect(html).not.toContain("quarantine:tx");
    expect(html).not.toContain("ui.review.request_input");
  });
  it("offers resume after actual partial success and blocks duplicate or stale actions", () => {
    expect(render("partial")).toContain("Some changes were saved");
    expect(render("received")).toContain("Continue review");
    for (const state of ["opening", "continuing", "stale"] as const) expect(render(state)).toContain('disabled=""');
    expect(render("idle", true)).toContain('disabled=""');
  });
  it("shows the request directly, outside collapsed tool details", () => {
    const html = renderToStaticMarkup(<ChatMessage message={{ id: "message", role: "assistant", content: "", status: "done",
      toolCalls: [{ callId: "call", name: "ui.review.request_input", status: "done", kindClass: "read_only",
        needsConsent: false, arguments: {}, result: { kind: "ui.review.request_input", data: request } }] }} />);
    expect(html).toContain("Choose evidence file");
    expect(html).not.toContain("Tool usage");
  });
});
