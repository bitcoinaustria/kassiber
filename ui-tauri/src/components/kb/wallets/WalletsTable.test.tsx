import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Connection } from "@/mocks/seed";

import { WalletsTable } from "./WalletsTable";

function connection(overrides: Partial<Connection>): Connection {
  return {
    id: "wallet-1",
    kind: "descriptor",
    label: "Treasury",
    last: "just now",
    lastSyncAt: "2026-07-06T11:59:00Z",
    lastTransactionAt: "2026-07-04T12:00:00Z",
    balance: 0.5,
    status: "synced",
    transactionCount: 3,
    ...overrides,
  };
}

describe("WalletsTable", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows transaction activity instead of sync recency", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-07-06T12:00:00Z"));

    const html = renderToStaticMarkup(
      <WalletsTable
        connections={[
          connection({ id: "wallet-active", label: "Active" }),
          connection({
            id: "wallet-empty",
            label: "Empty",
            last: "just now",
            lastSyncAt: "2026-07-06T11:59:00Z",
            lastTransactionAt: null,
            transactionCount: 0,
          }),
        ]}
        currency="btc"
        hideSensitive={false}
        onSelectConnection={vi.fn()}
        priceEur={60_000}
        totalBtc={0.5}
        totalCount={2}
      />,
    );

    expect(html).toContain("Last activity");
    expect(html).not.toContain("Last sync");
    expect(html).toContain("2d ago");
    expect(html).toContain("never");
    expect(html).not.toContain("just now");
  });

  it("does not show impossible percentages for overlapping wallet inventories", () => {
    const html = renderToStaticMarkup(
      <WalletsTable
        balanceSharesOverlap
        connections={[
          connection({ id: "wallet-a", balance: 0.5 }),
          connection({ id: "wallet-b", balance: 0.5 }),
        ]}
        currency="btc"
        hideSensitive={false}
        onSelectConnection={vi.fn()}
        priceEur={60_000}
        totalBtc={0.5}
        totalCount={2}
      />,
    );

    expect(html).toContain("Shared outpoint · wallet shares overlap");
    expect(html).not.toContain("100% of total balance");
  });
});
