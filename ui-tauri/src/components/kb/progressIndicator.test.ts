import { describe, expect, it } from "vitest";

import type { JobEntry } from "@/store/notifications";

import {
  daemonQueryUsesShellProgress,
  routeProgressFromActiveMaintenance,
  routeProgressFromJobs,
} from "./progressIndicator";

function job(
  id: string,
  startedAt: string,
  progress: JobEntry["progress"],
): JobEntry {
  return {
    id,
    code: "connection-sync",
    scope: { book: "book" },
    startedAt,
    updatedAt: startedAt,
    params: {},
    progress,
  };
}

const describeJob = (entry: JobEntry) => ({
  title: entry.id === "rebuild" ? "Rebuilding prices" : "Syncing wallet",
  label: entry.progress.label,
});

describe("route progress indicator label", () => {
  it("only uses foreground daemon loads for shell progress", () => {
    expect(
      daemonQueryUsesShellProgress({ state: { data: undefined } }),
    ).toBe(true);
    expect(
      daemonQueryUsesShellProgress({ state: { data: { cached: true } } }),
    ).toBe(false);
    expect(
      daemonQueryUsesShellProgress({
        state: { data: undefined },
        meta: { shellProgress: false },
      }),
    ).toBe(false);
  });

  it("stays quiet when no job is running", () => {
    expect(routeProgressFromJobs([], describeJob)).toBeNull();
  });

  it("combines the job title with its phase", () => {
    expect(
      routeProgressFromJobs(
        [job("sync", "2026-06-07T00:00:00Z", { value: 40, label: "Fetching source history" })],
        describeJob,
      ),
    ).toEqual({
      indeterminate: false,
      label: "Syncing wallet: Fetching source history",
      value: 40,
    });
  });

  it("shows the newest running job", () => {
    expect(
      routeProgressFromJobs(
        [
          job("sync", "2026-06-07T00:00:00Z", { value: 40, label: "Fetching" }),
          job("rebuild", "2026-06-07T00:05:00Z", { indeterminate: true }),
        ],
        describeJob,
      ),
    ).toEqual({ indeterminate: true, label: "Rebuilding prices", value: undefined });
  });

  it("maps active maintenance progress to the route progress rail", () => {
    expect(
      routeProgressFromActiveMaintenance({
        id: "book-refresh",
        title: "Refreshing book",
        body: "Cold: Fetching source history.",
        tone: "warning",
        progress: {
          value: 37.5,
          indeterminate: false,
          label: "Cold: Fetching source history · 300 / 600",
        },
        state: "running",
        startedAt: "2026-06-07T00:00:00Z",
        updatedAt: "2026-06-07T00:01:00Z",
      }),
    ).toEqual({
      indeterminate: false,
      label: "Refreshing book: Cold: Fetching source history · 300 / 600",
      value: 37.5,
    });
  });

  it("does not keep the route progress rail busy for retained failures", () => {
    expect(
      routeProgressFromActiveMaintenance({
        id: "book-refresh",
        title: "Book refresh failed",
        body: "Cold could not connect.",
        tone: "error",
        progress: { value: 46, label: "Cold: Fetching source history" },
        state: "failed",
        phase: "backend_fetch",
        startedAt: "2026-06-07T00:00:00Z",
        updatedAt: "2026-06-07T00:01:00Z",
      }),
    ).toBeNull();
  });
});
