import { describe, expect, it } from "vitest";

import {
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
      label: "Fetching source history: 12 / 24",
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
      label: "Importing transactions: 30 / 24",
    });
  });

  it("formats daemon-owned freshness phases without counters", () => {
    expect(
      formatSyncProgressBody({
        source_label: "Market-rate coverage",
        phase: "rate_coverage",
      }),
    ).toBe("Market-rate coverage: Checking market-rate coverage.");
  });
});
