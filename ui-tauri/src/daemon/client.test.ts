import { describe, expect, it } from "vitest";

import {
  daemonMutationKey,
  invalidatedDaemonQueryKindsForMutation,
  mutationAdvancesDaemonSession,
  parseDaemonAuthRequiredEventDetail,
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
    expect(daemonMutationKey("real", "ui.journals.process")).not.toEqual(
      daemonMutationKey("real", "ui.wallets.sync"),
    );
  });
});

describe("daemon mutation invalidation scope", () => {
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
