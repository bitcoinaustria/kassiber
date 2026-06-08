import { describe, expect, it } from "vitest";

import {
  activeSyncMaintenanceProgress,
  formatSyncProgressBody,
  syncProgressNotification,
} from "./syncProgress";

describe("sync progress notifications", () => {
  it("formats deterministic wallet progress", () => {
    expect(
      formatSyncProgressBody({
        wallet: "Cold",
        phase: "backend_fetch",
        processed: 12,
        total: 24,
      }),
    ).toBe("Cold: Fetching source history; 12 / 24 rows scanned.");

    const progress = syncProgressNotification({
      wallet: "Cold",
      phase: "backend_fetch",
      processed: 12,
      total: 24,
    });

    expect(progress.value).toBe(50);
    expect(progress.progress).toEqual({
      value: 50,
      indeterminate: false,
      label: "Cold: Fetching source history · 12 / 24",
    });
  });

  it("caps fallback progress before completion", () => {
    const progress = syncProgressNotification({}, 84);

    expect(progress.value).toBe(85);
    expect(progress.progress).toEqual({
      value: 85,
      indeterminate: false,
      label: "Refreshing configured sources",
    });
  });

  it("clamps progress when daemon counters exceed the reported total", () => {
    const progress = syncProgressNotification({
      wallet: "Cold",
      phase: "import",
      processed: 30,
      total: 24,
    });

    expect(progress.value).toBe(100);
    expect(progress.progress).toEqual({
      value: 100,
      indeterminate: false,
      label: "Cold: Importing transactions · 30 / 24",
    });
  });

  it("formats daemon-owned freshness phases without counters", () => {
    expect(
      formatSyncProgressBody({
        source_label: "Market-rate coverage",
        phase: "rate_coverage",
      }),
    ).toBe("Market-rate coverage: Checking market-rate coverage.");

    expect(
      syncProgressNotification({
        source_label: "Market-rate coverage",
        phase: "rate_coverage",
      }).progress,
    ).toEqual({
      value: 86,
      indeterminate: false,
      label: "Market-rate coverage: Checking market-rate coverage",
    });
  });

  it("weights daemon-owned phases across freshness sources", () => {
    const progress = syncProgressNotification({
      source_label: "Cold",
      source_type: "onchain_wallet",
      phase: "backend_fetch",
      job_index: 2,
      job_total: 4,
    });

    expect(progress.value).toBe(36.5);
    expect(progress.progress).toEqual({
      value: 36.5,
      indeterminate: false,
      label: "Cold: Fetching source history",
    });
  });

  it("builds active maintenance card details from sync progress", () => {
    const progress = activeSyncMaintenanceProgress(
      {
        source_label: "Cold",
        source_type: "onchain_wallet",
        phase: "backend_fetch",
        job_index: 2,
        job_total: 4,
        processed: 300,
        total: 600,
        imported: 42,
        skipped: 258,
      },
      5,
      {
        startedAt: "2026-06-06T10:00:00.000Z",
        updatedAt: "2026-06-06T10:01:00.000Z",
      },
    );

    expect(progress).toMatchObject({
      id: "book-refresh",
      title: "Refreshing book",
      body: "Cold: Fetching source history.",
      progress: {
        value: 37.5,
        indeterminate: false,
        label: "Cold: Fetching source history · 300 / 600",
      },
      details: [
        "Source 2 of 4",
        "Cold",
        "300 / 600 rows scanned",
        "42 imported · 258 unchanged",
      ],
      active: true,
    });
  });
});
