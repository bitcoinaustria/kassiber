import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW } from "@/mocks/seed";

import {
  compareWalletsTableConnections,
  WalletsTable,
} from "./WalletsTable";

describe("wallets table", () => {
  const renderTable = (
    props: Partial<Parameters<typeof WalletsTable>[0]> = {},
  ) =>
    renderToStaticMarkup(
      createElement(WalletsTable, {
        connections: MOCK_OVERVIEW.connections,
        currency: "btc",
        hideSensitive: false,
        onSelectConnection: vi.fn(),
        priceEur: MOCK_OVERVIEW.priceEur,
        totalBtc: MOCK_OVERVIEW.connections.reduce(
          (sum, connection) => sum + connection.balance,
          0,
        ),
        totalCount: MOCK_OVERVIEW.connections.length,
        ...props,
      }),
    );

  it("shows a tax-free yes/no column when the tax-free card is present", () => {
    const html = renderTable({
      taxFreeBalance: MOCK_OVERVIEW.taxFreeBalance,
    });

    expect(html).toContain("Tax-free");
    expect(html).toContain("Yes");
    expect(html).toContain("No");
  });

  it("hides stale tax-free wallet flags until journals are current", () => {
    for (const status of ["needs_journals", "quarantines"] as const) {
      const html = renderTable({
        taxFreeBalance: {
          ...MOCK_OVERVIEW.taxFreeBalance!,
          status,
          needsJournals: status === "needs_journals",
          quarantines: status === "quarantines" ? 1 : 0,
        },
      });

      expect(html).not.toContain("Tax-free");
      expect(html).not.toContain("Yes");
    }
  });

  it("sorts tax-free wallets before taxable-only wallets", () => {
    const taxFreeWalletIds = new Set(
      MOCK_OVERVIEW.taxFreeBalance!.wallets!
        .filter((wallet) => wallet.hasTaxFreeBalance)
        .map((wallet) => wallet.walletId),
    );
    const sorted = [...MOCK_OVERVIEW.connections].sort(
      (a, b) =>
        compareWalletsTableConnections(a, b, "taxFree", taxFreeWalletIds) *
        -1,
    );

    expect(sorted[0]?.id).toBe("c1");
    expect(taxFreeWalletIds.has(sorted[0]!.id)).toBe(true);
  });

  it("hides the tax-free column when the tax-free card is absent", () => {
    const html = renderTable();

    expect(html).not.toContain("Tax-free");
    expect(html).not.toContain("Yes");
  });
});
