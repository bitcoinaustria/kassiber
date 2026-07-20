import { describe, expect, it } from "vitest";

import { previewCustodyComponentBatch } from "@/lib/custodyComponentBulk";
import {
  createGuidedAllocation,
  createGuidedLeg,
  createInitialGuidedForm,
  formToComponentSpec,
  formToDocument,
  occurredAtToRfc3339,
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

  it("converts datetime-local values to RFC3339 UTC", () => {
    const converted = occurredAtToRfc3339("2024-01-15T12:00");
    expect(converted).toMatch(/Z$/);
    expect(new Date(converted).getTime()).toBe(new Date("2024-01-15T12:00").getTime());
    expect(occurredAtToRfc3339("")).toBe("");
  });
});
