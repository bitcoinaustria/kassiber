import { describe, expect, it } from "vitest";

import {
  autoCandidatePeriods,
  historyYearsBetween,
  isPeriodKey,
  normalizePeriodParam,
  selectablePeriods,
} from "./period";

describe("period keys", () => {
  it("folds hand-typed period params onto canonical keys", () => {
    expect(normalizePeriodParam("auto")).toBe("auto");
    expect(normalizePeriodParam("automatic")).toBe("auto");
    expect(normalizePeriodParam("30d")).toBe("30days");
    expect(normalizePeriodParam("1year")).toBe("1year");
    expect(normalizePeriodParam("1y")).toBe("1year");
    expect(normalizePeriodParam("4yrs")).toBe("4years");
    expect(normalizePeriodParam("10 years")).toBe("10years");
    expect(normalizePeriodParam("max")).toBe("all");
    expect(normalizePeriodParam("2033")).toBe("2033");
    expect(normalizePeriodParam("7months")).toBeNull();
    expect(normalizePeriodParam("")).toBeNull();
    expect(normalizePeriodParam(null)).toBeNull();
  });

  it("lands retired ranges on the closest one offered", () => {
    expect(normalizePeriodParam("3months")).toBe("1year");
    expect(normalizePeriodParam("6MO")).toBe("1year");
    expect(normalizePeriodParam("5years")).toBe("4years");
    expect(normalizePeriodParam("15years")).toBe("10years");
    // Year to date is the running calendar year.
    expect(normalizePeriodParam("YTD", new Date(2031, 4, 2))).toBe("2031");
  });

  it("knows a stored key from anything else", () => {
    expect(isPeriodKey("auto")).toBe(true);
    expect(isPeriodKey("4years")).toBe(true);
    expect(isPeriodKey("2033")).toBe(true);
    expect(isPeriodKey("6months")).toBe(false);
    expect(isPeriodKey(2033)).toBe(false);
  });

  it("only offers ranges the book's history can fill", () => {
    expect(selectablePeriods(0)).toEqual(["30days", "1year", "all"]);
    expect(selectablePeriods(5)).toEqual(["30days", "1year", "4years", "all"]);
    expect(selectablePeriods(30)).toEqual([
      "30days",
      "1year",
      "4years",
      "10years",
      "all",
    ]);
  });

  it("lets auto reach one step past the selectable long ranges", () => {
    // A 9-year book gets a bounded 10-year auto window instead of "all".
    expect(autoCandidatePeriods(9)).toEqual(["1year", "4years", "10years", "all"]);
    expect(autoCandidatePeriods(1)).toEqual(["1year", "all"]);
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
