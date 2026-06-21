import { describe, expect, it } from "vitest";

import {
  describeFreshnessSourceState,
  freshnessRunAutoPairCount,
  freshnessRunQuarantineCount,
  freshnessRunTransferReviewCount,
  describeWalletSyncResult,
  freshnessRunNeedsAttention,
  summarizeFreshnessRun,
  summarizeSyncResults,
  syncResultsAreTrustedForReports,
} from "./syncResults";

describe("syncResults", () => {
  it("keeps the wallet-specific error message in all-sync summaries", () => {
    expect(
      summarizeSyncResults([
        { wallet: "Cold", status: "synced" },
        {
          wallet: "Descriptor",
          status: "error",
          message: "Descriptor-backed refresh requires embit.",
          hint: "Use a desktop build that bundles embit.",
        },
      ]),
    ).toBe(
      "1 refreshed, 1 failed: Descriptor: Descriptor-backed refresh requires embit. Use a desktop build that bundles embit.",
    );
  });

  it("uses source wording when no daemon results are returned", () => {
    expect(summarizeSyncResults([])).toBe("No source changes returned.");
  });

  it("keeps the wallet-specific error message on detail sync", () => {
    expect(
      describeWalletSyncResult(
        {
          wallet: "Descriptor",
          status: "error",
          message: "Failed to reach backend local-esplora: timed out",
        },
        "Descriptor",
      ),
    ).toBe("Descriptor refresh failed: Failed to reach backend local-esplora: timed out");
  });

  it("summarizes successful refresh observability", () => {
    expect(
      describeWalletSyncResult(
        {
          wallet: "Descriptor",
          status: "synced",
          imported: 0,
          updated: 1,
          unchanged: 12,
          scripts_checked: 24,
          utxos_skipped_unchanged: true,
          journal_invalidated: true,
          elapsed_ms: 42,
        },
        "Descriptor",
      ),
    ).toBe(
      "Descriptor refreshed: 1 updated, 12 unchanged · 24 scripts checked · UTXOs unchanged · journals marked stale · 42 ms.",
    );
  });

  it("marks force-full rescans in refresh summaries", () => {
    expect(
      describeWalletSyncResult(
        {
          wallet: "Cold",
          status: "synced",
          force_full: true,
          records_fetched: 2,
          target_count: 3,
          utxos_refreshed: true,
          journal_invalidated: false,
        },
        "Cold",
      ),
    ).toBe(
      "Cold refreshed: full rescan · 2 source rows · 3 targets · UTXOs refreshed · journals unchanged.",
    );
  });

  it("only trusts all-wallet refresh results without errors for report refresh chaining", () => {
    expect(
      syncResultsAreTrustedForReports([
        { wallet: "Cold", status: "synced" },
        { wallet: "File", status: "skipped", reason: "No local file configured" },
      ]),
    ).toBe(true);
    expect(
      syncResultsAreTrustedForReports([
        { wallet: "Cold", status: "synced" },
        { wallet: "Descriptor", status: "error", message: "Timed out" },
      ]),
    ).toBe(false);
    expect(
      syncResultsAreTrustedForReports([
        { wallet: "Cold", status: "synced" },
        { wallet: "Rates", status: "blocking_reports" },
      ]),
    ).toBe(false);
  });

  it("describes normal freshness cooldowns without panic wording", () => {
    expect(
      describeFreshnessSourceState({
        source_key: "market_rates:profile",
        source_type: "market_rates",
        source_label: "Market-rate coverage",
        status: "rate_limited",
        rate_limited_until: "2026-06-04T12:30:00Z",
      }),
    ).toBe("Market-rate coverage is rate limited until 2026-06-04T12:30:00Z.");
  });

  it("summarizes combined book refresh jobs without treating cooldown as failure", () => {
    const payload = {
      completed: [
        { job_type: "onchain_wallet_history", source_label: "Cold", status: "done" },
        { job_type: "market_rate_coverage", source_label: "Market-rate coverage", status: "done" },
        { job_type: "journal_refresh", source_label: "Journal refresh", status: "done" },
        {
          job_type: "btcpay_provenance",
          source_label: "BTCPay provenance",
          status: "rate_limited",
          error: { message: "Retry after 90 seconds." },
        },
      ],
      summary: { rate_limited: 1, failed: 0, blocking_reports: 0 },
    };

    expect(summarizeFreshnessRun(payload)).toBe(
      "3 completed, 1 cooling down: BTCPay provenance: Retry after 90 seconds.",
    );
    expect(freshnessRunNeedsAttention(payload)).toBe(false);
  });

  it("surfaces journal quarantines in combined book refresh summaries", () => {
    const payload = {
      completed: [
        { job_type: "onchain_wallet_history", source_label: "Cold", status: "done" },
        {
          job_type: "journal_refresh",
          source_label: "Journal refresh",
          status: "done",
          result: { quarantined: 2 },
        },
      ],
    };

    expect(freshnessRunQuarantineCount(payload)).toBe(2);
    expect(summarizeFreshnessRun(payload)).toBe(
      "2 completed, 2 quarantined transactions",
    );
    expect(freshnessRunNeedsAttention(payload)).toBe(false);
  });

  it("summarizes auto-paired and still-reviewable transfer candidates", () => {
    const payload = {
      completed: [
        {
          job_type: "journal_refresh",
          source_label: "Journal refresh",
          status: "done",
          result: {
            auto_pair: {
              applied: 3,
              remaining: { total: 2, exact: 0, strong: 2, conflicts: 1 },
            },
          },
        },
      ],
    };

    expect(freshnessRunAutoPairCount(payload)).toBe(3);
    expect(freshnessRunTransferReviewCount(payload)).toBe(2);
    expect(summarizeFreshnessRun(payload)).toBe(
      "1 completed, 3 pairs applied, 2 swap/transfer candidates to review",
    );
    expect(freshnessRunNeedsAttention(payload)).toBe(false);
  });

  it("marks combined refresh attention only for blocking or failed sources", () => {
    const payload = {
      completed: [
        {
          job_type: "market_rate_coverage",
          source_label: "Market-rate coverage",
          status: "error",
          error: { message: "HTTP 500" },
        },
      ],
      sources: [
        {
          source_key: "market_rates:profile",
          source_type: "market_rates",
          source_label: "Market-rate coverage",
          status: "blocking_reports",
          blocking_reports: true,
        },
      ],
      summary: { failed: 1, blocking_reports: 1 },
    };

    expect(summarizeFreshnessRun(payload)).toBe(
      "1 needs attention: Market-rate coverage: HTTP 500",
    );
    expect(freshnessRunNeedsAttention(payload)).toBe(true);
  });
});
