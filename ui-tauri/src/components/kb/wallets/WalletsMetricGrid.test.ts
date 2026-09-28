import { createElement } from "react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", async () => {
  const React = await import("react");
  return {
    Link: ({
      children,
      className,
      hash,
      to,
    }: {
      children?: ReactNode;
      className?: string;
      hash?: string;
      to?: string;
    }) =>
      React.createElement(
        "a",
        {
          className,
          href: `${typeof to === "string" ? to : "#"}${hash ? `#${hash}` : ""}`,
        },
        children,
      ),
  };
});

import { normalizeOverviewSnapshot } from "@/lib/normalizeUiSnapshots";
import { MOCK_OVERVIEW } from "@/mocks/seed";

import { WalletsMetricGrid } from "./WalletsMetricGrid";

describe("wallets metric grid", () => {
  it("explains that quarantine affects accounting, not wallet custody", () => {
    const html = renderToStaticMarkup(
      createElement(WalletsMetricGrid, {
        balanceSummary: {
          totalBtc: 1.236,
          status: "quarantines",
          source: "mixed",
          needsJournals: false,
          quarantines: 3,
          chainWalletCount: 1,
          bookWalletCount: 1,
          transactionWalletCount: 0,
          duplicateOutpointAdjustmentBtc: 0,
        },
        connections: MOCK_OVERVIEW.connections,
        currency: "btc",
        hideSensitive: false,
        isSyncing: false,
        priceEur: MOCK_OVERVIEW.priceEur,
        totalBtc: 1.236,
      }),
    );

    expect(html).toContain("Wallet balance");
    expect(html).toContain("3 quarantines");
    expect(html).toContain("accounting view may change after review");
  });

  it("shows the daemon tax-free balance on the wallets overview", () => {
    const html = renderToStaticMarkup(
      createElement(WalletsMetricGrid, {
        connections: MOCK_OVERVIEW.connections,
        currency: "btc",
        hideSensitive: false,
        isSyncing: false,
        priceEur: MOCK_OVERVIEW.priceEur,
        taxFreeBalance: {
          ...MOCK_OVERVIEW.taxFreeBalance!,
          taxFreeQuantitySats: 0,
          taxableQuantitySats: 123_600_000,
          totalQuantitySats: 123_600_000,
        },
        totalBtc: 1.236,
      }),
    );

    expect(html).toContain("Tax-free balance");
    expect(html).toContain("\u20bf 0.000");
    expect(html).toContain("Taxable \u20bf 1.236");
  });

  it("blocks stale tax-free wallet values behind journal readiness", () => {
    const html = renderToStaticMarkup(
      createElement(WalletsMetricGrid, {
        connections: MOCK_OVERVIEW.connections,
        currency: "btc",
        hideSensitive: false,
        isSyncing: false,
        priceEur: MOCK_OVERVIEW.priceEur,
        taxFreeBalance: {
          ...MOCK_OVERVIEW.taxFreeBalance!,
          status: "needs_journals",
          needsJournals: true,
        },
        totalBtc: 1.236,
      }),
    );

    expect(html).toContain("Tax-free balance");
    expect(html).toContain("Needs journals");
    expect(html).toContain("Run journals before relying on this balance");
  });

  it("keeps the daemon's de-duplicated total and quarantine caveat through the normalizer", () => {
    // Two chain wallets watch the same coin: tiles sum to 2 BTC, the daemon's
    // balanceSummary says 1 BTC. The Wallets page reads the normalized payload.
    const snapshot = normalizeOverviewSnapshot({
      ...MOCK_OVERVIEW,
      connections: [
        { ...MOCK_OVERVIEW.connections[0], id: "a", balance: 1 },
        { ...MOCK_OVERVIEW.connections[0], id: "b", balance: 1 },
      ],
      balanceSummary: {
        totalBtc: 1,
        status: "quarantines",
        source: "chain",
        needsJournals: false,
        quarantines: 2,
        chainWalletCount: 2,
        bookWalletCount: 0,
        transactionWalletCount: 0,
        duplicateOutpointAdjustmentBtc: 1,
      },
    });
    const html = renderToStaticMarkup(
      createElement(WalletsMetricGrid, {
        balanceSummary: snapshot.balanceSummary,
        connections: snapshot.connections,
        currency: "btc",
        hideSensitive: false,
        isSyncing: false,
        priceEur: snapshot.priceEur,
        totalBtc: snapshot.balanceSummary?.totalBtc ?? 2,
      }),
    );

    expect(html).toContain("1.00000000");
    expect(html).not.toContain("2.00000000");
    expect(html).toContain("2 quarantines");
  });

  it("shows a dash for the fiat wallet total without a market rate", () => {
    const html = renderToStaticMarkup(
      createElement(WalletsMetricGrid, {
        connections: MOCK_OVERVIEW.connections,
        currency: "eur",
        hideSensitive: false,
        isSyncing: false,
        marketRateMissing: true,
        priceEur: 0,
        totalBtc: 1.236,
      }),
    );

    expect(html).toContain("—");
    expect(html).toContain("No market rate · fiat value unavailable");
    expect(html).not.toContain("€ 0,00");
  });
});
