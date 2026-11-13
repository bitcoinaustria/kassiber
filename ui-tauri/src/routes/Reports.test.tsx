import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";

import type { CapitalGainsReport } from "@/mocks/reports";

const state = vi.hoisted(() => ({
  report: null as CapitalGainsReport | null,
  costBasisComplete: true,
  overviewLoading: false,
}));

// Source blockers come from the bell's conditions for the open book; these
// tests cover the page's own readiness rules.
vi.mock("@/lib/reportBlockers", () => ({ useBlockingSourceCount: () => 0 }));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to: string }) => (
    <a href={to}>{children}</a>
  ),
  useNavigate: () => () => {},
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: (kind: string) => {
    if (kind === "ui.reports.capital_gains") {
      return { data: { data: state.report }, isLoading: false };
    }
    if (kind === "ui.overview.snapshot") {
      if (state.overviewLoading) return { isLoading: true };
      return {
        data: {
          data: {
            fiat: {
              completeness: {
                state: state.costBasisComplete ? "complete" : "incomplete",
                costBasisComplete: state.costBasisComplete,
                reasons: state.costBasisComplete ? [] : ["quarantines"],
              },
            },
          },
        },
      };
    }
    return {};
  },
  useDaemonMutation: () => ({ mutateAsync: vi.fn() }),
}));

vi.mock("@/components/lightning/LightningProfitabilityPanel", () => ({
  LightningProfitabilityPanel: () => null,
}));

import { Reports } from "./Reports";

// The currency formatter separates symbol and amount with a no-break space.
const render = () => renderToStaticMarkup(<Reports />).replace(/\u00a0/g, " ");

const totals = {
  rowCount: 554,
  costEur: 640_808.28,
  proceedsEur: 1_777_474.85,
  incomeEur: 0,
  gainLossEur: 1_136_666.57,
};

const breakdown = [
  {
    key: "long_term",
    rowCount: 554,
    transactionCount: 2,
    transactionIds: ["tx-a", "tx-b"],
    costEur: 640_808.28,
    proceedsEur: 1_777_474.85,
    incomeEur: 0,
    gainLossEur: 1_136_666.57,
  },
];

describe("Reports", () => {
  beforeEach(() => {
    state.costBasisComplete = true;
    state.overviewLoading = false;
  });

  it("shows a generic book's own rules and no tax estimate", () => {
    state.report = {
      jurisdictionCode: "GENERIC",
      fiatCurrency: "EUR",
      longTermDays: 365,
      year: 2025,
      method: "fifo",
      totals,
      kennzahlRows: [],
      indicativeTax: null,
    };
    const html = render();

    // The year's totals, not the sum of the capped rows.
    expect(html).toContain("€ 1.777.474,85");
    // The table counts transactions, so the tile counts no journal rows.
    expect(html).not.toContain("554 disposal");
    expect(html).not.toContain("KESt");
    expect(html).not.toContain("Indicative tax");
    expect(html).not.toContain("Tax fields");
    expect(html).toContain("Every year in the book");
    expect(html).toContain('id="report-rules-title"');
    expect(html).toContain('href="/books"');
    expect(html).toContain("Long-term after");
  });

  it("states how the result is built and leaves the rows to Transactions", () => {
    state.report = {
      jurisdictionCode: "GENERIC",
      fiatCurrency: "EUR",
      year: 2025,
      method: "fifo",
      totals,
      breakdown,
    };
    const html = render();

    expect(html).toContain("Gains and income");
    expect(html).toContain("Long-term gains");
    expect(html).toContain('href="/transactions?period=2025&amp;txids=tx-a%2Ctx-b"');
    expect(html).toContain('href="/transactions?period=2025"');
    // No row tables, tabs, or ledger exports of its own.
    expect(html).not.toContain('role="tab"');
    expect(html).not.toContain("Export Transactions");
  });

  it("shows an Austrian book's tax fields and the core's estimate", () => {
    state.report = {
      jurisdictionCode: "AT",
      fiatCurrency: "EUR",
      longTermDays: null,
      year: 2025,
      method: "moving_average_at",
      totals,
      kennzahlRows: [
        { code: "174", label: "", form: "E 1kv", amount: 13_224.43, rowCount: 1 },
        { code: "801", label: "", form: "E 1", amount: -40, rowCount: 1 },
      ],
      indicativeTax: {
        estimatedTaxEurCents: 363_672,
        rate: "0.275",
        basis: "E 1kv KZ 172 + KZ 174 - KZ 176, this book only",
        // A net KZ 801 loss is still left out of the estimate.
        progressiveRateAmountsEurCents: { "801": -4_000 },
        notes: ["Indicative only, not tax advice; a Steuerberater confirms the figure."],
      },
    };
    const html = render();

    expect(html).toContain("Indicative tax · 27.5 %");
    expect(html).toContain("€ 3.636,72");
    expect(html).toContain("KZ 172 + 174 − 176, without KZ 801");
    expect(html).toContain("KZ 174");
    // The core's caveats open from the tile's info button, not in the page.
    expect(html).toContain('aria-label="About Indicative tax · 27.5 %"');
    expect(html).toContain("E 1kv 2025 with every row");
    expect(html).toContain("E 1kv 27.5 % · KZ 801 income-tax rate");
  });

  it("states the tax-free part of the gain and no rate the core did not give", () => {
    state.report = {
      jurisdictionCode: "AT",
      fiatCurrency: "EUR",
      year: 2025,
      method: "moving_average_at",
      totals: { ...totals, incomeEur: 70.2 },
      breakdown: [
        { ...breakdown[0], key: "174" },
        {
          key: "tax_free",
          rowCount: 1,
          transactionCount: 1,
          transactionIds: ["tx-alt"],
          costEur: 10,
          proceedsEur: 310,
          incomeEur: 0,
          gainLossEur: 300,
        },
      ],
      indicativeTax: null,
    };
    const html = render();

    // The tile's figure stays the core's total; the sub-line names its parts.
    expect(html).toContain("+ € 1.136.666,57");
    expect(html).toContain("Includes € 70,20 income · + € 300,00 tax-free (Altbestand)");
    expect(html).toContain("E 1kv · KZ 801 income-tax rate");
    expect(html).not.toContain("27.5 %");
  });

  it("states no crypto Kennzahlen and no estimate for a year before 2022", () => {
    state.report = {
      jurisdictionCode: "AT",
      fiatCurrency: "EUR",
      year: 2021,
      method: "moving_average_at",
      totals,
      e1kvApplies: false,
      breakdown: [
        { ...breakdown[0], key: "801", rowCount: 1 },
        { ...breakdown[0], key: "unassigned", rowCount: 2, transactionIds: ["tx-neu"] },
      ],
      kennzahlRows: [{ code: "801", label: "", form: "E 1", amount: 20, rowCount: 1 }],
      indicativeTax: null,
    };
    const html = render();

    expect(html).toContain("crypto Kennzahlen (KZ 172, 174, 176) start on 1 March 2022, so 2021 states only KZ 801");
    expect(html).toContain("2 rows have no field that year");
    expect(html).not.toContain("KZ 174");
    expect(html).not.toContain("Indicative tax");
    expect(html).toContain("KZ 801 income-tax rate · no crypto Kennzahlen before 2022");
    expect(html).toContain("No crypto Kennzahlen before 2022 · the bookkeeping handoff lists every row");
    expect(html).not.toContain("E 1kv 2021 with every row");
    // The rows in no field may still be taxable.
    expect(html).toContain("text-amber-700");
    expect(html).toContain(">Provisional<");
  });

  it("says up front that a 2022 with rows before March gets no E 1kv", () => {
    state.report = {
      jurisdictionCode: "AT",
      fiatCurrency: "EUR",
      year: 2022,
      method: "moving_average_at",
      totals,
      rowsBeforeCryptoKennzahlen: 2,
      breakdown: [{ ...breakdown[0], key: "unassigned", rowCount: 2 }],
      indicativeTax: null,
    };
    const html = render();

    expect(html).toContain("2 rows from January or February 2022 followed the old rules");
    expect(html).toContain("No E 1kv 2022: rows before 1 March");
    expect(html).not.toContain("E 1kv 2022 with every row");
  });

  it("asks for a look at rows in no field before filing", () => {
    state.report = {
      jurisdictionCode: "AT",
      fiatCurrency: "EUR",
      year: 2025,
      method: "moving_average_at",
      totals,
      breakdown: [{ ...breakdown[0], key: "unassigned", rowCount: 1 }],
      indicativeTax: null,
    };
    const html = render();

    expect(html).toContain("1 row is in no field of the return; check it before filing.");
    expect(html).not.toContain("Ready for export");
  });

  it("names what blocks the figures and marks them provisional", () => {
    state.costBasisComplete = false;
    state.report = {
      jurisdictionCode: "GENERIC",
      year: 2025,
      method: "fifo",
      totals,
      status: { needsJournals: false, quarantines: 3 },
    };
    const html = render();

    expect(html).toContain("3 quarantined items should be resolved before filing.");
    expect(html).toContain('href="/review"');
    expect(html.match(/>Provisional</g)).toHaveLength(2);
    // The pill leads the header with the short state, linking where it is
    // fixed, as on Overview; the notice below explains it.
    expect(html).toContain('href="/review"><svg');
    expect(html).toContain("Review queue open");
    expect(html).toContain("3 quarantined items should be resolved before filing.");
  });

  it("waits for the completeness read before calling the figures provisional", () => {
    state.overviewLoading = true;
    state.report = {
      jurisdictionCode: "GENERIC",
      year: 2025,
      method: "fifo",
      totals,
      status: { needsJournals: false, quarantines: 0 },
    };
    const loading = render();

    expect(loading).toContain("Checking completeness");
    expect(loading).not.toContain('role="status"');
    expect(loading).not.toContain(">Provisional<");
    expect(loading).not.toContain("Completeness unknown");

    // A read that came back without the block stays unknown.
    state.overviewLoading = false;
    state.costBasisComplete = false;
    const unknown = render();
    expect(unknown).toContain("could not confirm that the cost basis is complete");
    expect(unknown).toContain(">Provisional<");
  });
});
