import { describe, expect, it } from "vitest";

import type { CapitalGainsReport, ReportBreakdownBucket } from "@/mocks/reports";

import {
  describeTaxYear,
  formatReportPeriod,
  formatTaxRate,
  methodLabel,
  reportBreakdown,
  reportRegion,
  reportTaxEstimate,
  reportTaxFieldRows,
  reportTotals,
  transactionsHref,
} from "./reportModel";

const report = (overrides: Partial<CapitalGainsReport> = {}): CapitalGainsReport => ({
  jurisdictionCode: "GENERIC",
  year: 2025,
  method: "fifo",
  ...overrides,
});

const bucket = (
  key: string,
  overrides: Partial<ReportBreakdownBucket> = {},
): ReportBreakdownBucket => ({
  key,
  rowCount: 2,
  transactionCount: 2,
  transactionIds: ["tx-a", "tx-b"],
  costEur: 100,
  proceedsEur: 150,
  incomeEur: 0,
  gainLossEur: 50,
  ...overrides,
});

describe("reportRegion", () => {
  it("reads Austria only from AT and everything else as generic", () => {
    expect(reportRegion("AT")).toBe("AT");
    expect(reportRegion("at")).toBe("AT");
    expect(reportRegion("GENERIC")).toBe("GENERIC");
    // Never fall back to Austrian rules for a code the screen does not know.
    expect(reportRegion("DE")).toBe("GENERIC");
    expect(reportRegion(undefined)).toBe("GENERIC");
  });
});

describe("methodLabel", () => {
  it("shows the book's method as stored, whatever the region", () => {
    expect(methodLabel("fifo")).toBe("FIFO · first in, first out");
    expect(methodLabel("moving_average_at")).toMatch(/^Austrian method/);
    expect(methodLabel("something_new")).toBe("SOMETHING_NEW");
  });
});

describe("reportTotals", () => {
  it("uses the daemon's year totals", () => {
    const totals = reportTotals(
      report({
        totals: {
          rowCount: 554,
          costEur: 640_808.28,
          proceedsEur: 1_777_474.85,
          incomeEur: 0,
          gainLossEur: 1_136_666.57,
        },
      }),
    );

    expect(totals).toEqual({
      rowCount: 554,
      proceeds: 1_777_474.85,
      cost: 640_808.28,
      gain: 1_136_666.57,
      income: 0,
      taxFreeGain: 0,
    });
    expect(reportTotals(report()).rowCount).toBe(0);
  });

  it("reads the tax-free part of the gain from the core's breakdown", () => {
    const totals = reportTotals(
      report({
        jurisdictionCode: "AT",
        breakdown: [bucket("174"), bucket("tax_free", { gainLossEur: 300 })],
      }),
    );

    expect(totals.taxFreeGain).toBe(300);
  });
});

describe("reportTaxEstimate", () => {
  it("is absent unless the core sends one", () => {
    expect(reportTaxEstimate(report())).toBeNull();
    expect(reportTaxEstimate(report({ indicativeTax: null }))).toBeNull();
  });

  it("reads the core's cents and sums the amounts it does not estimate", () => {
    const estimate = reportTaxEstimate(
      report({
        jurisdictionCode: "AT",
        indicativeTax: {
          estimatedTaxEurCents: 3_636_719,
          rate: "0.275",
          basis: "E 1kv KZ 172 + KZ 174 - KZ 176, this book only",
          progressiveRateAmountsEurCents: { "801": 4_000 },
          notes: ["Indicative only."],
        },
      }),
    );

    expect(estimate).toEqual({
      amount: 36_367.19,
      progressiveAmount: 40,
      notes: ["Indicative only."],
    });
  });

  it("formats the core's rate", () => {
    expect(formatTaxRate("0.275")).toBe("27.5 %");
    expect(formatTaxRate(undefined)).toBe("");
  });
});

describe("reportTaxFieldRows", () => {
  it("lists the Austrian fields awaiting figures and none for a generic book", () => {
    expect(
      reportTaxFieldRows(report({ jurisdictionCode: "AT" })).map((row) => row.code),
    ).toEqual(["172", "174", "176", "801"]);
    expect(reportTaxFieldRows(report())).toEqual([]);
  });

  it("awaits only the E 1's field for a year before the E 1kv", () => {
    expect(
      reportTaxFieldRows(report({ jurisdictionCode: "AT", e1kvApplies: false })).map((row) => row.code),
    ).toEqual(["801"]);
  });
});

describe("transactionsHref", () => {
  it("names the transactions, or opens the year when there are none to name", () => {
    expect(transactionsHref(2025, ["tx-a", "tx-b"])).toBe(
      "/transactions?period=2025&txids=tx-a%2Ctx-b",
    );
    expect(transactionsHref(2025, null)).toBe("/transactions?period=2025");
  });
});

describe("reportBreakdown", () => {
  it("lists a generic book's holding periods and income in order", () => {
    const groups = reportBreakdown(
      report({
        breakdown: [
          // An income row without a recorded gain still counts its value.
          bucket("income", { proceedsEur: 0, incomeEur: 70, gainLossEur: 0 }),
          bucket("long_term"),
        ],
      }),
    );

    expect(groups).toHaveLength(1);
    expect(groups[0].rows.map((row) => [row.label, row.amount])).toEqual([
      ["Long-term gains", 50],
      ["Income", 70],
    ]);
    expect(groups[0].rows[0].href).toBe(
      "/transactions?period=2025&txids=tx-a%2Ctx-b",
    );
    expect(groups[0].rows[0].exact).toBe(true);
    expect(reportBreakdown(report())).toEqual([]);
  });

  it("opens the whole year when a group has too many transactions to name", () => {
    const [group] = reportBreakdown(
      report({
        breakdown: [
          bucket("short_term", { transactionCount: 300, transactionIds: null }),
        ],
      }),
    );

    expect(group.rows[0].href).toBe("/transactions?period=2025");
    expect(group.rows[0].exact).toBe(false);
  });

  it("files an Austrian book's fields by form and keeps the rest off the return", () => {
    const groups = reportBreakdown(
      report({
        jurisdictionCode: "AT",
        kennzahlRows: [
          { code: "172", label: "", form: "E 1kv", amount: 0, rowCount: 0 },
          { code: "174", label: "", form: "E 1kv", amount: 132_244.32, rowCount: 74 },
          { code: "176", label: "", form: "E 1kv", amount: 0, rowCount: 0 },
          { code: "801", label: "", form: "E 1", amount: 40, rowCount: 1 },
        ],
        breakdown: [
          bucket("174", { transactionCount: 74, transactionIds: null }),
          bucket("801", { transactionIds: ["tx-alt"], transactionCount: 1 }),
          bucket("tax_free", { gainLossEur: 900 }),
          bucket("swaps", { gainLossEur: 0 }),
        ],
      }),
    );

    expect(groups.map((group) => [group.title, group.reported])).toEqual([
      ["E 1kv", true],
      ["E 1 · outside the E 1kv", true],
      ["Not reported", false],
    ]);
    const [e1kv, e1, offReturn] = groups;
    // The field's amount is the core's, not the bucket's sum.
    expect(e1kv.rows[1]).toMatchObject({
      code: "KZ 174",
      amount: 132_244.32,
      transactionCount: 74,
      href: "/transactions?period=2025",
    });
    expect(e1kv.rows[0]).toMatchObject({ code: "KZ 172", transactionCount: 0, href: null });
    expect(e1.rows[0].note).toBe("Taxed at your income-tax rate");
    expect(offReturn.rows.map((row) => [row.label, row.amount])).toEqual([
      ["Tax-free Altbestand", 900],
      ["Tax-neutral swaps", null],
    ]);
  });
});

describe("tax year periods", () => {
  it("labels what the report filters by: the calendar year", () => {
    expect(formatReportPeriod(2025)).toBe("1 Jan – 31 Dec 2025");
    expect(describeTaxYear("generic")).toBe("Calendar year, UTC");
    expect(describeTaxYear("at")).toBe("Calendar year, Europe/Vienna");
  });

  it("says a later tax-year start is not applied, rather than borrowing its dates", () => {
    // The UK tax year starts on 6 April; `ui.reports.capital_gains` still
    // filters the calendar year.
    expect(describeTaxYear("gb")).toBe(
      "Calendar year, Europe/London · 6 Apr tax year not applied yet",
    );
  });
});
