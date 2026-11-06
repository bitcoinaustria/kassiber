import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";
import type { AiChatToolCall, AiToolConsentRequest } from "@/daemon/stream";
import { aiToolAllowsSessionConsent } from "@/daemon/stream";
import { AssistantSessionContext, type AssistantSessionContextValue } from "./assistantSession";
import { PendingApprovalRow } from "./PendingApprovalRow";
import { PlanConsentBody } from "./PlanConsentBody";
import { planApprovalAvailable, planConsentPreview } from "./planConsent";
import { walletInputRequest } from "./walletInputRequest";

const plan = {
  ready: true,
  plan_digest: "a".repeat(64),
  target: { action: "create", label: "Broker XYZ", completeness_class: "manual" },
  rows: { in_file: 3, to_import: 2, already_present: 1, would_update_existing: 0 },
  date_range: { first: "2024-03-01T10:00:00Z", last: "2024-04-02T10:00:00Z", undated: 0 },
  totals: [{ asset: "BTC", direction: "inbound", rows: 1, amount_msat: 50_000_000_000, fee_msat: 10_000_000 }],
  overlap: { existing_rows_in_date_range: 0, external_ids_in_other_wallets: 0 },
  warnings: ["type_map_decides_tax_kind"],
  blockers: [],
};

function request(preview: Record<string, unknown>): AiToolConsentRequest {
  return {
    targetRequestId: "chat-1", callId: "import-1", name: "ui.wallets.import_attachment.apply",
    summary: "Import 2 row(s) from the attached file into new wallet Broker XYZ", argumentsPreview: preview,
    reversible: true, egresses: false, canAllowSession: false,
  };
}

function approvalRow(request: AiToolConsentRequest) {
  const session = { sendConsent: vi.fn(), abort: vi.fn() } as unknown as AssistantSessionContextValue;
  return renderToStaticMarkup(
    <AssistantSessionContext.Provider value={session}>
      <PendingApprovalRow request={request} />
    </AssistantSessionContext.Provider>,
  );
}

describe("plan-bound consent", () => {
  it("shows the daemon's import plan and allows one approval", () => {
    const preview = planConsentPreview("ui.wallets.import_attachment.apply", { status: "ready", plan });
    expect(planApprovalAvailable(preview)).toBe(true);
    const markup = renderToStaticMarkup(<PlanConsentBody preview={preview} />);
    expect(markup).toContain("new connection Broker XYZ");
    expect(markup).toContain("0.5 BTC");
    expect(markup).toContain("2024-03-01T10:00:00Z");
    expect(markup).toContain("translates the file");
    expect(aiToolAllowsSessionConsent("ui.wallets.import_attachment.apply")).toBe(false);
    expect(aiToolAllowsSessionConsent("ui.imports.rollback")).toBe(false);
    const row = approvalRow(request({ status: "ready", plan }));
    expect(row).toContain("Import 2 row(s)");
    expect(row).toContain("recalculated this plan");
    expect(row).toMatch(/<button[^>]*>Import once<\/button>/);
    expect(row).not.toMatch(/<button[^>]*disabled=""[^>]*>Import once<\/button>/);
  });

  it("refuses stale, blocked and unavailable previews", () => {
    for (const status of ["stale", "blocked", "unavailable", "forged"]) {
      const preview = planConsentPreview("ui.wallets.import_attachment.apply", { status, plan });
      expect(planApprovalAvailable(preview)).toBe(false);
    }
    const blockedPlan = { ...plan, ready: false, blockers: [{ code: "amount_units_unconfirmed", message: "x", remediation: { desktop_only: true } }] };
    const blocked = planConsentPreview("ui.wallets.import_attachment.apply", {
      status: "blocked",
      plan: blockedPlan,
    });
    // The row offers no approval for a plan that is not ready.
    expect(approvalRow(request({ status: "blocked", plan: blockedPlan })))
      .toMatch(/<button[^>]*disabled=""[^>]*>Import once<\/button>/);
    const markup = renderToStaticMarkup(<PlanConsentBody preview={blocked} />);
    expect(markup).toContain("does not say which unit");
    expect(markup).toContain("desktop or CLI");
    expect(renderToStaticMarkup(<PlanConsentBody preview={planConsentPreview("ui.imports.rollback", { status: "stale" })} />))
      .toContain("changed since this plan");
  });

  it("says when a restricted provider's plan withholds the file's values", () => {
    const preview = planConsentPreview("ui.wallets.import_attachment.apply", {
      status: "ready", plan: { ...plan, date_range: null, totals: [{ asset: "BTC", direction: "inbound", rows: 1 }] },
    });
    const markup = renderToStaticMarkup(<PlanConsentBody preview={preview} />);
    expect(markup).toContain("Not shown to this provider");
    expect(markup).not.toContain("0.5 BTC");
  });

  it("reads a rollback plan with the reviewed work it would delete", () => {
    const preview = planConsentPreview("ui.imports.rollback", {
      status: "blocked",
      plan: {
        transactions_to_delete: 2, batch: { imported_at: "2026-10-01T00:00:00Z", source_format: "generic_ledger" },
        also_removed: { transaction_tags: 1 }, blockers: [{ code: "rollback_removes_reviewed_work", message: "" }],
      },
    });
    const markup = renderToStaticMarkup(<PlanConsentBody preview={preview} />);
    expect(markup).toContain("transaction_tags: 1");
    expect(markup).toContain("delete reviewed work");
  });
});

describe("wallet connection handoff", () => {
  const call = (data: Record<string, unknown>): AiChatToolCall => ({
    callId: "w-1", name: "ui.wallets.request_input", arguments: {}, kindClass: "read_only",
    needsConsent: false, status: "done", result: { kind: "ui.wallets.request_input", data },
  });
  const packet = {
    schema_version: 1, request_id: "b".repeat(64), workspace_id: "w", profile_id: "p",
    input_version: 3, action: "connect_wallet", source: "descriptor", explanation: null, desktop_only: true,
  };

  it("accepts only a well-formed server packet", () => {
    expect(walletInputRequest(call(packet))?.source).toBe("descriptor");
    expect(walletInputRequest(call({ ...packet, source: "custom" }))).toBeNull();
    expect(walletInputRequest(call({ ...packet, desktop_only: false }))).toBeNull();
    expect(walletInputRequest(call({ ...packet, request_id: "short" }))).toBeNull();
    expect(walletInputRequest({ ...call(packet), status: "running" })).toBeNull();
  });
});
