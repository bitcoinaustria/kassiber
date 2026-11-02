import { renderToStaticMarkup } from "react-dom/server";
import { afterAll, beforeEach, describe, expect, it } from "vitest";

import i18n from "@/i18n";

import { rowsThatFit, type Transaction } from "./model";
import { RecentTransactionsTable } from "./RecentTransactionsTable";

const row = (tag: string) => ({
  id: "tx-1",
  txid: "external-1",
  counterparty: "Exchange",
  counterpartyInitials: "EX",
  tags: [tag],
  status: "confirmed" as const,
  flow: "incoming" as const,
  amount: 25_000,
  amountBtc: 0.5,
  fiatCurrency: "EUR",
  date: "2026-03-01 10:00",
});

const chipFor = (tag: string) =>
  renderToStaticMarkup(
    <RecentTransactionsTable
      transactions={[row(tag)]}
      hideSensitive={false}
      currency="btc"
      priceEur={60_000}
      fiatCurrency="EUR"
      showAllTo={null}
      // renders rows as buttons, so the test needs no router context
      onOpenTransaction={() => {}}
    />,
  );

afterAll(async () => {
  await i18n.changeLanguage("en");
});

describe("RecentTransactionsTable type chip", () => {
  it("renders the derived type, not a direction-derived guess", () => {
    expect(chipFor("Buy")).toContain(">Buy<");
    expect(chipFor("Acquired")).toContain(">Acquired<");
  });

  it("translates the derived type and leaves user tags alone", async () => {
    await i18n.changeLanguage("de");
    const html = chipFor("Buy");
    // The key itself leaking through would mean a mis-wired `t`.
    expect(html).not.toContain("type.buy");
    expect(html).toContain(">Kauf<");
    expect(chipFor("Revenue")).toContain(">Revenue<");
  });
});

describe("RecentTransactionsTable rows", () => {
  beforeEach(async () => {
    await i18n.changeLanguage("en");
  });

  const render = (rows: Transaction[], limit?: number) =>
    renderToStaticMarkup(
      <RecentTransactionsTable
        transactions={rows}
        hideSensitive={false}
        currency="btc"
        priceEur={60_000}
        fiatCurrency="EUR"
        showAllTo={null}
        onOpenTransaction={() => {}}
        limit={limit}
      />,
    );

  it("names a status only when it asks for something, and never twice", () => {
    expect(render([row("Buy")])).not.toContain(">Confirmed<");
    expect(render([{ ...row("Buy"), status: "pending" }])).toContain(">Pending<");
    // A row tagged Review keeps its Review chip and names its direction.
    const tagged = render([{ ...row("Review"), status: "review" }]);
    expect(tagged.match(/>Review</g)).toHaveLength(1);
    expect(tagged).toMatch(/rounded-md px-1 font-medium[^"]*">Review</);
  });

  it("fits whole rows only", () => {
    // Five 40px rows with four 1px dividers take 204px.
    expect(rowsThatFit(204, 41)).toBe(5);
    expect(rowsThatFit(203.9, 41)).toBe(4);
    expect(rowsThatFit(20, 41)).toBe(1);
    expect(rowsThatFit(0, 41)).toBeNull();
    expect(rowsThatFit(204, 0)).toBeNull();
  });

  it("lists at most the limit, with no scrolling inside the card", () => {
    const rows = Array.from({ length: 5 }, (_, index) => ({ ...row("Buy"), id: `tx-${index}` }));
    const html = render(rows, 3);
    expect(html.match(/data-recent-row/g)).toHaveLength(3);
    expect(html).not.toContain("overflow-auto");
    expect(render(rows).match(/data-recent-row/g)).toHaveLength(5);
  });
});

