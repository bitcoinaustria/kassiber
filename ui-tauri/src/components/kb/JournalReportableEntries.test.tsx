import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ReviewTableRow } from "@/components/kb/ReviewDataTable";

let capturedRows: ReviewTableRow[] = [];

vi.mock("@/components/kb/ReviewDataTable", () => ({
  ReviewDataTable: ({ rows }: { rows: ReviewTableRow[] }) => {
    capturedRows = rows;
    return <div>Processed journal entries</div>;
  },
}));

vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({
    data: {
      data: {
        summary: {
          workspace: "books",
          profile: "bitcoin",
          count: 1,
          reportableCount: 1,
          needsJournals: false,
          lastProcessedAt: "2026-08-31T12:01:00Z",
          freshnessStatus: "current",
          freshnessReason: "Journals are current",
          entryTypes: [{ type: "acquisition", count: 1, gainLossEur: 0 }],
          limit: 500,
        },
        events: [
          {
            id: "journal-1",
            transactionId: "transaction-1",
            transactionExternalId: "tx-1",
            transactionDirection: "inbound",
            occurredAt: "2026-08-31T12:00:00Z",
            createdAt: "2026-08-31T12:00:00Z",
            entryType: "acquisition",
            wallet: "Compensation wallet",
            account: "assets",
            accountLabel: "Bitcoin",
            asset: "BTC",
            quantity: 0.01,
            quantityMsat: 1_000_000_000,
            fiatValueEur: 600,
            unitCostEur: 60_000,
            costBasisEur: 600,
            proceedsEur: null,
            gainLossEur: null,
            pricingSourceKind: "source_price",
            pricingQuality: "exact",
            description: "",
            atCategory: null,
            atKennzahl: null,
          },
        ],
      },
    },
    isLoading: false,
    isError: false,
    error: null,
  }),
}));

vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: () => ({
    runJournalProcessing: vi.fn(),
    isProcessingJournals: false,
  }),
}));

import { JournalReportableEntries } from "./JournalReportableEntries";

describe("JournalReportableEntries", () => {
  beforeEach(() => {
    capturedRows = [];
  });

  it("renders compensation as an ordinary acquisition with its basis", () => {
    renderToStaticMarkup(<JournalReportableEntries />);

    expect(capturedRows).toHaveLength(1);
    expect(capturedRows[0]).toMatchObject({
      event: "Acquisition",
      basis: "Basis €\u00a0600,00",
      evidenceHint: "Priced by Source price",
      nextAction: "Ready for reports",
      metricFilterIds: ["acquisitions"],
    });
    expect(JSON.stringify(capturedRows[0])).not.toMatch(/employment|payroll|wage/i);
  });
});
