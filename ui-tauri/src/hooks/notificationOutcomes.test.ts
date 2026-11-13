import { QueryClient } from "@tanstack/react-query";
import { beforeEach, describe, expect, it } from "vitest";

import { daemonQueryKey } from "@/daemon/client";

import {
  recordJournalFailure,
  recordJournalSuccess,
} from "@/hooks/useJournalProcessingAction";
import {
  recordBookRefreshFailure,
  recordBookRefreshOutcome,
} from "@/hooks/useWalletSyncAction";
import { bookKeyFor } from "@/lib/activeBook";
import { clearFeedback, useFeedbackStore } from "@/lib/feedback";
import { resolveJournalFailureAfter } from "@/lib/journalOutcome";
import type { FreshnessRunData, FreshnessSourceState } from "@/lib/syncResults";
import {
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";
import { bookIdentityKey, useUiStore } from "@/store/ui";

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

describe("journal failures resolved by a later journal step", () => {
  const failedKey = conditionKey("journals.process-failed", { book: BOOK });

  it("clears after Refresh book processes journals", () => {
    recordJournalFailure(new Error("writer busy"), { book: BOOK }, true);
    recordBookRefreshOutcome(
      {
        completed: [{ id: "j", job_type: "journal_refresh", status: "done" }],
        journals: { status: "processed" },
        sources: [],
      },
      { book: BOOK },
      false,
    );
    expect(useNotificationStore.getState().conditions[failedKey]).toBeUndefined();
  });

  it("stays when the refresh's journal step was deferred or failed", () => {
    recordJournalFailure(new Error("writer busy"), { book: BOOK }, true);
    for (const status of ["deferred", "failed", "disabled"]) {
      recordBookRefreshOutcome(
        { completed: [], journals: { status }, sources: [] },
        { book: BOOK },
        false,
      );
    }
    expect(useNotificationStore.getState().conditions[failedKey]).toBeDefined();
  });

  it("clears after another book's success only in that book", () => {
    recordJournalFailure(new Error("writer busy"), { book: BOOK }, true);
    recordBookRefreshOutcome(
      { completed: [], journals: { status: "processed" }, sources: [] },
      { book: OTHER },
      false,
    );
    expect(useNotificationStore.getState().conditions[failedKey]).toBeDefined();
  });

  it("clears after any mutation reports a processed journal step in the same session", () => {
    const client = new QueryClient();
    const { dataMode, daemonSession } = useUiStore.getState();
    client.setQueryData(daemonQueryKey(dataMode, daemonSession, "ui.profiles.snapshot"), {
      kind: "ui.profiles.snapshot",
      schema_version: 1,
      data: { activeWorkspaceId: "ws", activeProfileId: "pf" },
    });
    const book = bookKeyFor(bookIdentityKey(useUiStore.getState().identity), {
      workspace_id: "ws",
      profile_id: "pf",
    });
    const key = conditionKey("journals.process-failed", { book });
    recordJournalFailure(new Error("writer busy"), { book }, true);

    // Started in an earlier session: the book may have changed since.
    resolveJournalFailureAfter(client, daemonSession - 1, {
      journals: { status: "processed" },
    });
    expect(useNotificationStore.getState().conditions[key]).toBeDefined();

    resolveJournalFailureAfter(client, daemonSession, { journals: { status: "deferred" } });
    expect(useNotificationStore.getState().conditions[key]).toBeDefined();

    resolveJournalFailureAfter(client, daemonSession, { journals: { status: "processed" } });
    expect(useNotificationStore.getState().conditions[key]).toBeUndefined();
  });
});
