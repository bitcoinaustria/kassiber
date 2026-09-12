import { describe, expect, it } from "vitest";

import {
  amountInput,
  linkReviewFormFromLink,
  linkReviewPayload,
  isStaleLinkReviewError,
  type SourceFundsLink,
} from "./model";

function link(overrides: Partial<SourceFundsLink> = {}): SourceFundsLink {
  return {
    id: "link-1",
    to_transaction_id: "in",
    link_type: "self_transfer",
    state: "suggested",
    confidence: "strong",
    method: "custody_component",
    asset: "BTC",
    allocation_amount: 0.00000000001,
    allocation_amount_msat: 1234,
    from_allocation_amount: 0.00000000001,
    from_allocation_amount_msat: 1234,
    allocation_policy: "heuristic",
    explanation: "",
    uses_chain_observation: false,
    ...overrides,
  };
}

describe("exact source-funds amounts", () => {
  it("renders sub-satoshi millisatoshis without rounding them away", () => {
    // 1234 msat is 0.00000001234 BTC; toFixed(8) would render it as zero.
    expect(amountInput(1234)).toBe("0.00000001234");
    expect(amountInput(null)).toBe("");
    expect(amountInput("")).toBe("");
  });

  it("builds the edit form from exact millisatoshis, not the float projection", () => {
    expect(linkReviewFormFromLink(link()).allocation_amount).toBe("0.00000001234");
  });
});

describe("reviewing is not editing", () => {
  it("omits untouched amounts so the stored value survives an approval", () => {
    const current = link();
    const payload = linkReviewPayload({
      link: current,
      form: linkReviewFormFromLink(current),
      state: "reviewed",
    });
    expect(payload).not.toHaveProperty("allocation_amount");
    expect(payload).not.toHaveProperty("from_allocation_amount");
    expect(payload.allocation_policy).toBe("explicit");
    // Bound to the amounts that were actually inspected.
    expect(payload.expected_allocation_amount_msat).toBe(1234);
    expect(payload.expected_from_allocation_amount_msat).toBe(1234);
  });

  it("sends a deliberate amount edit", () => {
    const current = link();
    const payload = linkReviewPayload({
      link: current,
      form: { ...linkReviewFormFromLink(current), allocation_amount: "0.00000002" },
      state: "reviewed",
    });
    expect(payload.allocation_amount).toBe("0.00000002");
    expect(payload).not.toHaveProperty("from_allocation_amount");
  });

  it("never lets a rejection carry an allocation edit", () => {
    const current = link();
    const payload = linkReviewPayload({
      link: current,
      form: {
        ...linkReviewFormFromLink(current),
        allocation_amount: "0.00000002",
        confidence: "weak",
      },
      state: "rejected",
    });
    expect(payload).not.toHaveProperty("allocation_amount");
    expect(payload).not.toHaveProperty("allocation_policy");
    // A confidence downgrade made in the same form is still a deliberate edit.
    expect(payload.confidence).toBe("weak");
  });

  it("recognizes a stale-inspection refusal", () => {
    expect(
      isStaleLinkReviewError({ envelope: { error: { code: "source_funds_link_stale" } } }),
    ).toBe(true);
    expect(isStaleLinkReviewError({ envelope: { error: { code: "validation" } } })).toBe(false);
    expect(isStaleLinkReviewError(undefined)).toBe(false);
  });
});
