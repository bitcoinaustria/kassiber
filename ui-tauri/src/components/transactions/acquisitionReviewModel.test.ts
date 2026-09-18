import { describe, expect, it, vi } from "vitest";
import { planAcquisitionReview } from "./acquisitionReviewModel";
import { reviewArtifact } from "@/components/ai/reviewWorkflow";

const scope = { workspace_id: "w", profile_id: "p", input_version: 7 };
const effects = { entries_count: 1, quarantine_count: 0, quarantines: [], report_ready: true,
  accounting_totals: [{ asset: "BTC", income: "0.00000000001", acquisition_basis: "20000.123456789" }] };
const artifact = { schema_version: 1, workspace_id: "w", profile_id: "p", base_input_version: 7, digest: "a".repeat(64),
  operations: [{ type: "kind_override", transaction_id: "tx", kind: "income", reason: "review" }], before: effects, after: effects };
function args() { return { readScope: vi.fn(async () => scope), createPlan: vi.fn(async () => artifact), isCurrent: () => true,
  transactionId: "tx", kind: "income", invalidMessage: "Invalid preview" }; }

describe("acquisition preview scope and authority", () => {
  it("binds a single authored field without including another dirty draft", async () => {
    const input = args();
    expect(await planAcquisitionReview(input)).toEqual(artifact);
    expect(input.createPlan).toHaveBeenCalledWith({ expected_scope: { workspace_id: "w", profile_id: "p" }, expected_input_version: 7,
      operations: [{ type: "kind_override", transaction_id: "tx", kind: "income", reason: "Reviewed acquisition classification" }] });
  });
  it("stops before planning when the scope read returns after a book switch", async () => {
    const input = args(); input.isCurrent = () => false;
    expect(await planAcquisitionReview(input)).toBeNull();
    expect(input.createPlan).not.toHaveBeenCalled();
  });
  it("discards a delayed plan after the session changes", async () => {
    const input = args(); let current = true; input.isCurrent = () => current;
    input.createPlan.mockImplementation(async () => { current = false; return artifact; });
    expect(await planAcquisitionReview(input)).toBeNull();
  });
  it("rejects wrong-book and wrong-version responses", async () => {
    for (const changed of [{ profile_id: "other" }, { base_input_version: 8 }]) {
      const input = args(); input.createPlan.mockResolvedValue({ ...artifact, ...changed });
      await expect(planAcquisitionReview(input)).rejects.toThrow("Invalid preview");
    }
  });
  it("does not enable acquisition approval on the AI review surface", () => {
    expect(reviewArtifact(artifact)).toBeNull();
    expect(reviewArtifact(artifact, true)?.after.accounting_totals).toEqual(effects.accounting_totals);
  });
});
