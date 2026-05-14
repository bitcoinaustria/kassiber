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
        processed: 12,
        total: 24,
      }),
    ).toBe("Cold: 12 / 24 transactions scanned.");

    const progress = syncProgressNotification({
      wallet: "Cold",
      processed: 12,
      total: 24,
    });

    expect(progress.value).toBe(50);
    expect(progress.progress).toEqual({
      value: 50,
      indeterminate: false,
      label: "Scanning transactions: 12 / 24",
    });
  });

  it("caps fallback progress before completion", () => {
    const progress = syncProgressNotification({}, 84);

    expect(progress.value).toBe(85);
    expect(progress.progress).toEqual({
      value: 85,
      indeterminate: false,
      label: "Scanning configured sources",
    });
  });
});
