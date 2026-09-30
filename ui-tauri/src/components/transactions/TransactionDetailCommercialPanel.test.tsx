import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";

import { CommercialProvenancePanel } from "./TransactionDetailCommercialPanel";
import type { CommercialContextData } from "./TransactionDetailSheetParts";

const emptyContext: CommercialContextData = {
  transaction_id: "tx-1",
  transaction_external_id: "external-1",
  links: [],
  btcpay: [],
  documents: [],
};

const linkedContext: CommercialContextData = {
  ...emptyContext,
  btcpay: [
    {
      link: {
        id: "link-1",
        invoice_id: "invoice-1",
        payment_id: "payment-1",
        document_id: "",
        document_label: "",
        link_type: "btcpay_payment_transaction",
        state: "reviewed",
        confidence: "high",
        reconciliation_state: "matched",
        commercial_kind: "income",
        reviewed_at: "2026-01-01T00:00:00Z",
      },
      payment: {
        id: "payment-record-1",
        record_type: "payment",
        invoice_id: "invoice-1",
        payment_id: "payment-1",
        order_id: "",
        status: "settled",
        occurred_at: "2026-01-01T00:00:00Z",
        asset: "BTC",
        amount_msat: 100_000,
        amount: 0.000001,
        payment_request_id: "",
        origin_kind: "",
        origin_app_id: "",
        origin_label: "",
        origin_url: "",
        fiat_currency: "EUR",
        fiat_value_exact: "10.00",
        fiat_rate_exact: "50000.00",
        pricing_timestamp: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
      },
      invoice: null,
      payment_request: null,
      origin: null,
    },
  ],
};

describe("CommercialProvenancePanel", () => {
  it("hides itself when no BTCPay context is linked", () => {
    const html = renderToStaticMarkup(
      <CommercialProvenancePanel context={emptyContext} />,
    );

    expect(html).toBe("");
  });

  it("renders when a BTCPay commercial match exists", () => {
    const html = renderToStaticMarkup(
      <CommercialProvenancePanel context={linkedContext} />,
    );

    expect(html).toContain("Commercial provenance");
    expect(html).toContain("payment-1");
  });

  it("offers an explicit review for a suggested refund payout", () => {
    const payoutContext: CommercialContextData = {
      ...linkedContext,
      btcpay: [
        {
          ...linkedContext.btcpay[0],
          link: { ...linkedContext.btcpay[0].link, state: "suggested", commercial_kind: "" },
          payment: {
            ...linkedContext.btcpay[0].payment!,
            record_type: "payout",
            origin_kind: "refund",
            origin_label: "Refund invoice-1",
          },
          origin: { kind: "refund", app_id: "", label: "Refund invoice-1" },
        },
      ],
    };
    const html = renderToStaticMarkup(
      <CommercialProvenancePanel context={payoutContext} onReview={() => {}} />,
    );

    expect(html).toContain("BTCPay payout");
    expect(html).toContain("Refunded invoice");
    expect(html).toContain('<option value="refund" selected="">Refund</option>');
    expect(html).not.toContain('value="income"');
    expect(html).toContain("Confirm match");
    expect(html).toContain("Not related");
    expect(html).toContain("Uses the BTCPay value in EUR as this transaction&#x27;s price.");
  });

  it("reviews the payouts of one send together at their combined value", () => {
    const payout = (id: string, value: string) => ({
      ...linkedContext.btcpay[0],
      link: { ...linkedContext.btcpay[0].link, id, state: "suggested", commercial_kind: "" },
      payment: {
        ...linkedContext.btcpay[0].payment!,
        record_type: "payout",
        origin_kind: "store_payout",
        payment_id: id,
        fiat_value_exact: value,
      },
      origin: { kind: "store_payout", app_id: "", label: "" },
    });
    const batchContext: CommercialContextData = {
      ...linkedContext,
      btcpay: [payout("po-1", "5.00"), payout("po-2", "1.80")],
      payout_batch: {
        size: 2,
        link_ids: ["po-1", "po-2"],
        amount_msat: 13_611_000,
        fiat_currency: "EUR",
        fiat_value_exact: "6.80",
        mixed_currencies: false,
      },
    };
    const html = renderToStaticMarkup(
      <CommercialProvenancePanel context={batchContext} onReview={() => {}} />,
    );

    expect(html).toContain("Confirms all 2 payouts in this transaction together");
    expect(html).toContain("Uses the BTCPay value in EUR as this transaction&#x27;s price.");
  });

  it("says when BTCPay data may be out of date", () => {
    const freshness = {
      backend: "shop",
      store_id: "S1",
      last_attempt_at: "2026-01-01T00:00:00Z",
      last_success_at: "2026-01-01T00:00:00Z",
      last_error_code: null,
      age_seconds: 200_000,
      stale: true,
      never_synced: false,
    };
    const stale = renderToStaticMarkup(
      <CommercialProvenancePanel context={{ ...linkedContext, btcpay_freshness: [freshness] }} />,
    );
    expect(stale).toContain("may be out of date");
    expect(stale).toContain("BTCPay does not send updates to Kassiber");

    const fresh = renderToStaticMarkup(
      <CommercialProvenancePanel
        context={{ ...linkedContext, btcpay_freshness: [{ ...freshness, stale: false, age_seconds: 60 }] }}
      />,
    );
    expect(fresh).toContain("BTCPay data as of");
    expect(fresh).not.toContain("may be out of date");
  });

  it("lets a reviewed match be reopened", () => {
    const html = renderToStaticMarkup(
      <CommercialProvenancePanel context={linkedContext} onReview={() => {}} />,
    );

    expect(html).toContain("Reopen review");
    expect(html).not.toContain("Confirm match");
  });
});
