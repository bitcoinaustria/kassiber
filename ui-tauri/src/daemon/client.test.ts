import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";

import {
  DaemonAuthRequiredError,
  DaemonRequestError,
  daemonMutationKey,
  invalidateDaemonQueriesForMutation,
  invalidatedDaemonQueryKindsForMutation,
  isRetryableDaemonError,
  mutationAdvancesDaemonSession,
  parseDaemonAuthRequiredEventDetail,
  retryDaemonQuery,
  retryRetryableDaemonError,
  shouldHandleDaemonAuthRequiredEvent,
} from "./client";
import type { DaemonEnvelope } from "./transport";

const authEnvelope: DaemonEnvelope = {
  kind: "auth_required",
  schema_version: 1,
  request_id: "locked-1",
  data: { scope: "unlock_database" },
};

describe("daemon auth-required event detail", () => {
  it("keeps legacy envelope-only events current before the first session bump", () => {
    expect(parseDaemonAuthRequiredEventDetail(authEnvelope)).toEqual({
      envelope: authEnvelope,
    });
    expect(shouldHandleDaemonAuthRequiredEvent(authEnvelope, 0)).toBe(true);
  });

  it("ignores legacy envelope-only events after the daemon session advances", () => {
    expect(shouldHandleDaemonAuthRequiredEvent(authEnvelope, 1)).toBe(false);
  });

  it("handles events from the current daemon session", () => {
    expect(
      shouldHandleDaemonAuthRequiredEvent(
        { envelope: authEnvelope, daemonSession: 7 },
        7,
      ),
    ).toBe(true);
  });

  it("ignores auth-required events from an older daemon session", () => {
    expect(
      shouldHandleDaemonAuthRequiredEvent(
        { envelope: authEnvelope, daemonSession: 6 },
        7,
      ),
    ).toBe(false);
  });

  it("ignores non-auth envelopes on the auth-required event channel", () => {
    expect(
      shouldHandleDaemonAuthRequiredEvent(
        { kind: "ui.transactions.list", schema_version: 1 },
        7,
      ),
    ).toBe(false);
  });
});

describe("daemon query retry policy", () => {
  const busyError = new DaemonRequestError("ui.overview.snapshot", {
    kind: "error",
    schema_version: 1,
    error: { code: "daemon_busy", message: "busy", retryable: true },
  });
  const hardError = new DaemonRequestError("ui.overview.snapshot", {
    kind: "error",
    schema_version: 1,
    error: { code: "validation", message: "bad input", retryable: false },
  });
  const authError = new DaemonAuthRequiredError(authEnvelope);

  it("classifies only daemon-flagged retryable errors as retryable", () => {
    expect(isRetryableDaemonError(busyError)).toBe(true);
    expect(isRetryableDaemonError(hardError)).toBe(false);
    expect(isRetryableDaemonError(authError)).toBe(false);
    expect(isRetryableDaemonError(new Error("plain"))).toBe(false);
  });

  it("never retries an auth-required prompt", () => {
    expect(retryDaemonQuery(0, authError)).toBe(false);
    expect(retryRetryableDaemonError(0, authError)).toBe(false);
  });

  it("rides out a transient busy daemon a few extra times", () => {
    expect(retryDaemonQuery(4, busyError)).toBe(true);
    expect(retryDaemonQuery(5, busyError)).toBe(false);
    expect(retryRetryableDaemonError(4, busyError)).toBe(true);
    expect(retryRetryableDaemonError(5, busyError)).toBe(false);
  });

  it("keeps the prior 3x policy for hard errors, and opt-out panels never retry them", () => {
    expect(retryDaemonQuery(2, hardError)).toBe(true);
    expect(retryDaemonQuery(3, hardError)).toBe(false);
    expect(retryRetryableDaemonError(0, hardError)).toBe(false);
  });
});

describe("daemon mutation key", () => {
  // Sharing a stable key across `useDaemonMutation` instances lets the
  // QueryClient report cross-instance in-flight counts via `isMutating`,
  // which is how the native menu and route screens coalesce the same
  // workflow (sync, journal-process) instead of issuing duplicate jobs.
  it("partitions by data mode and kind", () => {
    expect(daemonMutationKey("real", "ui.wallets.sync")).toEqual([
      "daemon-mutation",
      "real",
      "ui.wallets.sync",
    ]);
    expect(daemonMutationKey("mock", "ui.wallets.sync")).not.toEqual(
      daemonMutationKey("real", "ui.wallets.sync"),
    );
    expect(daemonMutationKey("regtest", "ui.wallets.sync")).toEqual([
      "daemon-mutation",
      "regtest",
      "ui.wallets.sync",
    ]);
    expect(daemonMutationKey("regtest", "ui.wallets.sync")).not.toEqual(
      daemonMutationKey("real", "ui.wallets.sync"),
    );
    expect(daemonMutationKey("real", "ui.journals.process")).not.toEqual(
      daemonMutationKey("real", "ui.wallets.sync"),
    );
  });
});

describe("daemon mutation invalidation scope", () => {
  it("applies the same targeted cache policy to external mutation callers", () => {
    const queryClient = new QueryClient();
    const journalKey = ["daemon", "real", "ui.journals.snapshot", {}];
    const transactionKey = ["daemon", "real", "ui.transactions.list", {}];
    const unrelatedKey = ["daemon", "real", "ui.backends.list", {}];
    queryClient.setQueryData(journalKey, { ok: true });
    queryClient.setQueryData(transactionKey, { ok: true });
    queryClient.setQueryData(unrelatedKey, { ok: true });

    invalidateDaemonQueriesForMutation(
      queryClient,
      "real",
      "ui.journals.process",
    );

    expect(queryClient.getQueryState(journalKey)?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(transactionKey)?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(unrelatedKey)?.isInvalidated).toBe(false);
  });

  it("limits attachment renames to attachment and evidence reads", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.attachments.rename"),
    ).toEqual([
      "ui.attachments.list",
      "ui.audit.evidence.summary",
      "ui.source_funds.evidence.list",
      "ui.source_funds.preview",
    ]);
  });

  it("refreshes journal-derived reads after journal processing", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.journals.process"),
    ).toEqual(
      expect.arrayContaining([
        "ui.journals.events.list",
        "ui.transactions.extremes",
        "ui.transactions.graph",
        "ui.transactions.list",
        "ui.transactions.resolve",
      ]),
    );
  });

  it("refreshes node and lightning reads after sync and maintenance refreshes", () => {
    for (const kind of ["ui.freshness.run", "ui.wallets.sync"]) {
      expect(invalidatedDaemonQueryKindsForMutation(kind)).toEqual(
        expect.arrayContaining([
          "ui.connections.node.snapshot",
          "ui.journals.events.list",
          "ui.rates.coverage",
          "ui.reports.lightning_profitability",
          "ui.transactions.graph",
          "ui.transactions.resolve",
        ]),
      );
    }
  });

  it("refreshes transaction-derived reads after document OCR import", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation(
        "ui.wallets.document_import.import",
      ),
    ).toEqual(
      expect.arrayContaining([
        "ui.transactions.extremes",
        "ui.transactions.list",
        "ui.review.badges",
        "ui.workspace.health",
      ]),
    );
  });

  it("does not invalidate daemon reads after read-only backend probes", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.electrum.test"),
    ).toEqual([]);
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.http.test"),
    ).toEqual([]);
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.bitcoinrpc.test"),
    ).toEqual([]);
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.lightning.test"),
    ).toEqual([]);
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.detect_core"),
    ).toEqual([]);
  });

  it("refreshes backend default reads after changing the default backend", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.backends.set_default"),
    ).toEqual([
      "status",
      "ui.backends.list",
      "ui.backends.options",
      "ui.backends.public_defaults",
      "ui.backends.settings.list",
    ]);
  });

  it("keeps unaudited mutations on broad daemon invalidation", () => {
    expect(
      invalidatedDaemonQueryKindsForMutation("ui.transactions.metadata.update"),
    ).toBeNull();
  });
});

describe("daemon session advancing mutations", () => {
  it("advances after profile switches so cached pages cannot cross books", () => {
    expect(mutationAdvancesDaemonSession("ui.profiles.switch")).toBe(true);
    expect(mutationAdvancesDaemonSession("ui.transactions.metadata.update")).toBe(
      false,
    );
  });
});
