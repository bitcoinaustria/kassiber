import { describe, expect, it } from "vitest";

import { normalizeReportYear, reportYearFromSearch } from "./reportYear";

describe("reportYear", () => {
  it("accepts plausible four-digit tax years", () => {
    expect(normalizeReportYear("2009")).toBe(2009);
    expect(normalizeReportYear("2026")).toBe(2026);
    expect(normalizeReportYear("2100")).toBe(2100);
  });

  it("rejects malformed or implausible tax years", () => {
    expect(normalizeReportYear(null)).toBeNull();
    expect(normalizeReportYear("0")).toBeNull();
    expect(normalizeReportYear("2008")).toBeNull();
    expect(normalizeReportYear("2101")).toBeNull();
    expect(normalizeReportYear("2025.5")).toBeNull();
    expect(normalizeReportYear("not-a-year")).toBeNull();
  });

  it("reads valid years from URL search strings", () => {
    expect(reportYearFromSearch("?year=2025&period=1year")).toBe(2025);
    expect(reportYearFromSearch("?year=0")).toBeNull();
    expect(reportYearFromSearch("?period=1year")).toBeNull();
  });
});
