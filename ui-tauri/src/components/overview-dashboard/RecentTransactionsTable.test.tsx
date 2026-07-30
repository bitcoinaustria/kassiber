import { renderToStaticMarkup } from "react-dom/server";
import { afterAll, describe, expect, it } from "vitest";

import i18n from "@/i18n";

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
