import { describe, expect, it } from "vitest";

import {
  amountInput,
  linkReviewFormFromLink,
  linkReviewPayload,
  isStaleLinkReviewError,
  reconcileLinkForm,
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

describe("a refreshed link cannot be approved against its old amounts", () => {
  // Astra's reproduction: inspected at 1,000 msat, refreshed to 2,000, user
  // changes nothing. The old guard compared the pristine form to the NEW link,
  // saw a difference, kept the form, then submitted it against the new
  // precondition -- restoring 1,000 with the guard satisfied.
  const inspected = link({ allocation_amount_msat: 1000, from_allocation_amount_msat: 1000, updated_at: "t1" });
  const refreshed = link({ allocation_amount_msat: 2000, from_allocation_amount_msat: 2000, updated_at: "t2" });

  it("refreshes a pristine form and approves the refreshed amounts unchanged", () => {
    const pristine = linkReviewFormFromLink(inspected);
    const next = reconcileLinkForm({ form: pristine, inspected, latest: refreshed });
    expect(next.inspected).toBe(refreshed);
    const payload = linkReviewPayload({ link: next.inspected, form: next.form, state: "reviewed" });
    expect(payload.expected_allocation_amount_msat).toBe(2000);
    // Nothing was edited, so no amount travels -- the stored 2,000 stands.
    expect(payload).not.toHaveProperty("allocation_amount");
    expect(payload).not.toHaveProperty("from_allocation_amount");
  });

  it("keeps a dirty form AND its original precondition, so the server refuses", () => {
    const dirty = { ...linkReviewFormFromLink(inspected), allocation_amount: "0.00000003" };
    const next = reconcileLinkForm({ form: dirty, inspected, latest: refreshed });
    expect(next.form).toBe(dirty);
    expect(next.inspected).toBe(inspected);
    const payload = linkReviewPayload({ link: next.inspected, form: next.form, state: "reviewed" });
    // Bound to what was inspected (1,000), not to what was fetched last (2,000):
    // update_link_review compares against stored 2,000 and raises
    // source_funds_link_stale instead of writing the edit over unseen amounts.
    expect(payload.expected_allocation_amount_msat).toBe(1000);
    expect(payload.allocation_amount).toBe("0.00000003");
  });

  it("selecting a different link always rebuilds the form", () => {
    const dirty = { ...linkReviewFormFromLink(inspected), explanation: "half-typed" };
    const other = link({ id: "link-2", allocation_amount_msat: 5000 });
    const next = reconcileLinkForm({ form: dirty, inspected, latest: other });
    expect(next.inspected).toBe(other);
    expect(next.form.allocation_amount).toBe(amountInput(5000));
  });
});
