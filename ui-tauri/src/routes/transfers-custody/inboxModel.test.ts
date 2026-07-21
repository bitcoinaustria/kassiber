import { describe, expect, it } from "vitest";

import type { CustodyGap } from "../custodyGapsModel";
import {
  blockedYears,
  buildInboxItems,
  candidatePresentation,
  compareInboxItems,
  countInboxItems,
  filterInboxItems,
  itemBlocksReports,
  itemHasCompetingEvidence,
  itemIsLowConfidence,
  itemIsSuggested,
  topReasonCodes,
  type InboxCandidate,
  type InboxItem,
} from "./inboxModel";

function gap(overrides: Partial<CustodyGap> = {}): CustodyGap {
  return {
    gap_id: "gap-1",
    candidate_fingerprint: "fp-1",
    status: "needs_review",
    asset: "BTC",
    source_wallet_label: "Ledger cold",
    destination_wallet_labels: ["Sparrow hot"],
    source_total_msat: "1000000000000",
    source_fee_msat: "40000000",
    source_debit_msat: "1000040000000",
    return_total_msat: "990000000000",
    residual_msat: "9960000000",
    started_at: "2026-06-12T00:00:00Z",
    ended_at: "2026-06-14T00:00:00Z",
    confidence: "strong",
    promotion_eligible: true,
    competitor_score_margin: 210,
    reason_codes: ["amount_coverage_high", "wallet_transition"],
    downstream: { affected_disposals: 3, affected_years: [2024, 2025] },
    ...overrides,
  };
}

function candidate(overrides: Partial<InboxCandidate> = {}): InboxCandidate {
  return {
    out_id: "tx-out",
    in_id: "tx-in",
    out_asset: "BTC",
    in_asset: "BTC",
    out_amount_msat: 30_000_000_000,
    in_amount_msat: 29_990_000_000,
    out_wallet_label: "Kraken",
    in_wallet_label: "Cold storage",
    out_wallet_kind: "esplora",
    in_wallet_kind: "esplora",
    out_occurred_at: "2026-07-10T10:00:00Z",
    in_occurred_at: "2026-07-10T10:20:00Z",
    confidence: "strong",
    method: "heuristic",
    swap_fee_msat: 10_000_000,
    default_kind: "manual",
    default_policy: "carrying-value",
    conflict_set_id: "c-1",
    conflict_size: 1,
    ...overrides,
  };
}

describe("buildInboxItems", () => {
  it("emits open questions for needs_review and conflicting gaps", () => {
    const items = buildInboxItems(
      [gap(), gap({ gap_id: "gap-2", status: "conflicting" })],
      [],
    );
    expect(items.map((item) => item.kind)).toEqual(["gap", "gap"]);
  });

  it("emits a residual follow-up for a resolved gap with unclassified residual", () => {
    const items = buildInboxItems(
      [gap({ status: "resolved", residual_msat: "5000" })],
      [],
    );
    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe("residual");
  });

  it("drops resolved gaps without residual, dismissed gaps, and classified residuals", () => {
    const items = buildInboxItems(
      [
        gap({ status: "resolved", residual_msat: "0" }),
        gap({ gap_id: "gap-2", status: "dismissed" }),
        gap({
          gap_id: "gap-3",
          status: "resolved",
          residual_msat: "5000",
          residual_classification: {
            classification: "external_payment",
            custody_state: "external_confirmed",
            country_tax_meaning: "not_assigned",
            amount_msat: "5000",
          },
        }),
      ],
      [],
    );
    expect(items).toEqual([]);
  });

  it("keeps reopened bridges (revise corrections) out of the inbox", () => {
    const items = buildInboxItems(
      [gap({ status: "needs_review", status_reason: "bridge_reopened" })],
      [],
    );
    expect(items).toEqual([]);
  });

  it("ranks report-blocking gaps before candidates and weak hints last", () => {
    const blocking = gap();
    const hint = gap({
      gap_id: "gap-weak",
      confidence: "weak",
      promotion_eligible: false,
      downstream: { affected_disposals: 0, affected_years: [] },
    });
    const quiet = gap({
      gap_id: "gap-quiet",
      confidence: "moderate",
      promotion_eligible: false,
      downstream: { affected_disposals: 0, affected_years: [] },
    });
    const items = buildInboxItems([hint, quiet, blocking], [candidate()]);
    expect(items.map((item) => item.id)).toEqual([
      "gap:gap-1",
      "gap:gap-quiet",
      "candidate:tx-out->tx-in",
      "gap:gap-weak",
    ]);
  });

  it("orders same-band items by confidence then amount", () => {
    const exact = candidate({ out_id: "a", confidence: "exact" });
    const bigStrong = candidate({ out_id: "b", out_amount_msat: 90_000_000_000 });
    const smallStrong = candidate({ out_id: "c", out_amount_msat: 1_000_000 });
    const items = buildInboxItems([], [smallStrong, bigStrong, exact]);
    expect(items.map((item) => item.id.split(":")[1])).toEqual([
      "a->tx-in",
      "b->tx-in",
      "c->tx-in",
    ]);
  });
});

describe("item classification", () => {
  it("marks gaps with downstream impact as report-blocking", () => {
    const items = buildInboxItems([gap()], [candidate()]);
    expect(items.map(itemBlocksReports)).toEqual([true, false]);
  });

  it("marks promotion-eligible gaps and exact solo candidates as suggested", () => {
    const suggested: InboxItem[] = buildInboxItems(
      [gap()],
      [
        candidate({ out_id: "a", confidence: "exact" }),
        candidate({ out_id: "b", confidence: "exact", conflict_size: 2 }),
        candidate({ out_id: "c" }),
      ],
    );
    expect(
      suggested.filter(itemIsSuggested).map((item) => item.id),
    ).toEqual(["gap:gap-1", "candidate:a->tx-in"]);
  });

  it("flags competing evidence for conflicting gaps and clustered candidates", () => {
    expect(
      itemHasCompetingEvidence({
        kind: "gap",
        id: "g",
        gap: gap({ status: "conflicting" }),
      }),
    ).toBe(true);
    expect(
      itemHasCompetingEvidence({
        kind: "candidate",
        id: "c",
        candidate: candidate({ conflict_size: 3 }),
      }),
    ).toBe(true);
    expect(
      itemHasCompetingEvidence({
        kind: "candidate",
        id: "c",
        candidate: candidate(),
      }),
    ).toBe(false);
  });

  it("collapses weak gaps as low confidence", () => {
    expect(
      itemIsLowConfidence({
        kind: "gap",
        id: "g",
        gap: gap({ confidence: "weak" }),
      }),
    ).toBe(true);
    expect(
      itemIsLowConfidence({ kind: "candidate", id: "c", candidate: candidate() }),
    ).toBe(false);
  });
});

describe("filters and counts", () => {
  const items = buildInboxItems(
    [
      gap(),
      gap({
        gap_id: "gap-weak",
        confidence: "weak",
        promotion_eligible: false,
        downstream: { affected_disposals: 0, affected_years: [] },
      }),
    ],
    [candidate({ confidence: "exact" })],
  );

  it("filters blocking and suggested subsets", () => {
    expect(filterInboxItems(items, "blocking").map((i) => i.id)).toEqual([
      "gap:gap-1",
    ]);
    expect(filterInboxItems(items, "suggested")).toHaveLength(2);
    expect(filterInboxItems(items, "all")).toHaveLength(3);
  });

  it("counts each chip facet", () => {
    expect(countInboxItems(items)).toEqual({
      open: 3,
      blocking: 1,
      suggested: 2,
      lowConfidence: 1,
    });
  });

  it("collects blocked tax years ascending", () => {
    expect(blockedYears(items)).toEqual([2024, 2025]);
  });
});

describe("candidate presentation", () => {
  it("classifies same-asset moves, layer transitions, and swaps", () => {
    expect(candidatePresentation(candidate())).toBe("transfer");
    expect(
      candidatePresentation(
        candidate({ in_asset: "LBTC", default_kind: "peg-in" }),
      ),
    ).toBe("layer-transition");
    expect(
      candidatePresentation(candidate({ in_asset: "USDT" })),
    ).toBe("swap");
  });
});

describe("topReasonCodes", () => {
  it("prefers structured evidence over generic codes and caps at three", () => {
    const codes = topReasonCodes(
      gap({
        reason_codes: [
          "long_horizon",
          "amount_coverage_high",
          "structured_privacy_boundary",
          "wallet_transition",
          "split_source",
        ],
      }),
    );
    expect(codes).toEqual([
      "structured_privacy_boundary",
      "amount_coverage_high",
      "wallet_transition",
    ]);
  });
});

describe("compareInboxItems", () => {
  it("is a stable total order (ties break on id)", () => {
    const a: InboxItem = { kind: "candidate", id: "candidate:a", candidate: candidate() };
    const b: InboxItem = { kind: "candidate", id: "candidate:b", candidate: candidate() };
    expect(compareInboxItems(a, b)).toBeLessThan(0);
    expect(compareInboxItems(b, a)).toBeGreaterThan(0);
  });
});
