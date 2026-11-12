import { beforeEach, describe, expect, it } from "vitest";

import {
  recordJournalFailure,
  recordJournalSuccess,
} from "@/hooks/useJournalProcessingAction";
import {
  recordBookRefreshFailure,
  recordBookRefreshOutcome,
} from "@/hooks/useWalletSyncAction";
import { clearFeedback, useFeedbackStore } from "@/lib/feedback";
import type { FreshnessRunData, FreshnessSourceState } from "@/lib/syncResults";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";

// How finished book refreshes and journal runs land in the center: what they
// raise, what they resolve, and which options a Retry keeps.

const BOOK = "db:/books/a.sqlite3";
const OTHER = "db:/books/b.sqlite3";

function source(key: string, status: string): FreshnessSourceState {
  return {
    source_key: key,
    source_type: "onchain_wallet",
    source_label: key.toUpperCase(),
    status,
  };
}

function failedJob(key: string) {
  return {
    id: `job-${key}`,
    job_type: "wallet_sync",
    source_key: key,
    source_label: key.toUpperCase(),
    status: "error",
    error: { code: "backend_error", message: "timeout" },
  };
}

function sourceCondition(key: string, book = BOOK) {
  return useNotificationStore.getState().conditions[
    conditionKey("source.refresh-failed", { book, source: key })
  ];
}

beforeEach(() => {
  resetNotificationStore();
  clearFeedback();
});

describe("book refresh outcomes", () => {
  it("keeps a failed run's options for Retry and resolves it on the next run", () => {
    recordBookRefreshFailure(new Error("daemon went away"), { book: BOOK }, true);
    const key = conditionKey("book-refresh.failed", { book: BOOK });
    expect(useNotificationStore.getState().conditions[key]).toMatchObject({
      params: { forceFull: true },
      action: {
        type: "operation",
        operation: { kind: "refresh-book", forceFull: true },
        review: "/connections",
      },
      details: [{ message: "daemon went away" }],
    });
    expect(useFeedbackStore.getState().items).toHaveLength(1);

    recordBookRefreshOutcome({ completed: [], sources: [] }, { book: BOOK }, false);
    expect(useNotificationStore.getState().conditions[key]).toBeUndefined();
    expect(useNotificationStore.getState().results[0]).toMatchObject({
      code: "book-refresh.finished",
      scope: { book: BOOK },
    });
    // One toast for the latest outcome, not one per run.
    expect(useFeedbackStore.getState().items).toHaveLength(1);
  });

  it("reports a late completion against the book the run started in", () => {
    const data: FreshnessRunData = {
      completed: [failedJob("a")],
      sources: [source("a", "failed")],
    };
    recordBookRefreshOutcome(data, { book: BOOK }, false);
    expect(sourceCondition("a", BOOK)).toBeDefined();
    expect(sourceCondition("a", OTHER)).toBeUndefined();
    expect(useNotificationStore.getState().results[0]).toMatchObject({
      code: "book-refresh.problems",
      scope: { book: BOOK },
    });
  });

  it("raises backend-required and resolves it once a run no longer needs one", () => {
    recordBookRefreshOutcome(
      { completed: [], results: [{ wallet: "w", status: "error", code: "backend_not_configured" }] },
      { book: BOOK },
      false,
    );
    const key = conditionKey("book-refresh.backend-required", { book: BOOK });
    expect(useNotificationStore.getState().conditions[key]).toMatchObject({
      action: { type: "navigate", to: "/settings/bitcoin" },
    });
    recordBookRefreshOutcome({ completed: [], sources: [] }, { book: BOOK }, false);
    expect(useNotificationStore.getState().conditions[key]).toBeUndefined();
  });
});

describe("journal processing outcomes", () => {
  it("retries with the original auto-pair option and resolves on success", () => {
    recordJournalFailure(new Error("writer busy"), { book: BOOK }, false);
    const key = conditionKey("journals.process-failed", { book: BOOK });
    expect(useNotificationStore.getState().conditions[key]?.action).toEqual({
      type: "operation",
      operation: { kind: "process-journals", autoPair: false },
      review: "/journals",
    });

    recordJournalSuccess(
      { processed_transactions: 3, entries_created: 5, quarantined: 0 },
      { book: BOOK },
    );
    expect(useNotificationStore.getState().conditions[key]).toBeUndefined();
    expect(useNotificationStore.getState().results[0]).toMatchObject({
      code: "journals.processed",
      severity: "info",
      params: { transactions: 3, entries: 5 },
    });
    expect(useFeedbackStore.getState().items.at(-1)).toMatchObject({
      tone: "success",
      title: "Journals processed",
      description: "3 transactions, 5 entries",
    });
  });

  it("keeps a result with warnings as a warning that opens Journals", () => {
    recordJournalSuccess(
      {
        processed_transactions: 1,
        entries_created: 1,
        warnings: [{ code: "duplicate_wallet_label", message: "2 wallets share a label" }],
      },
      { book: BOOK },
    );
    expect(useNotificationStore.getState().results[0]).toMatchObject({
      severity: "warning",
      action: { type: "navigate", to: "/journals" },
      details: [{ message: "2 wallets share a label" }],
    });
  });
});
