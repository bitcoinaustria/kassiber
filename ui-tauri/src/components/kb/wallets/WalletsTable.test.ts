import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { WalletsTable } from "./WalletsTable";

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

  it("hides the tax-free column when the tax-free card is absent", () => {
    const html = renderTable();

    expect(html).not.toContain("Tax-free");
    expect(html).not.toContain("Yes");
  });
});
