import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";

import {
  DocumentDraftReviewTable,
  type DocumentDraft,
} from "./DocumentImportDialog";

const draft: DocumentDraft = {
  document_token: "preview-session",
  source: {
    filename: "statement.pdf",
    kind: "pdf",
    pdf: {
      total_pages: 6,
      rendered_pages: [2, 3, 4],
      complete: false,
      selection_explicit: true,
      selection: "2-4",
    },
  },
  model: "glm-ocr",
  rows: [
    {
      id: "docrow-source-001",
      status: "ready",
      confidence: 0.96,
      confidence_threshold: 0.78,
      cell_confidences: {
        occurred_at: 0.99,
        direction: 0.98,
        asset: 0.97,
        amount_btc: 0.96,
        fee_btc: 0.95,
        fiat_currency: 0.94,
        fiat_value: 0.93,
        fiat_rate: 0.92,
      },
      source_region: { page: 3 },
      evidence_text: "Bought 0.01 BTC for EUR 500 plus 0.00001 BTC fee",
      record: {
        occurred_at: "2026-01-02T00:00:00Z",
        direction: "inbound",
        asset: "BTC",
        amount_btc: "0.01",
        fee_btc: "0.00001",
        fiat_currency: "EUR",
        fiat_value: "500",
        fiat_rate: "50000",
        counterparty: "Local exchange",
        description: "Reviewed purchase",
      },
    },
  ],
  summary: { rows: 1, ready: 1, quarantined: 0 },
};

describe("DocumentDraftReviewTable", () => {
  it("shows every accounting field, its confidence, and the source page", () => {
    const html = renderToStaticMarkup(
      <DocumentDraftReviewTable draft={draft} selectedRows={new Set()} />,
    );

    expect(html).toContain("2026-01-02T00:00:00Z");
    expect(html).toContain("0.00001");
    expect(html).toContain("500 EUR");
    expect(html).toContain("50000 EUR/BTC");
    expect(html).toContain("Local exchange");
    expect(html).toContain("Reviewed purchase");
    expect(html).toContain("Source page");
    expect(html).toContain("amount_btc 96%");
    expect(html).toContain("fiat_currency 94%");
    expect(html).toContain('data-state="unchecked"');
    expect(html).not.toContain('data-state="checked"');
  });

  it("discloses when an omitted fee will be recorded as zero", () => {
    const feeDefaultedDraft: DocumentDraft = {
      ...draft,
      rows: [
        {
          ...draft.rows[0],
          record: {
            ...draft.rows[0].record,
            fee_btc: "0",
            fee_defaulted: true,
          },
        },
      ],
    };
    const html = renderToStaticMarkup(
      <DocumentDraftReviewTable
        draft={feeDefaultedDraft}
        selectedRows={new Set()}
      />,
    );

    expect(html).toContain("not provided; 0 will be recorded");
  });
});
