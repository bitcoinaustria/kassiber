import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";

import type { CapitalGainsReport } from "@/mocks/reports";

const state = vi.hoisted(() => ({
  report: null as CapitalGainsReport | null,
  costBasisComplete: true,
}));

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
      return {
        data: {
          data: {
            fiat: {
              completeness: {
                state: state.costBasisComplete ? "complete" : "incomplete",
                costBasisComplete: state.costBasisComplete,
                reasons: state.costBasisComplete ? [] : ["quarantine"],
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
    expect(html).toContain("554 disposals");
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
        { code: "801", label: "", form: "E 1", amount: 40, rowCount: 1 },
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
    expect(html).toContain('href="/quarantine"');
    expect(html.match(/>Provisional</g)).toHaveLength(2);
  });
});
