import { describe, expect, it } from "vitest";

import type { FiatCompleteness } from "@/mocks/seed";

import {
  UNKNOWN_FIAT_COMPLETENESS,
  basisIncompleteFromMs,
  completenessHref,
  fiatCompleteness,
  isBasisIncompleteAt,
  isBasisIncompleteOnDay,
  normalizeFiatCompleteness,
} from "./fiatCompleteness";

const GAP: FiatCompleteness = {
  ...UNKNOWN_FIAT_COMPLETENESS,
  state: "incomplete",
  reasons: ["quarantines"],
  quarantineCount: 1,
  earliestIncompleteAt: "2026-03-10T15:00:00Z",
};

describe("fiat completeness helpers", () => {
  it("never reads a missing block as complete", () => {
    expect(fiatCompleteness(undefined).costBasisComplete).toBe(false);
    expect(fiatCompleteness({}).state).toBe("unavailable");
  });

  it("dates the gap from earliestIncompleteAt", () => {
    expect(basisIncompleteFromMs(GAP)).toBe(Date.parse("2026-03-10T15:00:00Z"));
    expect(
      basisIncompleteFromMs({ ...GAP, costBasisComplete: true }),
    ).toBeNull();
    expect(basisIncompleteFromMs({ ...GAP, earliestIncompleteAt: null })).toBe(
      Number.NEGATIVE_INFINITY,
    );
  });

  it("treats the end-of-day point that holds the gap as affected", () => {
    expect(isBasisIncompleteOnDay(GAP, "2026-03-09")).toBe(false);
    expect(isBasisIncompleteOnDay(GAP, "2026-03-10")).toBe(true);
    expect(isBasisIncompleteOnDay(GAP, "2026-03-11")).toBe(true);
  });

  it("compares events by instant", () => {
    expect(isBasisIncompleteAt(GAP, Date.parse("2026-03-10T14:59:59Z"))).toBe(false);
    expect(isBasisIncompleteAt(GAP, Date.parse("2026-03-10T15:00:00Z"))).toBe(true);
  });

  it("routes stale books to journals and quarantines to the queue", () => {
    expect(completenessHref(GAP)).toBe("/quarantine");
    expect(completenessHref({ ...GAP, state: "stale" })).toBe("/journals");
    expect(
      completenessHref({
        ...GAP,
        quarantineCount: 0,
        reasons: ["custody_unresolved"],
      }),
    ).toBe("/journals");
  });
});

describe("normalizeFiatCompleteness", () => {
  it("does not honor a complete flag without a recognized state", () => {
    expect(normalizeFiatCompleteness({ costBasisComplete: true }).costBasisComplete).toBe(false);
    expect(
      normalizeFiatCompleteness({ state: "complete", costBasisComplete: true }).costBasisComplete,
    ).toBe(true);
  });
});
