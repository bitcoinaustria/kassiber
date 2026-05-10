import { describe, expect, it } from "vitest";

import {
  daemonMutationKey,
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
  it("keeps legacy envelope-only events current", () => {
    expect(parseDaemonAuthRequiredEventDetail(authEnvelope)).toEqual({
      envelope: authEnvelope,
    });
    expect(shouldHandleDaemonAuthRequiredEvent(authEnvelope, 7)).toBe(true);
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
