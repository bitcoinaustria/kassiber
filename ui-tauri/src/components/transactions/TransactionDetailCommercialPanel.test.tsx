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
});
