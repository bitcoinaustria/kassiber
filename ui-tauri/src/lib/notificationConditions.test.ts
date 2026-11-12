import { beforeEach, describe, expect, it } from "vitest";

import { parseDaemonFreshnessEvent } from "@/lib/daemonFreshnessEvent";
import {
  applyFreshnessRun,
  reviewConditionsFromBadges,
  sourceConditionsFromSnapshot,
} from "@/lib/notificationConditions";
import type { FreshnessSourceState } from "@/lib/syncResults";
import {
  attentionGroupCount,
  conditionKey,
  resetNotificationStore,
  useNotificationStore,
} from "@/store/notifications";

const BOOK = "db:/books/a.sqlite3";
const OTHER = "db:/books/b.sqlite3";

function source(
  key: string,
  status: string,
  extra: Partial<FreshnessSourceState> = {},
): FreshnessSourceState {
  return {
    source_key: key,
    source_type: "onchain_wallet",
    source_label: key.toUpperCase(),
    status,
    ...extra,
  };
}

function failedJob(key: string, message = "timeout") {
  return {
    id: `job-${key}`,
    job_type: "wallet_sync",
    source_key: key,
    source_label: key.toUpperCase(),
    status: "error",
    error: { code: "backend_error", message },
  };
}

function sourceCondition(key: string, book = BOOK) {
  return useNotificationStore.getState().conditions[
    conditionKey("source.refresh-failed", { book, source: key })
  ];
}

beforeEach(() => {
  resetNotificationStore();
});

describe("per-source freshness conditions", () => {
  it("keeps a failure for B when A recovers, and resolves B only once B is fresh", () => {
    // Run 1: A and B both fail.
    applyFreshnessRun(
      {
        completed: [failedJob("a"), failedJob("b")],
        sources: [source("a", "failed"), source("b", "failed")],
      },
      BOOK,
    );
    expect(sourceCondition("a")?.occurrenceCount).toBe(1);
    expect(sourceCondition("b")?.occurrenceCount).toBe(1);

    // Run 2: A succeeds; B is not retried and stays failed in the snapshot.
    applyFreshnessRun(
      { completed: [], sources: [source("a", "fresh"), source("b", "failed")] },
      BOOK,
    );
    expect(sourceCondition("a")).toBeUndefined();
    expect(sourceCondition("b")?.occurrenceCount).toBe(1);

    // Run 3: B is still syncing, which proves nothing either way.
    applyFreshnessRun(
      { completed: [], sources: [source("a", "fresh"), source("b", "syncing")] },
      BOOK,
    );
    expect(sourceCondition("b")).toBeDefined();

    // Run 4: B fails again: a new occurrence.
    applyFreshnessRun(
      { completed: [failedJob("b")], sources: [source("b", "failed")] },
      BOOK,
    );
    expect(sourceCondition("b")?.occurrenceCount).toBe(2);

    // Run 5: B is fresh.
    applyFreshnessRun({ completed: [], sources: [source("b", "fresh")] }, BOOK);
    expect(sourceCondition("b")).toBeUndefined();
  });

  it("resolves nothing from a run without a snapshot or a source missing from it", () => {
    applyFreshnessRun(
      { completed: [failedJob("a")], sources: [source("a", "failed")] },
      BOOK,
    );
    applyFreshnessRun({ completed: [] }, BOOK);
    applyFreshnessRun({ completed: [], sources: [] }, BOOK);
    applyFreshnessRun({ completed: [], sources: [source("z", "fresh")] }, BOOK);
    expect(sourceCondition("a")).toBeDefined();
  });

  it("does not count a repeat when polling reconciles the same snapshot", () => {
    applyFreshnessRun(
      { completed: [failedJob("a")], sources: [source("a", "failed")] },
      BOOK,
    );
    const snapshot = sourceConditionsFromSnapshot(
      [source("a", "failed", { last_error_message: "timeout" })],
      BOOK,
    );
    const before = useNotificationStore.getState().conditions;
    for (let index = 0; index < 5; index += 1) {
      useNotificationStore.getState().reconcileConditions(snapshot);
    }
    expect(sourceCondition("a")?.occurrenceCount).toBe(1);
    expect(sourceCondition("a")?.lastOccurredAt).toBe(
      before[conditionKey("source.refresh-failed", { book: BOOK, source: "a" })]
        .lastOccurredAt,
    );
  });

  it("keeps the same source label apart in different books", () => {
    applyFreshnessRun(
      { completed: [failedJob("a")], sources: [source("a", "failed")] },
      BOOK,
    );
    applyFreshnessRun({ completed: [], sources: [source("a", "fresh")] }, OTHER);
    expect(sourceCondition("a", BOOK)).toBeDefined();
    expect(attentionGroupCount(useNotificationStore.getState().conditions, OTHER)).toBe(0);
    expect(attentionGroupCount(useNotificationStore.getState().conditions, BOOK)).toBe(1);
  });

  it("marks a source that blocks reports as a blocker", () => {
    const { present } = sourceConditionsFromSnapshot(
      [source("a", "partially_stale", { blocking_reports: true })],
      BOOK,
    );
    expect(present[0]).toMatchObject({ priority: "blocking", severity: "warning" });
  });

  it("parses background events with their snapshot and worker errors", () => {
    const parsed = parseDaemonFreshnessEvent({
      event: true,
      kind: "ui.freshness.background",
      schema_version: 1,
      data: {
        completed: [failedJob("a", "HTTP 502")],
        sources: [source("a", "failed", { last_error_message: "HTTP 502" })],
      },
    });
    expect(parsed).toMatchObject({ kind: "background", needsAttention: true });
    if (parsed?.kind !== "background") throw new Error("expected background");
    expect(parsed.data.completed?.[0]).toMatchObject({
      source_key: "a",
      error: { message: "HTTP 502" },
    });
    expect(parsed.data.sources?.[0].last_error_message).toBe("HTTP 502");

    // No `sources` list is not an empty snapshot.
    const bare = parseDaemonFreshnessEvent({
      event: true,
      kind: "ui.freshness.background",
      schema_version: 1,
      data: { completed: [] },
    });
    if (bare?.kind !== "background") throw new Error("expected background");
    expect(bare.data.sources).toBeUndefined();

    expect(
      parseDaemonFreshnessEvent({
        event: true,
        kind: "ui.freshness.worker",
        schema_version: 1,
        data: { status: "error", code: "freshness_background_error", message: "failed" },
      }),
    ).toEqual({
      kind: "worker-error",
      code: "freshness_background_error",
      message: "failed",
    });
  });
});

describe("live review conditions", () => {
  it("raises and resolves journals and review from badges", () => {
    const store = useNotificationStore.getState();
    store.reconcileConditions(
      reviewConditionsFromBadges(
        { quarantine: 2, journals_needs_processing: true, swaps: 1, review: 3 },
        BOOK,
      ),
    );
    const conditions = useNotificationStore.getState().conditions;
    expect(conditions[conditionKey("journals.stale", { book: BOOK })]).toMatchObject({
      priority: "attention",
    });
    expect(conditions[conditionKey("review.pending", { book: BOOK })]).toMatchObject({
      priority: "blocking",
      params: { count: 3, blocking: true },
    });

    store.reconcileConditions(
      reviewConditionsFromBadges(
        { quarantine: 0, journals_needs_processing: false, swaps: 0, review: 0 },
        BOOK,
      ),
    );
    expect(useNotificationStore.getState().conditions).toEqual({});
  });
});
