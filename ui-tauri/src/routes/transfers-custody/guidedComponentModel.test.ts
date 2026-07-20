import { describe, expect, it } from "vitest";

import { previewCustodyComponentBatch } from "@/lib/custodyComponentBulk";
import {
  componentToFormState,
  createGuidedAllocation,
  createGuidedLeg,
  createInitialGuidedForm,
  formToComponentSpec,
  formToDocument,
  msatToBtcInput,
  occurredAtToRfc3339,
  type CustodyComponentInput,
  type GuidedComponentFormState,
} from "./guidedComponentModel";

/** A balanced 1-source → destination + fee migration anchored to transactions. */
function migrationForm(): GuidedComponentFormState {
  const form = createInitialGuidedForm();
  const [source, destination, fee] = form.legs;
  source.amountBtc = "1.0";
  source.locationKind = "transaction";
  source.transactionRef = "tx-out";
  destination.amountBtc = "0.99";
  destination.locationKind = "transaction";
  destination.transactionRef = "tx-in";
  fee.amountBtc = "0.01";
  fee.locationKind = "transaction";
  fee.transactionRef = "tx-out";
  return form;
}

describe("guidedComponentModel", () => {
  it("serializes a migration into the daemon spec shape", () => {
    const spec = formToComponentSpec(migrationForm());
    expect(spec.component_type).toBe("manual_bridge");
    // Quantity is the default, so conservation_mode is left off the spec.
    expect(spec.conservation_mode).toBeUndefined();
    const legs = spec.legs as Array<Record<string, unknown>>;
    expect(legs).toHaveLength(3);
    expect(legs[0]).toMatchObject({
      role: "source",
      amount_btc: "1.0",
      transaction: "tx-out",
    });
    expect(legs[2]).toMatchObject({ role: "fee", amount_btc: "0.01" });
    // No allocations for a single-source component.
    expect(spec.allocations).toBeUndefined();
  });

  it("passes local validation with no activation errors when balanced", () => {
    const preview = previewCustodyComponentBatch(formToDocument(migrationForm()));
    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
  });

  it("flags an unbalanced quantity as an activation error", () => {
    const form = migrationForm();
    form.legs[1].amountBtc = "0.98"; // 0.98 + 0.01 fee ≠ 1.0 source
    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors.some((issue) => issue.code === "quantityUnbalanced")).toBe(
      true,
    );
  });

  it("requires occurred_at for a transactionless untracked owned leg", () => {
    const form = createInitialGuidedForm();
    form.legs = [
      (() => {
        const leg = createGuidedLeg("source");
        leg.amountBtc = "1.0";
        leg.locationKind = "transaction";
        leg.transactionRef = "tx-out";
        return leg;
      })(),
      (() => {
        const leg = createGuidedLeg("retained");
        leg.amountBtc = "1.0";
        leg.locationKind = "untracked";
        leg.untrackedWallet = "Old migration wallet";
        return leg; // occurredAt intentionally blank
      })(),
    ];
    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(
      preview.structuralErrors.some((i) => i.code === "transactionlessTimeRequired"),
    ).toBe(true);

    // Providing occurred_at clears the structural error.
    form.legs[1].occurredAt = "2024-01-15T12:00";
    const spec = formToComponentSpec(form);
    const legs = spec.legs as Array<Record<string, unknown>>;
    expect(legs[1].untracked_wallet).toBe("Old migration wallet");
    // Serialized as RFC3339 UTC representing the same wall-clock instant
    // (timezone-independent so the assertion holds under any test TZ).
    expect(String(legs[1].occurred_at)).toMatch(/Z$/);
    expect(new Date(String(legs[1].occurred_at)).getTime()).toBe(
      new Date("2024-01-15T12:00").getTime(),
    );
    const cleared = previewCustodyComponentBatch(formToDocument(form));
    expect(
      cleared.structuralErrors.some((i) => i.code === "transactionlessTimeRequired"),
    ).toBe(false);
  });

  it("requires and satisfies N:M allocations by leg ordinal", () => {
    const form = createInitialGuidedForm();
    const s1 = createGuidedLeg("source");
    s1.amountBtc = "0.5";
    s1.transactionRef = "tx-a";
    const s2 = createGuidedLeg("source");
    s2.amountBtc = "0.5";
    s2.transactionRef = "tx-b";
    const dest = createGuidedLeg("destination");
    dest.amountBtc = "0.99";
    dest.transactionRef = "tx-c";
    const fee = createGuidedLeg("fee");
    fee.amountBtc = "0.01";
    fee.transactionRef = "tx-a";
    form.legs = [s1, s2, dest, fee];

    // Two sources feeding an owned sink + a fee needs explicit allocations.
    const missing = previewCustodyComponentBatch(formToDocument(form));
    expect(missing.activationErrors.some((i) => i.code === "allocationsRequired")).toBe(
      true,
    );

    const alloc = (source: string, sink: string, amountBtc: string) => {
      const a = createGuidedAllocation();
      a.sourceKey = source;
      a.sinkKey = sink;
      a.amountBtc = amountBtc;
      return a;
    };
    form.allocations = [
      alloc(s1.key, dest.key, "0.5"),
      alloc(s2.key, dest.key, "0.49"),
      alloc(s2.key, fee.key, "0.01"),
    ];

    const spec = formToComponentSpec(form);
    const allocs = spec.allocations as Array<Record<string, unknown>>;
    expect(allocs[0]).toMatchObject({
      source_ordinal: 0,
      sink_ordinal: 2,
      source_amount_msat: "50000000000",
      sink_amount_msat: "50000000000",
    });
    const satisfied = previewCustodyComponentBatch(formToDocument(form));
    expect(satisfied.structuralErrors).toEqual([]);
    expect(satisfied.activationErrors).toEqual([]);
  });

  it("serializes a reviewed conversion with per-leg valuations", () => {
    const form = createInitialGuidedForm();
    form.componentType = "swap";
    form.conservationMode = "conversion";
    form.conversionPolicy = "taxable_swap";
    form.conversionReviewed = true;
    const source = createGuidedLeg("source");
    source.amountBtc = "1.0";
    source.transactionRef = "tx-a";
    source.valuationUnit = "eur_cents";
    source.valuationAmount = "5000000";
    const dest = createGuidedLeg("destination");
    dest.amountBtc = "1.0";
    dest.transactionRef = "tx-b";
    dest.asset = "LBTC";
    dest.valuationUnit = "eur_cents";
    dest.valuationAmount = "5000000";
    form.legs = [source, dest];

    const spec = formToComponentSpec(form);
    expect(spec.conservation_mode).toBe("conversion");
    expect(spec.conversion_policy).toBe("taxable_swap");
    expect(spec.conversion_reviewed).toBe(true);
    const legs = spec.legs as Array<Record<string, unknown>>;
    expect(legs[0]).toMatchObject({
      valuation_unit: "eur_cents",
      valuation_amount: "5000000",
    });
    expect(legs[1].asset).toBe("LBTC");

    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
  });

  it("flags an unreviewed conversion as needing review", () => {
    const form = createInitialGuidedForm();
    form.componentType = "swap";
    form.conservationMode = "conversion";
    form.conversionPolicy = "";
    form.conversionReviewed = false;
    const source = createGuidedLeg("source");
    source.amountBtc = "1.0";
    source.transactionRef = "tx-a";
    source.valuationUnit = "eur_cents";
    source.valuationAmount = "5000000";
    const dest = createGuidedLeg("destination");
    dest.amountBtc = "1.0";
    dest.transactionRef = "tx-b";
    dest.valuationUnit = "eur_cents";
    dest.valuationAmount = "5000000";
    form.legs = [source, dest];

    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(
      preview.activationErrors.some((i) => i.code === "conversionReviewRequired"),
    ).toBe(true);
  });

  it("activates a suspense component with an allocation from a source", () => {
    const form = createInitialGuidedForm(); // manual_bridge, quantity, grade reviewed
    const source = createGuidedLeg("source");
    source.amountBtc = "1.0";
    source.transactionRef = "tx-a";
    const dest = createGuidedLeg("destination");
    dest.amountBtc = "0.6";
    dest.transactionRef = "tx-b";
    const suspense = createGuidedLeg("suspense");
    suspense.amountBtc = "0.4";
    suspense.occurredAt = "2024-02-01T10:00";
    form.legs = [source, dest, suspense];
    const edge = (sink: string, amountBtc: string) => {
      const a = createGuidedAllocation();
      a.sourceKey = source.key;
      a.sinkKey = sink;
      a.amountBtc = amountBtc;
      return a;
    };
    form.allocations = [edge(dest.key, "0.6"), edge(suspense.key, "0.4")];

    const spec = formToComponentSpec(form);
    const legs = spec.legs as Array<Record<string, unknown>>;
    // A suspense leg carries no wallet/transaction anchor — only a time.
    expect(legs[2].role).toBe("suspense");
    expect(legs[2].transaction).toBeUndefined();
    expect(legs[2].wallet).toBeUndefined();
    expect(legs[2].untracked_wallet).toBeUndefined();
    expect(legs[2].occurred_at).toBeDefined();

    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
  });

  it("round-trips an existing component through the form for revise", () => {
    const component: CustodyComponentInput = {
      component_type: "manual_bridge",
      conservation_mode: "quantity",
      evidence_kind: "manual_migration_review",
      evidence_grade: "reviewed",
      conversion_policy: null,
      conversion_reviewed: false,
      notes: "migration",
      legs: [
        {
          id: "L1",
          role: "source",
          asset: "BTC",
          amount_msat: 100_000_000_000,
          transaction_id: "tx-out",
          anchor_transaction_id: "tx-out",
          wallet_id: "w1",
          occurred_at: null,
          rail: "bitcoin",
          exposure: "bitcoin",
          conservation_unit: "msat",
        },
        {
          id: "L2",
          role: "destination",
          asset: "BTC",
          amount_msat: 99_000_000_000,
          transaction_id: "tx-in",
          wallet_id: "w2",
          occurred_at: null,
        },
        {
          id: "L3",
          role: "fee",
          asset: "BTC",
          amount_msat: 1_000_000_000,
          transaction_id: "tx-out",
          wallet_id: "w1",
          occurred_at: null,
        },
      ],
      allocations: [],
    };

    const form = componentToFormState(component);
    expect(form.legs[0].locationMode).toBe("origin");
    expect(form.legs[0].origin?.transactionId).toBe("tx-out");
    expect(form.legs[0].amountBtc).toBe("1");
    expect(form.legs[1].amountBtc).toBe("0.99");

    const spec = formToComponentSpec(form);
    const legs = spec.legs as Array<Record<string, unknown>>;
    expect(legs[0]).toMatchObject({
      transaction_id: "tx-out",
      anchor_transaction_id: "tx-out",
      amount_btc: "1",
      rail: "bitcoin",
    });
    // Resolved-location legs must NOT emit alias fields.
    expect(legs[0].transaction).toBeUndefined();
    expect(legs[0].wallet).toBeUndefined();

    const preview = previewCustodyComponentBatch(formToDocument(form));
    expect(preview.structuralErrors).toEqual([]);
    expect(preview.activationErrors).toEqual([]);
  });

  it("formats exact msat as a lossless BTC input string", () => {
    expect(msatToBtcInput(100_000_000_000)).toBe("1");
    expect(msatToBtcInput(99_000_000_000)).toBe("0.99");
    expect(msatToBtcInput(1)).toBe("0.00000000001");
    expect(msatToBtcInput(0)).toBe("0");
    expect(msatToBtcInput("250000000000")).toBe("2.5");
  });

  it("converts datetime-local values to RFC3339 UTC", () => {
    const converted = occurredAtToRfc3339("2024-01-15T12:00");
    expect(converted).toMatch(/Z$/);
    expect(new Date(converted).getTime()).toBe(new Date("2024-01-15T12:00").getTime());
    expect(occurredAtToRfc3339("")).toBe("");
  });
});
