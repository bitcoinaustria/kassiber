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
      to,
    }: {
      children?: ReactNode;
      className?: string;
      to?: string;
    }) =>
      React.createElement(
        "a",
        { className, href: typeof to === "string" ? to : "#" },
        children,
      ),
  };
});

import { MOCK_OVERVIEW } from "@/mocks/seed";
import type { OverviewSnapshot } from "@/mocks/seed";

import { HoldingsBySourceChart } from "./OverviewSidePanel";

describe("overview holdings summary", () => {
  it("shows the daemon tax-free balance in the asset summary", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      connections: [
        {
          ...MOCK_OVERVIEW.connections[0],
          label: "Cold Storage",
          balance: 1.2,
        },
      ],
      taxFreeBalance: {
        ...MOCK_OVERVIEW.taxFreeBalance!,
        taxFreeQuantitySats: 0,
        taxableQuantitySats: 120_000_000,
        totalQuantitySats: 120_000_000,
        buckets: [
          {
            ...MOCK_OVERVIEW.taxFreeBalance!.buckets[0],
            quantitySats: 0,
          },
          {
            ...MOCK_OVERVIEW.taxFreeBalance!.buckets[1],
            quantitySats: 120_000_000,
          },
        ],
      },
    };

    const html = renderToStaticMarkup(
      createElement(HoldingsBySourceChart, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("Tax-free");
    expect(html).toContain("\u20bf 0.000");
    expect(html).toContain("Taxable \u20bf 1.200");
  });

  it("blocks stale tax-free summary values behind review copy", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      connections: [MOCK_OVERVIEW.connections[0]],
      taxFreeBalance: {
        ...MOCK_OVERVIEW.taxFreeBalance!,
        status: "needs_journals",
        needsJournals: true,
      },
    };

    const html = renderToStaticMarkup(
      createElement(HoldingsBySourceChart, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("Needs journals");
    expect(html).toContain("Review required");
    expect(html).toContain("Run journals before relying on this balance");
  });
});
