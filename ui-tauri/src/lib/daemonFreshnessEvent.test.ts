import { describe, expect, it } from "vitest";

import { classifyDaemonFreshnessEvent } from "./daemonFreshnessEvent";

const event = (kind: string, data: unknown) => ({
  kind,
  schema_version: 1,
  event: true,
  data,
});

describe("classifyDaemonFreshnessEvent", () => {
  it("invalidates daemon reads after a clean background run", () => {
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.background", {
          completed: [{ status: "done" }],
          sources: [{ status: "fresh" }],
        }),
      ),
    ).toBe("refresh");
  });

  it("surfaces terminal jobs and persisted source blockers", () => {
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.background", {
          completed: [{ status: "error" }],
        }),
      ),
    ).toBe("needs-attention");
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.background", {
          completed: [],
          sources: [{ status: "failed", blocking_reports: true }],
        }),
      ),
    ).toBe("needs-attention");
  });

  it("surfaces worker failures but ignores progress and response envelopes", () => {
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.worker", { status: "unavailable" }),
      ),
    ).toBe("worker-error");
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.progress", { phase: "backend_fetch" }),
      ),
    ).toBeNull();
    expect(
      classifyDaemonFreshnessEvent({
        ...event("ui.freshness.background", { completed: [] }),
        request_id: "foreground-request",
      }),
    ).toBeNull();
  });

  it("fails closed on malformed event data", () => {
    expect(
      classifyDaemonFreshnessEvent(
        event("ui.freshness.background", {
          completed: [null, "not-a-job"],
          sources: "not-a-list",
        }),
      ),
    ).toBe("refresh");
    expect(
      classifyDaemonFreshnessEvent({
        kind: "ui.freshness.background",
        schema_version: 1,
        data: {},
      }),
    ).toBeNull();
  });
});
