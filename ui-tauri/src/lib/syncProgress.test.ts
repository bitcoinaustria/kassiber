import { describe, expect, it } from "vitest";

import {
  activeSyncMaintenanceProgress,
  FIRST_SYNC_MILESTONES,
  firstSyncActiveMilestoneIndex,
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

  it("surfaces a rate-limit backoff without nudging the bar forward", () => {
    // A 429/503 backoff is a wait: the label must read as "rate limited,
    // retrying" rather than freezing silently, and the bar must hold steady.
    const progress = syncProgressNotification(
      {
        wallet: "Cold",
        phase: "rate_limited",
        retry_attempt: 1,
        retry_max: 2,
        wait_seconds: 2,
      },
      46,
    );

    expect(progress.value).toBe(46);
    expect(progress.progress.label).toBe("Cold: Waiting out rate limit");

    const card = activeSyncMaintenanceProgress(
      {
        source_label: "Cold",
        source_type: "onchain_wallet",
        phase: "rate_limited",
        retry_attempt: 1,
        retry_max: 2,
        wait_seconds: 2,
      },
      46,
      {
        startedAt: "2026-06-06T10:00:00.000Z",
        updatedAt: "2026-06-06T10:01:00.000Z",
      },
    );

    expect(card.title).toBe("Waiting out rate limit");
    expect(card.progress.value).toBe(46);
    expect(card.details).toContain("Rate limited — retry 1/2 in 2s");
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

describe("first-sync milestones", () => {
  const indexOfPhase = (phase: string) =>
    FIRST_SYNC_MILESTONES.findIndex((m) => m.phase === phase);

  it("treats the first milestone as active before any determinate value", () => {
    expect(firstSyncActiveMilestoneIndex(0, false)).toBe(0);
    // Indeterminate wins even if a stale fraction is passed.
    expect(firstSyncActiveMilestoneIndex(0.9, false)).toBe(0);
  });

  it("keeps a phase active while the bar is still within it", () => {
    // Regression guard: at a phase's own threshold the phase is the CURRENT one,
    // not already done (the `<=` vs `<` off-by-one).
    expect(firstSyncActiveMilestoneIndex(0.12, true)).toBe(
      indexOfPhase("discovery"),
    );
    expect(firstSyncActiveMilestoneIndex(0.46, true)).toBe(
      indexOfPhase("backend_fetch"),
    );
  });

  it("advances to the next phase once the bar passes a threshold", () => {
    expect(firstSyncActiveMilestoneIndex(0.13, true)).toBe(
      indexOfPhase("backend_fetch"),
    );
    expect(firstSyncActiveMilestoneIndex(0.5, true)).toBe(
      indexOfPhase("decode_enrich"),
    );
  });

  it("reports every phase done at or past completion", () => {
    expect(firstSyncActiveMilestoneIndex(0.95, true)).toBe(
      FIRST_SYNC_MILESTONES.length,
    );
    expect(firstSyncActiveMilestoneIndex(1, true)).toBe(
      FIRST_SYNC_MILESTONES.length,
    );
  });
});
