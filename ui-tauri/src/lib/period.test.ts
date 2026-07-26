import { describe, expect, it } from "vitest";

import {
  autoCandidatePeriods,
  historyYearsBetween,
  normalizePeriodParam,
  selectablePeriods,
} from "./period";

describe("period keys", () => {
  it("folds hand-typed period params onto canonical keys", () => {
    expect(normalizePeriodParam("auto")).toBe("auto");
    expect(normalizePeriodParam("automatic")).toBe("auto");
    expect(normalizePeriodParam("30d")).toBe("30days");
    expect(normalizePeriodParam("6MO")).toBe("6months");
    expect(normalizePeriodParam("1year")).toBe("1year");
    expect(normalizePeriodParam("1y")).toBe("1year");
    expect(normalizePeriodParam("5yrs")).toBe("5years");
    expect(normalizePeriodParam("15 years")).toBe("15years");
    expect(normalizePeriodParam("max")).toBe("all");
    expect(normalizePeriodParam("YTD")).toBe("ytd");
    expect(normalizePeriodParam("7months")).toBeNull();
    expect(normalizePeriodParam("")).toBeNull();
    expect(normalizePeriodParam(null)).toBeNull();
  });

  it("only offers ranges the book's history can fill", () => {
    expect(selectablePeriods(0)).toEqual([
      "30days",
      "3months",
      "6months",
      "ytd",
      "1year",
      "all",
    ]);
    expect(selectablePeriods(6)).toContain("5years");
    expect(selectablePeriods(6)).not.toContain("10years");
    expect(selectablePeriods(16)).toEqual([
      "30days",
      "3months",
      "6months",
      "ytd",
      "1year",
      "5years",
      "10years",
      "15years",
      "all",
    ]);
  });

  it("lets auto reach one step past the selectable long ranges", () => {
    // A 9-year book gets a bounded 10-year auto window instead of "all".
    expect(autoCandidatePeriods(9)).toEqual(["ytd", "1year", "5years", "10years", "all"]);
    expect(autoCandidatePeriods(1)).toEqual(["ytd", "1year", "all"]);
  });

  it("measures history in years between two dates", () => {
    expect(
      Math.round(
        historyYearsBetween(
          new Date("2016-07-01T00:00:00Z"),
          new Date("2026-07-01T00:00:00Z"),
        ),
      ),
    ).toBe(10);
    expect(
      historyYearsBetween(
        new Date("2026-07-01T00:00:00Z"),
        new Date("2016-07-01T00:00:00Z"),
      ),
    ).toBe(0);
  });
});
