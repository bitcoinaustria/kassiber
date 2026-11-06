import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { Transaction } from "@/components/overview-dashboard/model";

import { WalletTransactionsTable } from "./WalletTransactionsTable";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

function tx(index: number, overrides: Partial<Transaction> = {}): Transaction {
  return {
    id: `tx-${index}`,
    txid: `txid-${index}`,
    counterparty: `Row ${index}`,
    counterpartyInitials: "R",
    tags: [],
    status: "confirmed",
    flow: "incoming",
    amount: 1_000,
    amountBtc: 0.01,
    fiatCurrency: "EUR",
    date: "2025-01-01 10:00",
    ...overrides,
  };
}

function render(
  transactions: Transaction[],
  props: Partial<ComponentProps<typeof WalletTransactionsTable>> = {},
) {
  return renderToStaticMarkup(
    <WalletTransactionsTable
      walletLabel="Vault"
      transactions={transactions}
      totalCount={transactions.length}
      hideSensitive={false}
      priceEur={100_000}
      fiatCurrency="EUR"
      onOpenTransaction={() => {}}
      {...props}
    />,
  );
}

describe("WalletTransactionsTable", () => {
  it("shows the first page and offers the rest", () => {
    const rows = Array.from({ length: 30 }, (_, index) => tx(index));
    const html = render(rows, { totalCount: 240 });

    expect(html).toContain("Row 24");
    expect(html).not.toContain("Row 25");
    expect(html).toContain("Showing 25 of 240");
    expect(html).toContain("Show 5 more");
    expect(html).toContain('href="/transactions"');
  });

  it("signs amounts by direction and leaves zero unsigned", () => {
    const html = render([
      tx(1, { flow: "incoming", amountBtc: 0.5 }),
      tx(2, { flow: "outgoing", amountBtc: -0.25 }),
      tx(3, { flow: "transfer", amountBtc: 0, amount: 0 }),
    ]);

    expect(html).toContain("+ ₿ 0.50000000");
    expect(html).toContain("− ₿ 0.25000000");
    expect(html).toContain(">₿ 0.00000000<");
  });

  it("marks a row without a booked value as unpriced", () => {
    const html = render([tx(1, { amount: null })]);

    expect(html).toContain("Unpriced");
  });

  it("masks descriptions and amounts while values are hidden", () => {
    const html = render([tx(1)], { hideSensitive: true });

    expect(html).toMatch(/class="[^"]*\bsensitive\b[^"]*">Row 1</);
    expect(html).toMatch(/class="[^"]*\bsensitive\b[^"]*">\+ ₿ 0\.01000000</);
  });
});
