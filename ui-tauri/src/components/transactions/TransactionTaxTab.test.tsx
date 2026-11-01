import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import "@/i18n";

import { TooltipProvider } from "@/components/ui/tooltip";

import type { JournalEventItem } from "./TransactionDetailSheetParts";
import { TaxCalculationRows } from "./TransactionTaxTab";

const reference = (entryId: string) => ({
  database_id: "db",
  workspace_id: "ws",
  profile_id: "pf",
  input_version: 3,
  processed_at: "2026-01-01T00:00:00Z",
  entry_id: entryId,
});

const event = (
  id: string,
  overrides: Partial<JournalEventItem> = {},
): JournalEventItem => ({
  id,
  entryType: "disposal",
  asset: "BTC",
  quantity: -0.1,
  fiatValueEur: 4_000,
  ...overrides,
});

const render = (events: JournalEventItem[]) =>
  renderToStaticMarkup(
    <TooltipProvider>
      <TaxCalculationRows events={events} hideSensitive={false} />
    </TooltipProvider>,
  );

describe("TaxCalculationRows", () => {
  it("opens the calculation behind each realized gain", () => {
    const html = render([
      event("je-sale", { explanationReference: reference("je-sale") }),
      // An acquisition realizes nothing and has no calculation to open.
      event("je-buy", { entryType: "acquisition", explanationReference: null }),
    ]);

    expect(html.match(/>Explain</g)).toHaveLength(1);
    expect(html).toContain(">Calculation<");
  });

  it("numbers the calculations when a transaction books several", () => {
    const html = render([
      event("je-1", { explanationReference: reference("je-1") }),
      event("je-2", { entryType: "income", explanationReference: reference("je-2") }),
    ]);

    expect(html).toContain(">Calculation 1<");
    expect(html).toContain(">Calculation 2<");
    // Each trigger names its row, not just "Explain".
    expect(html).toContain('aria-label="Explain Calculation 2"');
  });

  it("adds nothing without a realized gain", () => {
    expect(render([event("je-buy", { entryType: "acquisition" })])).toBe("");
  });
});
