import { describe, expect, it } from "vitest";

import { isBulkReviewableLink, type SourceFundsLink } from "./model";

function link(method: string, confidence = "strong"): SourceFundsLink {
  return {
    id: `${method}:${confidence}`,
    to_transaction_id: "in",
    link_type: "self_transfer",
    state: "suggested",
    confidence,
    method,
    asset: "BTC",
    allocation_amount: 0.1,
    allocation_policy: "heuristic",
    uses_chain_observation: false,
  };
}

describe("source-funds deterministic bulk boundary", () => {
  it("keeps provider and legacy external ids manual", () => {
    expect(isBulkReviewableLink(link("provider_trade_id"))).toBe(false);
    expect(isBulkReviewableLink(link("same_external_id", "exact"))).toBe(false);
  });

  it("requires exact confidence for canonical row identity", () => {
    expect(isBulkReviewableLink(link("same_onchain_scope"))).toBe(false);
    expect(isBulkReviewableLink(link("same_onchain_scope", "exact"))).toBe(true);
  });

  it("allows revalidated structural and reviewed-pair methods", () => {
    expect(isBulkReviewableLink(link("utxo_spend", "exact"))).toBe(true);
    expect(isBulkReviewableLink(link("utxo_spend", "strong"))).toBe(false);
    expect(isBulkReviewableLink(link("payment_hash", "exact"))).toBe(true);
    expect(isBulkReviewableLink(link("transaction_pair"))).toBe(true);
  });

  it("honors an explicit manual-review boundary", () => {
    expect(
      isBulkReviewableLink({
        ...link("utxo_spend", "exact"),
        requires_review: true,
      }),
    ).toBe(false);
  });
});
