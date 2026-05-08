import { describe, expect, it } from "vitest";

import {
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
