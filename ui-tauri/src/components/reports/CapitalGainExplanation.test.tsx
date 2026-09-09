import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CapitalGainExplanation, ExplanationBody, type ResultReference } from "./CapitalGainExplanation";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@/daemon/client", () => ({ useDaemon: invoke }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
const reference: ResultReference = { database_id: "book", workspace_id: "w", profile_id: "p", input_version: 3, processed_at: "now", entry_id: "entry" };
const source = { transaction_id: "source", engine_event_id: "engine-source", occurred_at: "2024", asset: "BTC", spot_price_exact: "100.00000000001", fiat_fee_exact: "0.1", crypto_fee_msat: 100, pricing: { pricing_provider: "manual" } };
beforeEach(() => {
  invoke.mockClear();
  invoke.mockReturnValue({ data: { data: { reference, status: "available", currency: "EUR", quarantines: 0, totals: { proceeds_exact: "20.01", cost_basis_exact: "10.00", gain_loss_exact: "10.01" }, calculation: { method: "fifo", fragments: [{ quantity_msat: 1000, proceeds_exact: "20.01", cost_basis_exact: "10.00", gain_loss_exact: "10.01", unit_basis_override_exact: null, acquisition_basis_exact: "100", acquisition_quantity_msat: 10000, event: source, lot: source }] }, custody_decisions: [], custody_truncated: false } } });
});
describe("capital-gains explanation", () => {
  it("does not request evidence while the dialog is closed", () => {
    renderToStaticMarkup(<CapitalGainExplanation reference={reference} hideSensitive={false} />);
    expect(invoke).not.toHaveBeenCalled();
  });
  it("renders exact strings and pins calculation and source links", () => {
    const html = renderToStaticMarkup(<ExplanationBody reference={reference} hideSensitive={false} />);
    expect(invoke).toHaveBeenCalledWith("ui.reports.explain_capital_gain", { reference }, { staleTime: 0, retry: false });
    expect(html).toContain("20.01 − 10.00 = 10.01 EUR");
    expect(html).toContain(encodeURIComponent(JSON.stringify(reference)));
    expect(html).toContain("#event-0");
    expect(html).toContain("100.00000000001");
  });
  it("does not render calculation when the endpoint reports a stale scope", () => {
    invoke.mockReturnValue({ error: new Error("This result belongs to another book") });
    const html = renderToStaticMarkup(<ExplanationBody reference={reference} hideSensitive={false} />);
    expect(html).toContain("another book");
    expect(html).not.toContain("20.01");
  });
  it("keeps exact financial content sensitive", () => {
    const html = renderToStaticMarkup(<ExplanationBody reference={reference} hideSensitive />);
    expect(html).toContain("sensitive");
  });
});
