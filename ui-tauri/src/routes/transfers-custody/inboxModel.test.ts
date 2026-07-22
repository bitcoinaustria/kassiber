import { describe, expect, it } from "vitest";

import type { CustodyGap } from "../custodyGapsModel";
import {
  blockedYears,
  buildInboxItems,
  compareInboxItems,
  countInboxItems,
  filterInboxItems,
  itemBlocksReports,
  itemHasCompetingEvidence,
  itemIsLowConfidence,
  itemIsSuggested,
  topReasonCodes,
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

describe("buildInboxItems", () => {
  it("emits open questions for needs_review and conflicting gaps", () => {
    const items = buildInboxItems([
      gap(),
      gap({ gap_id: "gap-2", status: "conflicting" }),
    ]);
    expect(items.map((item) => item.kind)).toEqual(["gap", "gap"]);
  });

  it("emits a residual follow-up for a resolved gap with unclassified residual", () => {
    const items = buildInboxItems([
      gap({ status: "resolved", residual_msat: "5000" }),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0]?.kind).toBe("residual");
  });

  it("drops resolved gaps without residual, dismissed gaps, and classified residuals", () => {
    const items = buildInboxItems([
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
    ]);
    expect(items).toEqual([]);
  });

  it("keeps reopened bridges (revise corrections) out of the inbox", () => {
    const items = buildInboxItems([
      gap({ status: "needs_review", status_reason: "bridge_reopened" }),
    ]);
    expect(items).toEqual([]);
  });

  it("ranks report-blocking gaps first and weak hints last", () => {
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
    const followUp = gap({
      gap_id: "gap-followup",
      status: "resolved",
      residual_msat: "5000",
      downstream: { affected_disposals: 0, affected_years: [] },
    });
    const items = buildInboxItems([hint, quiet, followUp, blocking]);
    expect(items.map((item) => item.id)).toEqual([
      "gap:gap-1",
      "gap:gap-quiet",
      "residual:gap-followup",
      "gap:gap-weak",
    ]);
  });

  it("orders same-band items by confidence then amount", () => {
    const base = {
      promotion_eligible: false,
      downstream: { affected_disposals: 0, affected_years: [] },
    };
    const strongSmall = gap({
      ...base,
      gap_id: "a",
      source_total_msat: "1000",
    });
    const strongBig = gap({
      ...base,
      gap_id: "b",
      source_total_msat: "2000",
    });
    const moderate = gap({ ...base, gap_id: "c", confidence: "moderate" });
    const items = buildInboxItems([moderate, strongSmall, strongBig]);
    expect(items.map((item) => item.id)).toEqual([
      "gap:b",
      "gap:a",
      "gap:c",
    ]);
  });
});

describe("item classification", () => {
  it("marks gaps with downstream impact as report-blocking", () => {
    const items = buildInboxItems([
      gap(),
      gap({
        gap_id: "gap-2",
        downstream: { affected_disposals: 0, affected_years: [] },
      }),
    ]);
    expect(items.map(itemBlocksReports)).toEqual([true, false]);
  });

  it("marks promotion-eligible gaps as suggested", () => {
    const items = buildInboxItems([
      gap(),
      gap({ gap_id: "gap-2", promotion_eligible: false }),
    ]);
    expect(items.map(itemIsSuggested)).toEqual([true, false]);
  });

  it("flags competing evidence for conflicting gaps", () => {
    expect(
      itemHasCompetingEvidence({
        kind: "gap",
        id: "g",
        gap: gap({ status: "conflicting" }),
      }),
    ).toBe(true);
    expect(
      itemHasCompetingEvidence({ kind: "gap", id: "g", gap: gap() }),
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
    expect(itemIsLowConfidence({ kind: "gap", id: "g", gap: gap() })).toBe(
      false,
    );
  });
});

describe("filters and counts", () => {
  const items = buildInboxItems([
    gap(),
    gap({
      gap_id: "gap-weak",
      confidence: "weak",
      promotion_eligible: false,
      downstream: { affected_disposals: 0, affected_years: [] },
    }),
  ]);

  it("filters blocking and suggested subsets", () => {
    expect(filterInboxItems(items, "blocking").map((i) => i.id)).toEqual([
      "gap:gap-1",
    ]);
    expect(filterInboxItems(items, "suggested")).toHaveLength(1);
    expect(filterInboxItems(items, "all")).toHaveLength(2);
  });

  it("counts each chip facet", () => {
    expect(countInboxItems(items)).toEqual({
      open: 2,
      blocking: 1,
      suggested: 1,
      lowConfidence: 1,
    });
  });

  it("collects blocked tax years ascending", () => {
    expect(blockedYears(items)).toEqual([2024, 2025]);
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
    const a: InboxItem = { kind: "gap", id: "gap:a", gap: gap() };
    const b: InboxItem = { kind: "gap", id: "gap:b", gap: gap() };
    expect(compareInboxItems(a, b)).toBeLessThan(0);
    expect(compareInboxItems(b, a)).toBeGreaterThan(0);
  });
});
