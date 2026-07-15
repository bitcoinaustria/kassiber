import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";

import {
  BridgePreviewPanel,
  CorrectionPreviewPanel,
  CustodyCoverageTimeline,
  CustodyLineageTimeline,
  ResidualPreviewPanel,
  ReviewHistoryPanel,
} from "./CustodyGaps";
import {
  bridgeCreateArgs,
  bridgePreviewArgs,
  reopenConfirmArgs,
  reopenPreviewArgs,
  residualConfirmArgs,
  residualPreviewArgs,
  reviseConfirmArgs,
  revisePreviewArgs,
  shouldOfferResidualClassification,
  type BridgePreview,
  type GuidedCorrectionPreview,
  type ResidualClassificationPreview,
  canShowNoKnownCustodyGaps,
  collectCustodyGapPages,
  custodyGapActionMode,
  formatCustodyMsat,
  type CustodyGap,
  type CustodyGapReviewHistoryEntry,
  type CustodyGapSnapshot,
  type CustodyCoverageSnapshot,
  type CustodyLineageSnapshot,
} from "./custodyGapsModel";

const reviewedBridgePreview: BridgePreview = {
  gap_id: "gap-og",
  candidate_fingerprint: "suggestion-fingerprint-og",
  authored_claim_fingerprint: "authored-fingerprint-og",
  dry_run: true,
  activatable: true,
  review_mode: "manual_weak_hint",
  warnings: [
    "manual_review_required",
    "weak_advisory_evidence",
    "unresolved_residual",
  ],
  requires_explicit_confirmation: true,
  retained_msat: "990000000000",
  residual_msat: "10000000000",
  fee_msat: "10000000",
  source_count: 1,
  destination_count: 1,
  filed_report_impacts: [
    {
      filed_report_snapshot_id: "report-2021",
      report_kind: "capital_gains",
      report_state: "filed",
      affected_period_start_year: 2021,
      affected_period_end_year: 2021,
      after_gain_summary: { status: "pending_journal_rebuild" },
      amendment_warning: "Review whether an amended filing is required.",
    },
  ],
};

describe("CustodyGaps guided desktop bridge", () => {
  it("renders exact quantities and filed-report impact before confirmation", () => {
    const html = renderToStaticMarkup(
      <BridgePreviewPanel
        preview={reviewedBridgePreview}
        asset="BTC"
        isCreating={false}
        onConfirm={() => undefined}
      />,
    );

    expect(html).toContain("Exact bridge preview passed");
    expect(html).toContain("9.9 BTC");
    expect(html).toContain("0.1 BTC");
    expect(html).toContain("Saved or filed report may need amendment");
    expect(html).toContain("Review whether an amended filing is required.");
    expect(html).toContain("not final until journals are rebuilt");
    expect(html).toContain("Amount and timing alone are ambiguous");
    expect(html).toContain("I reviewed these exact amounts");
    expect(html).toContain("Create reviewed bridge");
    expect(html).toContain("disabled");
    expect(html).not.toContain("raw_json");
    expect(html).not.toContain("component JSON");
  });

  it("sends only the guided gap identity and preview fingerprint", () => {
    expect(bridgePreviewArgs("gap-og")).toEqual({ gap_id: "gap-og" });
    expect(bridgeCreateArgs(reviewedBridgePreview)).toEqual({
      gap_id: "gap-og",
      expected_fingerprint: "authored-fingerprint-og",
    });
  });
});

describe("CustodyGaps immutable correction workflow", () => {
  const reopenPreview: GuidedCorrectionPreview = {
    gap_id: "gap-og",
    expected_fingerprint: "reopen-fingerprint",
    dry_run: true,
    requires_explicit_confirmation: true,
    resulting_status: "needs_review",
    current_component_revision: 4,
    filed_report_impacts: [],
  };
  const revisePreview: GuidedCorrectionPreview = {
    gap_id: "gap-og",
    expected_fingerprint: "revise-fingerprint",
    dry_run: true,
    requires_explicit_confirmation: true,
    activatable: true,
    current_component_revision: 4,
    new_component_revision: 5,
    retained_msat: "990000000000",
    residual_msat: "10000000000",
    filed_report_impacts: [],
  };

  it("previews reopening before confirmation without asking for component input", () => {
    const html = renderToStaticMarkup(
      <CorrectionPreviewPanel
        preview={reopenPreview}
        mode="reopen"
        asset="BTC"
        isPending={false}
        onConfirm={() => undefined}
      />,
    );

    expect(html).toContain("Exact reopen preview passed");
    expect(html).toContain("superseded, not deleted");
    expect(html).toContain("reports remain blocked");
    expect(html).toContain("Reopen reviewed bridge");
    expect(html).toContain("disabled");
    expect(html).not.toContain("component JSON");
    expect(html).not.toContain("component_id");
  });

  it("keeps revision in the existing lineage and preserves exact quantities", () => {
    const html = renderToStaticMarkup(
      <CorrectionPreviewPanel
        preview={revisePreview}
        mode="revise"
        asset="BTC"
        isPending={false}
        onConfirm={() => undefined}
      />,
    );

    expect(html).toContain("Exact revision preview passed");
    expect(html).toContain("revision 4");
    expect(html).toContain("revision 5");
    expect(html).toContain("same lineage");
    expect(html).toContain("9.9 BTC");
    expect(html).toContain("0.1 BTC");
  });

  it("binds confirmation to the preview fingerprint and identical normalized note", () => {
    expect(reopenPreviewArgs("gap-og", "  Wrong return group  ")).toEqual({
      gap_id: "gap-og",
      reason: "Wrong return group",
    });
    expect(reopenConfirmArgs(reopenPreview, "Wrong return group")).toEqual({
      gap_id: "gap-og",
      expected_fingerprint: "reopen-fingerprint",
      reason: "Wrong return group",
    });
    expect(revisePreviewArgs("gap-og", "")).toEqual({ gap_id: "gap-og" });
    expect(reviseConfirmArgs(revisePreview, "")).toEqual({
      gap_id: "gap-og",
      expected_fingerprint: "revise-fingerprint",
    });
  });

  it("routes reopened bridges to revise, never to unrelated bridge creation", () => {
    const base: CustodyGap = {
      gap_id: "gap-og",
      candidate_fingerprint: "candidate",
      status: "needs_review",
      asset: "BTC",
      source_wallet_label: "Old vault",
      destination_wallet_labels: ["New vault"],
      source_total_msat: "1000000000000",
      source_fee_msat: "0",
      source_debit_msat: "1000000000000",
      return_total_msat: "990000000000",
      residual_msat: "10000000000",
      started_at: null,
      ended_at: null,
      confidence: "strong",
      promotion_eligible: true,
      competitor_score_margin: null,
      reason_codes: [],
      downstream: { affected_disposals: 0, affected_years: [] },
    };

    expect(custodyGapActionMode(base)).toBe("create");
    expect(
      custodyGapActionMode({ ...base, status_reason: "bridge_reopened" }),
    ).toBe("revise");
    expect(custodyGapActionMode({ ...base, status: "resolved" })).toBe(
      "reopen",
    );
    expect(custodyGapActionMode({ ...base, status: "dismissed" })).toBe(
      "none",
    );
  });
});

describe("CustodyGaps guided residual workflow", () => {
  const residualPreview: ResidualClassificationPreview = {
    gap_id: "gap-og",
    expected_fingerprint: "residual-fingerprint",
    dry_run: true,
    requires_explicit_confirmation: true,
    activatable: true,
    classification: "external_gift",
    custody_state: "external_confirmed",
    country_tax_meaning: "not_assigned",
    residual_msat: "10000000000",
    current_component_revision: 4,
    new_component_revision: 5,
    filed_report_impacts: [],
  };

  it("shows custody outcome separately from unassigned country-tax meaning", () => {
    const html = renderToStaticMarkup(
      <ResidualPreviewPanel
        preview={residualPreview}
        asset="BTC"
        isPending={false}
        onConfirm={() => undefined}
      />,
    );

    expect(html).toContain("Exact residual preview passed");
    expect(html).toContain("0.1 BTC");
    expect(html).toContain("External gift");
    expect(html).toContain("Country-tax meaning");
    expect(html).toContain("Not assigned by this custody action");
    expect(html).toContain("Gift and loss labels still require");
    expect(html).toContain("does not assign country-specific tax treatment");
    expect(html).toContain("disabled");
  });

  it("never asks for component JSON or an internal component id", () => {
    expect(
      residualPreviewArgs("gap-og", "external_gift", "  Family transfer  "),
    ).toEqual({
      gap_id: "gap-og",
      classification: "external_gift",
      reason: "Family transfer",
    });
    expect(residualConfirmArgs(residualPreview, "Family transfer")).toEqual({
      gap_id: "gap-og",
      classification: "external_gift",
      expected_fingerprint: "residual-fingerprint",
      reason: "Family transfer",
    });
  });

  it("offers one residual decision at a time and routes corrections through reopen", () => {
    const gap: CustodyGap = {
      gap_id: "gap-og",
      candidate_fingerprint: "candidate",
      status: "resolved",
      asset: "BTC",
      source_wallet_label: "Old vault",
      destination_wallet_labels: ["New vault"],
      source_total_msat: "1000000000000",
      source_fee_msat: "0",
      source_debit_msat: "1000000000000",
      return_total_msat: "990000000000",
      residual_msat: "10000000000",
      started_at: null,
      ended_at: null,
      confidence: "strong",
      promotion_eligible: true,
      competitor_score_margin: null,
      reason_codes: [],
      downstream: { affected_disposals: 0, affected_years: [] },
    };

    expect(shouldOfferResidualClassification(gap)).toBe(true);
    expect(
      shouldOfferResidualClassification({
        ...gap,
        residual_classification: {
          classification: "external_gift",
          custody_state: "external_confirmed",
          country_tax_meaning: "not_assigned",
          amount_msat: "10000000000",
        },
      }),
    ).toBe(false);
  });
});

describe("CustodyGaps bounded review history", () => {
  const history: CustodyGapReviewHistoryEntry[] = [
    {
      revision: 1,
      event_kind: "bridge_created",
      status: "resolved",
      component_revision: 4,
      authored_source: "user",
      reason: "Reviewed missing wallet interval",
      created_at: "2026-07-14T12:00:00Z",
      retained_msat: "990000000000",
      residual_msat: "10000000000",
      residual_classification: null,
      filed_report_impact_count: 1,
    },
    {
      revision: 2,
      event_kind: "residual_classified",
      status: "resolved",
      component_revision: 5,
      authored_source: "user",
      reason: null,
      created_at: "2026-07-15T12:00:00Z",
      retained_msat: "990000000000",
      residual_msat: "10000000000",
      residual_classification: "external_gift",
      filed_report_impact_count: 0,
    },
  ];

  it("renders append-only decisions without internal component identifiers", () => {
    const html = renderToStaticMarkup(
      <ReviewHistoryPanel history={history} asset="BTC" />,
    );

    expect(html).toContain("Append-only review history");
    expect(html).toContain("Reviewed bridge created");
    expect(html).toContain("Residual classified");
    expect(html).toContain("9.9 BTC");
    expect(html).toContain("0.1 BTC");
    expect(html).toContain("External gift");
    expect(html).not.toContain("component_id");
  });
});

describe("CustodyGaps imported-policy coverage", () => {
  const coverage: CustodyCoverageSnapshot = {
    schema_version: 1,
    scope: "imported_policy_technical_coverage",
    ownership_universe_known: false,
    coverage_can_clear_custody_gaps: false,
    summary: {
      wallet_count: 1,
      epoch_count: 2,
      active_epoch_count: 1,
      retired_epoch_count: 1,
      source_count: 1,
      covered_branch_count: 2,
    },
    wallets: [
      {
        wallet_label: "Operations vault",
        epochs: [
          {
            epoch_id: "epoch-retired",
            status: "retired",
            chain: "bitcoin",
            network: "main",
            created_at: "2021-01-01T00:00:00Z",
            retired_at: "2024-01-01T00:00:00Z",
            sources: [
              {
                source: "descriptor-policy",
                observer_kind: "bdk",
                branches: [
                  {
                    branch: "receive",
                    scanned_to_exclusive: 50,
                    highest_used: 7,
                    observed_at: "2024-01-01T00:00:00Z",
                  },
                  {
                    branch: "change",
                    scanned_to_exclusive: 50,
                    highest_used: null,
                    observed_at: "2024-01-01T00:00:00Z",
                  },
                ],
              },
            ],
          },
          {
            epoch_id: "epoch-active",
            status: "active",
            chain: "bitcoin",
            network: "main",
            created_at: "2024-01-01T00:00:00Z",
            retired_at: null,
            sources: [],
          },
        ],
      },
    ],
  };

  it("renders technical epoch boundaries without implying wallet completeness", () => {
    const html = renderToStaticMarkup(
      <CustodyCoverageTimeline snapshot={coverage} />,
    );

    expect(html).toContain("Imported wallet policy timeline");
    expect(html).toContain("Ownership universe unknown");
    expect(html).toContain("Operations vault");
    expect(html).toContain("Retired epoch");
    expect(html).toContain("Active epoch");
    expect(html).toContain("Exclusive scan bound: 50");
    expect(html).toContain("Highest used index: 7");
    expect(html).toContain("No used index observed");
    expect(html).toContain("never clears a custody gap");
    expect(html).not.toContain("xpub6PrivateMaterial");
    expect(html).not.toContain("wpkh(secret-descriptor)");
    expect(html).not.toContain("bc1privateaddress");
  });
});

describe("CustodyGaps canonical custody lineage", () => {
  const lineage: CustodyLineageSnapshot = {
    summary: {
      total_count: 1,
      returned_count: 1,
      truncated: false,
      internal_verified: 1,
      internal_reviewed: 0,
      basis_eligible: 0,
      basis_blocked: 1,
    },
    items: [
      {
        out_transaction_id: "private-out-transaction-id",
        in_transaction_id: "private-in-transaction-id",
        occurred_at: "2026-03-12T11:00:00Z",
        asset: "BTC",
        amount_msat: "609000000000",
        from_wallet_id: "private-source-wallet-id",
        from_wallet_label: "Treasury vault",
        to_wallet_id: "private-target-wallet-id",
        to_wallet_label: "Spending wallet",
        custody_state: "internal_verified",
        basis_state: "blocked_by_prior_custody_basis",
        basis_barrier_at: "2024-08-01T10:00:00Z",
        evidence_reason: "recorded_fanout",
        network: "main",
        rail: "bitcoin",
        atomic_bundle_id: "private-atomic-bundle-id",
        component_id: "private-component-id",
      },
    ],
  };

  it("shows exact custody finality separately from a prior tax-basis barrier", () => {
    const html = renderToStaticMarkup(
      <CustodyLineageTimeline snapshot={lineage} />,
    );

    expect(html).toContain("Custody lineage");
    expect(html).toContain("Treasury vault");
    expect(html).toContain("Spending wallet");
    expect(html).toContain("6.09 BTC");
    expect(html).toContain("Custody verified");
    expect(html).toContain("Tax basis blocked");
    expect(html).toContain(
      "Custody is settled, but tax basis remains blocked",
    );
    expect(html).toContain(
      "Exact outputs from one recorded chain transaction",
    );
  });

  it("does not render transaction, wallet, component, or bundle identifiers", () => {
    const html = renderToStaticMarkup(
      <CustodyLineageTimeline snapshot={lineage} />,
    );

    expect(html).not.toContain("private-out-transaction-id");
    expect(html).not.toContain("private-in-transaction-id");
    expect(html).not.toContain("private-source-wallet-id");
    expect(html).not.toContain("private-target-wallet-id");
    expect(html).not.toContain("private-atomic-bundle-id");
    expect(html).not.toContain("private-component-id");
  });
});

describe("CustodyGaps quantity formatting", () => {
  it("formats exact integer payloads without crossing the JavaScript safe-integer boundary", () => {
    expect(formatCustodyMsat("1000000000000", "BTC")).toBe("10 BTC");
    expect(formatCustodyMsat("990000000000", "BTC")).toBe("9.9 BTC");
    expect(formatCustodyMsat("10000000000", "BTC")).toBe("0.1 BTC");
  });

  it("keeps millisatoshi precision for off-chain assets", () => {
    expect(formatCustodyMsat("1", "LNBTC")).toBe("0.00000000001 LNBTC");
  });
});

describe("CustodyGaps clear-state guard", () => {
  const snapshot = (derivedStateCurrent: boolean): CustodyGapSnapshot => ({
    summary: {
      total: 0,
      needs_review: 0,
      conflicting: 0,
      resolved: 0,
      dismissed: 0,
      unresolved_msat: "0",
      canonical_issue_count: 0,
      derived_state_current: derivedStateCurrent,
      search_complete: true,
    },
    gaps: [],
  });

  it("never claims a clear queue from stale or unprocessed derived state", () => {
    expect(canShowNoKnownCustodyGaps(snapshot(false), 0)).toBe(false);
  });

  it("allows the qualified clear state only for a current empty projection", () => {
    expect(canShowNoKnownCustodyGaps(snapshot(true), 0)).toBe(true);
    expect(canShowNoKnownCustodyGaps(snapshot(true), 1)).toBe(false);
  });

  it("fails closed when summary counts disagree with the visible queue", () => {
    const inconsistent = snapshot(true);
    inconsistent.summary.needs_review = 1;
    expect(canShowNoKnownCustodyGaps(inconsistent, 0)).toBe(false);
  });

  it("never claims clear when advisory suggestion search was incomplete", () => {
    const incomplete = snapshot(true);
    incomplete.summary.search_complete = false;
    expect(canShowNoKnownCustodyGaps(incomplete, 0)).toBe(false);
  });
});

describe("CustodyGaps pagination", () => {
  const gap = (index: number): CustodyGap => ({
    gap_id: `gap-${index.toString().padStart(3, "0")}`,
    candidate_fingerprint: `fingerprint-${index}`,
    status: "needs_review",
    asset: "BTC",
    source_wallet_label: "Old vault",
    destination_wallet_labels: ["New vault"],
    source_total_msat: "100000000000",
    source_fee_msat: "0",
    source_debit_msat: "100000000000",
    return_total_msat: "99000000000",
    residual_msat: "1000000000",
    started_at: "2020-01-01T00:00:00Z",
    ended_at: "2021-01-01T00:00:00Z",
    confidence: "strong",
    promotion_eligible: true,
    competitor_score_margin: null,
    reason_codes: ["wallet_transition"],
    downstream: { affected_disposals: 0, affected_years: [] },
  });

  const page = (gaps: CustodyGap[], nextCursor: string | null): CustodyGapSnapshot => ({
    summary: {
      total: 101,
      needs_review: 101,
      conflicting: 0,
      resolved: 0,
      dismissed: 0,
      unresolved_msat: "101000000000",
    },
    gaps,
    next_cursor: nextCursor,
  });

  it("keeps actionable gaps beyond the first 100 reachable", () => {
    const first = Array.from({ length: 100 }, (_, index) => gap(index));
    const all = collectCustodyGapPages([
      page(first, "100"),
      page([gap(100)], null),
    ]);

    expect(all).toHaveLength(101);
    expect(all.at(-1)?.gap_id).toBe("gap-100");
  });

  it("deduplicates a boundary row if evidence changes during pagination", () => {
    const all = collectCustodyGapPages([
      page([gap(0), gap(1)], "2"),
      page([gap(1), gap(2)], null),
    ]);

    expect(all.map((item) => item.gap_id)).toEqual(["gap-000", "gap-001", "gap-002"]);
  });
});
