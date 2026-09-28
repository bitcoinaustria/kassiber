import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: ComponentProps<"a">) => <a>{children}</a>,
}));

import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";

import { HoldingsBySourceChart } from "./OverviewSidePanel";

const UNCOVERED: FiatCompleteness = {
  state: "incomplete",
  costBasisComplete: false,
  reasons: ["quarantines"],
  quarantineCount: 1,
  quarantinedInboundMsat: 12_000_000_000,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: 100_000_000_000,
  basisUncoveredMsat: 12_000_000_000,
  earliestIncompleteAt: "2026-01-03T10:00:00Z",
  missingPriceCount: 0,
  marketRateMissing: false,
};

describe("holdings header", () => {
  it("masks the uncovered BTC amount when sensitive values are hidden", () => {
    const render = (hideSensitive: boolean) =>
      renderToStaticMarkup(
        createElement(HoldingsBySourceChart, {
          snapshot: { ...MOCK_OVERVIEW, fiat: { ...MOCK_OVERVIEW.fiat, completeness: UNCOVERED } },
          hideSensitive,
          currency: "eur",
        }),
      );
    const hidden = render(true);
    const amountLine = hidden.match(/<p class="([^"]*)"[^>]*>[^<]*0\.12/);
    expect(amountLine?.[1]).toContain("sensitive");
    expect(render(false).match(/<p class="([^"]*)"[^>]*>[^<]*0\.12/)?.[1]).not.toContain("sensitive");
  });
});
