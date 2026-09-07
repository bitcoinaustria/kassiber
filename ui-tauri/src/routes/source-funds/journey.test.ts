import { describe, expect, it } from "vitest";
import { canApproveSourceFundsPreview, isReviewedSourceFundsPreview } from "./journey";

describe("source-funds disclosure review", () => {
  const ready = { current: true, exportable: true, diagramLoading: false, diagramError: false, fingerprint: "current-case" };
  it("allows confirmation only after the current report and printable preview are ready", () => {
    expect(canApproveSourceFundsPreview(ready)).toBe(true);
    for (const change of [{ current: false }, { exportable: false }, { diagramLoading: true }, { diagramError: true }, { fingerprint: undefined }]) {
      expect(canApproveSourceFundsPreview({ ...ready, ...change })).toBe(false);
    }
  });
});

it("requires a fresh explicit confirmation for changed evidence or disclosure", () => {
  expect(isReviewedSourceFundsPreview(true, true, "new-case", null)).toBe(false);
  expect(isReviewedSourceFundsPreview(true, true, "new-case", "old-case")).toBe(false);
  expect(isReviewedSourceFundsPreview(false, true, "same-case", "same-case")).toBe(false);
  expect(isReviewedSourceFundsPreview(true, false, "same-case", "same-case")).toBe(false);
  expect(isReviewedSourceFundsPreview(true, true, "same-case", "same-case")).toBe(true);
});
