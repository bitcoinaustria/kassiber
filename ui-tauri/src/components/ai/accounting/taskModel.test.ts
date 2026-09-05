import { describe, expect, it } from "vitest";
import { formatMinor } from "./model";
import { isTaskPreview } from "./taskModel";

const preview = {
  id: "task", period_id: "2025", state: "active", source_count: 1,
  expected_digest: "a".repeat(64), expected_revision: 3, ready: true,
  blockers: [], step: "prepare", proposals: [{
    source_id: "row", source_kind: "bank", payload: {
      description: "Synthetic", entry_date: "2025-01-01", lines: [
        { account_code: "bank", debit_minor: "100", credit_minor: "0" },
        { account_code: "sales", debit_minor: "0", credit_minor: "100" },
      ],
    },
  }],
};

function bitcoin(step: "prepare" | "post", zero = false) {
  const request = { policy_id: "policy", artifact_id: "artifact", binding_id: "binding", event_id: "source",
    category: "purchase", period_id: "2025", idempotency_key: "prepare" };
  const projection = { request, policy_digest: "b".repeat(64), valuation_release_digest: null,
    lines: zero ? [] : preview.proposals[0].payload.lines,
    quantitative_posting: { asset: "BTC", account_code: "btc", location: "inventory", quantity_msat: zero ? "1" : "100000000000",
      basis_exact: "1", book_value_minor: zero ? "0" : "100", currency_rounding: [{ account_code: "btc",
        before_basis_exact: "0", before_minor: "0", unrounded_event_minor: zero ? "0" : "100", remainder_minor: "0", dependencies_digest: "c".repeat(64) }] } };
  const row = { source_kind: "bitcoin", source_id: "source", request, projection,
    status: "draft", proposal_id: "proposal", proposal_digest: "d".repeat(64), artifact_digest: "e".repeat(64), policy_digest: projection.policy_digest };
  return { ...preview, step, proposals: step === "prepare" ? [row] : [], detail: { entries: [], projections: step === "post" ? [row] : [] } };
}

describe("minimal local accounting review", () => {
  it("rejects a ready preparation without exact proposals or valid authority metadata", () => {
    expect(isTaskPreview(preview)).toBe(true);
    for (const change of [
      { proposals: undefined }, { proposals: [] }, { expected_digest: "bad" },
      { expected_revision: -1 }, { source_count: -1 }, { blockers: undefined },
      { blockers: [{ kind: "not_ready" }] },
    ]) expect(isTaskPreview({ ...preview, ...change })).toBe(false);
  });

  it("requires exact tax forms and export hashes", () => {
    expect(isTaskPreview({ ...preview, step: "tax_finalize", detail: { forms: [] } })).toBe(false);
    expect(isTaskPreview({ ...preview, step: "export_close", detail: { id: "close", snapshot_digest: "bad" } })).toBe(false);
    expect(isTaskPreview({ ...preview, step: "export_close", detail: { id: "close", snapshot_digest: "b".repeat(64) } })).toBe(true);
  });

  it("renders large and negative minor units without floating point loss", () => {
    const eur = { currency: "EUR", minor_unit_exponent: 2 };
    expect(formatMinor("9007199254740997", eur)).toContain("90.071.992.547.409,97");
    expect(formatMinor("-101", eur)).toMatch(/^−.*1,01/);
    expect(formatMinor("100", { currency: "JPY", minor_unit_exponent: 0 })).toContain("100");
  });

  it("accepts exact Bitcoin prepare/post and quantity-only effects without inventing fiat entries", () => {
    for (const step of ["prepare", "post"] as const) for (const zero of [false, true]) {
      expect(isTaskPreview(bitcoin(step, zero))).toBe(true);
    }
    expect(isTaskPreview({ ...bitcoin("post", true), detail: { entries: [], projections: [] } })).toBe(false);
  });

  it("rejects malformed or mismatched Bitcoin wire effects before approval", () => {
    for (const step of ["prepare", "post"] as const) {
      for (const damage of ["quantity", "basis", "period", "event", "rounding", "policy", "emptyLines", "related", "asset"]) {
        const candidate = bitcoin(step);
        const row = step === "prepare" ? candidate.proposals[0] : candidate.detail.projections[0];
        const projected = row.projection as Record<string, unknown>;
        const amount = projected.quantitative_posting as Record<string, unknown>;
        if (damage === "quantity") amount.quantity_msat = 1;
        if (damage === "basis") amount.basis_exact = "NaN";
        if (damage === "period") row.request.period_id = "2026";
        if (damage === "event") row.request.event_id = "wrong";
        if (damage === "rounding") amount.currency_rounding = [];
        if (damage === "policy") projected.policy_digest = "bad";
        if (damage === "emptyLines") projected.lines = [];
        if (damage === "related") amount.related_postings = null;
        if (damage === "asset") amount.asset = ["BTC"];
        expect(isTaskPreview(candidate), `${step}:${damage}`).toBe(false);
      }
    }
    expect(isTaskPreview({ ...bitcoin("post"), detail: { entries: preview.proposals.map((row) => row.payload), projections: null } })).toBe(false);
    const post = bitcoin("post", true);
    post.detail.projections[0].proposal_digest = "bad";
    expect(isTaskPreview(post)).toBe(false);
  });
});
