import { describe, expect, it } from "vitest";

import {
  buildCustodyBulkRequest,
  buildCustodyRevisionDocument,
  CUSTODY_COMPONENT_EXAMPLE,
  formatCustodyExactInteger,
  previewCustodyComponentBatch,
} from "./custodyComponentBulk";

function issueCodes(issues: Array<{ code: string }>) {
  return issues.map((issue) => issue.code);
}

describe("previewCustodyComponentBatch", () => {
  it("accepts the multi-hop migration example for atomic activation", () => {
    const preview = previewCustodyComponentBatch(CUSTODY_COMPONENT_EXAMPLE);

    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
    expect(preview.summary).toMatchObject({
      components: 2,
      sources: 2,
      destinations: 3,
      transactionAnchors: 3,
      untrackedLegs: 2,
    });
  });

  it("allows an unresolved migration to be saved as a draft but blocks activation", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          component_type: "manual_bridge",
          legs: [
            {
              role: "source",
              transaction: "old-wallet-send",
              amount_msat: 100_000,
            },
            {
              role: "destination",
              transaction: "new-wallet-receive",
              amount_msat: 90_000,
            },
            {
              role: "unresolved",
              amount_msat: 10_000,
            },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(issueCodes(preview.activationErrors)).toContain("unresolvedValue");
  });

  it("accepts exact reviewed residual suspense without treating it as unresolved", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          component_type: "manual_bridge",
          conservation_mode: "quantity",
          evidence_grade: "reviewed",
          legs: [
            {
              id: "source",
              role: "source",
              transaction: "old-wallet-send",
              occurred_at: "2020-01-01T00:00:00Z",
              amount_msat: 100_000,
            },
            {
              id: "destination",
              role: "destination",
              transaction: "new-wallet-receive",
              occurred_at: "2021-01-01T00:00:00Z",
              amount_msat: 90_000,
            },
            {
              id: "suspense",
              role: "suspense",
              occurred_at: "2020-01-01T00:00:00Z",
              amount_msat: 10_000,
            },
          ],
          allocations: [
            {
              source_leg_id: "source",
              sink_leg_id: "destination",
              source_amount_msat: 90_000,
              sink_amount_msat: 90_000,
            },
            {
              source_leg_id: "source",
              sink_leg_id: "suspense",
              source_amount_msat: 10_000,
              sink_amount_msat: 10_000,
            },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
    expect(preview.summary).toMatchObject({
      unresolvedLegs: 0,
      suspenseLegs: 1,
    });
  });

  it("fails closed when suspense is inferred, located, or not reviewed", () => {
    const withoutAllocation = previewCustodyComponentBatch(
      JSON.stringify([
        {
          component_type: "manual_bridge",
          evidence_grade: "reviewed",
          legs: [
            { role: "source", transaction: "out", amount_msat: 100 },
            { role: "destination", transaction: "in", amount_msat: 90 },
            {
              role: "suspense",
              occurred_at: "2020-01-01T00:00:00Z",
              amount_msat: 10,
            },
          ],
        },
      ]),
    );
    expect(issueCodes(withoutAllocation.activationErrors)).toContain(
      "suspenseAllocationRequired",
    );

    const invalid = previewCustodyComponentBatch(
      JSON.stringify([
        {
          component_type: "native_transfer",
          evidence_grade: "exact",
          legs: [
            {
              id: "source",
              role: "source",
              transaction: "out",
              occurred_at: "2020-01-01T00:00:00Z",
              asset: "BTC",
              amount_msat: 100,
            },
            {
              id: "destination",
              role: "destination",
              transaction: "in",
              asset: "BTC",
              amount_msat: 90,
            },
            {
              id: "suspense",
              role: "suspense",
              wallet: "not-allowed",
              occurred_at: "2020-01-02T00:00:00Z",
              asset: "LBTC",
              amount_msat: 10,
            },
          ],
          allocations: [
            {
              source_leg_id: "source",
              sink_leg_id: "destination",
              source_amount_msat: 90,
              sink_amount_msat: 90,
            },
            {
              source_leg_id: "source",
              sink_leg_id: "suspense",
              source_amount_msat: 10,
              sink_amount_msat: 10,
            },
          ],
        },
      ]),
    );
    expect(issueCodes(invalid.activationErrors)).toEqual(
      expect.arrayContaining([
        "suspenseReviewRequired",
        "suspenseLocationInvalid",
        "suspenseAssetMismatch",
        "suspenseTimeMismatch",
      ]),
    );
  });

  it("requires explicit allocations for a genuine N:M component", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            { role: "source", transaction: "out-a", amount_msat: 60 },
            { role: "source", transaction: "out-b", amount_msat: 40 },
            { role: "destination", transaction: "in-a", amount_msat: 50 },
            { role: "destination", transaction: "in-b", amount_msat: 50 },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(issueCodes(preview.activationErrors)).toContain("allocationsRequired");
  });

  it("accepts a fully-covered N:M allocation graph and rejects undercoverage", () => {
    const base = {
      legs: [
        { id: "s1", role: "source", transaction: "out-a", amount_msat: 60 },
        { id: "s2", role: "source", transaction: "out-b", amount_msat: 40 },
        { id: "d1", role: "destination", transaction: "in-a", amount_msat: 50 },
        { id: "d2", role: "destination", transaction: "in-b", amount_msat: 50 },
      ],
      allocations: [
        {
          source_ordinal: 0,
          sink_ordinal: 2,
          source_amount_msat: 50,
          sink_amount_msat: 50,
        },
        {
          source_ordinal: 0,
          sink_ordinal: 3,
          source_amount_msat: 10,
          sink_amount_msat: 10,
        },
        {
          source_ordinal: 1,
          sink_ordinal: 3,
          source_amount_msat: 40,
          sink_amount_msat: 40,
        },
      ],
    };
    const valid = previewCustodyComponentBatch(JSON.stringify([base]));
    expect(valid.structuralErrors).toEqual([]);
    expect(valid.activationErrors).toEqual([]);

    const invalid = previewCustodyComponentBatch(
      JSON.stringify([
        {
          ...base,
          allocations: base.allocations.slice(0, 2),
        },
      ]),
    );
    expect(invalid.activationErrors).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          code: "allocationSourceCoverage",
          values: expect.objectContaining({ leg: 2, covered: "0", expected: "40" }),
        }),
        expect.objectContaining({
          code: "allocationSinkCoverage",
          values: expect.objectContaining({ leg: 4, covered: "10", expected: "50" }),
        }),
      ]),
    );
  });

  it("requires wallet and time for a transaction-less owned leg", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            { role: "source", transaction: "old-send", amount_msat: 10 },
            { role: "destination", amount_msat: 10 },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([
      {
        code: "transactionlessWalletRequired",
        values: { component: 1, leg: 2 },
      },
      {
        code: "transactionlessTimeRequired",
        values: { component: 1, leg: 2 },
      },
    ]);
  });

  it("never throws on invalid JSON, out-of-range allocations, or unsafe msat values", () => {
    expect(previewCustodyComponentBatch("{").structuralErrors[0]).toMatchObject({
      code: "jsonInvalid",
    });

    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            {
              role: "source",
              transaction: "out",
              amount_msat: Number.MAX_SAFE_INTEGER + 1,
            },
            { role: "destination", transaction: "in", amount_msat: 1 },
          ],
          allocations: [
            {
              source_ordinal: 99,
              sink_ordinal: 1,
              source_amount_msat: 1,
              sink_amount_msat: 1,
            },
          ],
        },
      ]),
    );
    expect(issueCodes(preview.structuralErrors)).toEqual(
      expect.arrayContaining(["amountInvalid", "allocationSourceInvalid"]),
    );
  });

  it("round-trips unsafe custody integers as exact decimal strings", () => {
    const exact = "9007199254740993";
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          conservation_mode: "conversion",
          conversion_policy: "reviewed_exact_value",
          conversion_reviewed: true,
          legs: [
            {
              id: "source-leg",
              role: "source",
              transaction: "out",
              amount_msat: exact,
              valuation_unit: "eur-cent",
              valuation_amount: exact,
            },
            {
              id: "sink-leg",
              role: "destination",
              transaction: "in",
              amount_msat: exact,
              valuation_unit: "eur-cent",
              valuation_amount: exact,
            },
          ],
          allocations: [
            {
              source_leg_id: "source-leg",
              sink_leg_id: "sink-leg",
              source_amount_msat: exact,
              sink_amount_msat: exact,
            },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
    const request = buildCustodyBulkRequest(preview, { activate: true });
    expect(request.components[0]).toMatchObject({
      legs: [
        expect.objectContaining({
          amount_msat: exact,
          valuation_amount: exact,
        }),
        expect.objectContaining({
          amount_msat: exact,
          valuation_amount: exact,
        }),
      ],
      allocations: [
        expect.objectContaining({
          source_amount_msat: exact,
          sink_amount_msat: exact,
        }),
      ],
    });

    const revision = JSON.parse(
      buildCustodyRevisionDocument({
        component_type: "swap",
        conservation_mode: "conversion",
        evidence_kind: "reviewed",
        evidence_grade: "reviewed",
        conversion_policy: "reviewed_exact_value",
        conversion_reviewed: true,
        notes: null,
        legs: [
          {
            id: "source-leg",
            role: "source",
            rail: "bitcoin",
            asset: "BTC",
            amount_msat: exact,
            valuation_unit: "eur-cent",
            valuation_amount: exact,
            occurred_at: "2026-01-01T00:00:00Z",
            transaction_id: "out",
            wallet_id: "source-wallet",
          },
          {
            id: "sink-leg",
            role: "destination",
            rail: "liquid",
            asset: "LBTC",
            amount_msat: exact,
            valuation_unit: "eur-cent",
            valuation_amount: exact,
            occurred_at: "2026-01-01T00:00:00Z",
            transaction_id: "in",
            wallet_id: "sink-wallet",
          },
        ],
        allocations: [
          {
            source_leg_id: "source-leg",
            sink_leg_id: "sink-leg",
            source_amount_msat: exact,
            sink_amount_msat: exact,
          },
        ],
      }),
    );
    expect(revision.components[0].legs[0].amount_msat).toBe(exact);
    expect(revision.components[0].legs[0].valuation_amount).toBe(exact);
    expect(revision.components[0].allocations[0].source_amount_msat).toBe(exact);
    expect(formatCustodyExactInteger(exact, "en-US")).toBe(
      "9,007,199,254,740,993",
    );

    const beyondSqlite = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            {
              id: "s",
              role: "source",
              transaction: "out",
              amount_msat: "9223372036854775808",
            },
            {
              id: "d",
              role: "destination",
              transaction: "in",
              amount_msat: 1,
            },
          ],
          allocations: [
            {
              source_leg_id: "s",
              sink_leg_id: "d",
              source_amount_msat: "9223372036854775808",
              sink_amount_msat: 1,
            },
          ],
        },
      ]),
    );
    expect(issueCodes(beyondSqlite.structuralErrors)).toEqual(
      expect.arrayContaining(["amountInvalid", "allocationAmountInvalid"]),
    );
  });

  it("parses decimal-string BTC amounts exactly at the one-msat boundary", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            {
              role: "source",
              transaction: "out",
              amount_btc: "0.00000000001",
            },
            {
              role: "destination",
              transaction: "in",
              amount_btc: "0.00000000001",
            },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
    expect(preview.summary).toMatchObject({ sources: 1, destinations: 1 });
  });

  it("rejects unbounded BTC decimal digits before BigInt parsing", () => {
    const hugeDigits = "9".repeat(10_000);
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            { role: "source", transaction: "out", amount_btc: hugeDigits },
            { role: "destination", transaction: "in", amount_btc: "1" },
          ],
        },
      ]),
    );

    expect(issueCodes(preview.structuralErrors)).toContain("amountInvalid");
  });

  it("matches daemon half-up rounding without forcing decimal strings through Number", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          legs: [
            { role: "source", transaction: "out", amount_btc: "+5e-12" },
            {
              role: "destination",
              transaction: "in",
              amount_btc: "0.000000000005",
            },
          ],
        },
      ]),
    );

    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
    expect(preview.summary).toMatchObject({ sources: 1, destinations: 1 });
  });

  it("balances reviewed conversions by exact valuation unit", () => {
    const component = {
      conservation_mode: "conversion",
      conversion_policy: "reviewed_market_conversion",
      conversion_reviewed: true,
      legs: [
        {
          role: "source",
          transaction: "out",
          amount_msat: 1_000,
          valuation_unit: "eur-cent",
          valuation_amount: 100,
        },
        {
          role: "destination",
          transaction: "in",
          amount_msat: 900,
          valuation_unit: "eur-cent",
          valuation_amount: 90,
        },
      ],
    };
    const invalid = previewCustodyComponentBatch(JSON.stringify([component]));
    expect(issueCodes(invalid.activationErrors)).toContain(
      "conversionValuationUnbalanced",
    );

    const valid = previewCustodyComponentBatch(
      JSON.stringify([
        {
          ...component,
          legs: [
            component.legs[0],
            { ...component.legs[1], valuation_amount: 100 },
          ],
        },
      ]),
    );
    expect(valid.structuralErrors).toEqual([]);
    expect(valid.activationErrors).toEqual([]);
  });

  it("classifies invalid conversion metadata as structural instead of draft-safe", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          conservation_mode: "conversion",
          conversion_policy: "Free-form policy text",
          conversion_reviewed: "yes",
          legs: [
            {
              role: "source",
              transaction: "out",
              amount_msat: 10,
              valuation_unit: "eur-cent",
              valuation_amount: "10.0",
            },
            {
              role: "destination",
              transaction: "in",
              amount_msat: 10,
              valuation_unit: "eur-cent",
              valuation_amount: 10,
            },
          ],
          allocations: {},
        },
      ]),
    );

    expect(issueCodes(preview.structuralErrors)).toEqual(
      expect.arrayContaining([
        "conversionPolicyInvalid",
        "conversionReviewedInvalid",
        "valuationAmountInvalid",
        "allocationsInvalid",
      ]),
    );
  });

  it("uses the same sanitized components for dry run and commit", () => {
    const preview = previewCustodyComponentBatch(
      JSON.stringify([
        {
          activate: true,
          legs: [
            { role: "source", transaction: "out", amount_msat: 1 },
            { role: "destination", transaction: "in", amount_msat: 1 },
          ],
        },
      ]),
    );

    const plan = buildCustodyBulkRequest(preview, {
      activate: false,
    });
    const commit = buildCustodyBulkRequest(preview, {
      activate: false,
      expectedFingerprint: "a".repeat(64),
    });

    expect(plan).toEqual({
      components: [expect.not.objectContaining({ activate: expect.anything() })],
      activate: false,
    });
    expect(commit).toEqual({
      components: plan.components,
      activate: false,
      expected_fingerprint: "a".repeat(64),
    });
  });
});
