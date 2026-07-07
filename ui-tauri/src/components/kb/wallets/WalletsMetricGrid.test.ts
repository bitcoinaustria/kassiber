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

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { WalletsMetricGrid } from "./WalletsMetricGrid";

describe("wallets metric grid", () => {
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
});
