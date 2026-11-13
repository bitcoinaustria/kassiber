import { describe, expect, it } from "vitest";

import { notificationProgressValue } from "./notificationDisplay";
import { formatDateTime, formatRelativeTime } from "@/lib/localeFormat";

describe("notification progress bar", () => {
  it("clamps to 0-100 and treats anything else as 0", () => {
    expect(notificationProgressValue(42)).toBe(42);
    expect(notificationProgressValue(140)).toBe(100);
    expect(notificationProgressValue(-3)).toBe(0);
    expect(notificationProgressValue(Number.NaN)).toBe(0);
    expect(notificationProgressValue(undefined)).toBe(0);
  });
});

describe("notification timestamps", () => {
  const now = Date.parse("2026-11-12T12:00:00Z");

  it("formats elapsed time in the UI language", () => {
    expect(formatRelativeTime("2026-11-12T11:59:40Z", now, "en")).toBe("now");
    expect(formatRelativeTime("2026-11-12T11:58:00Z", now, "en")).toBe("2 min. ago");
    expect(formatRelativeTime("2026-11-12T09:00:00Z", now, "en")).toBe("3 hr. ago");
    expect(formatRelativeTime("2026-11-11T11:00:00Z", now, "en")).toBe("yesterday");
    expect(formatRelativeTime("2026-11-12T11:58:00Z", now, "de")).toBe("vor 2 Min.");
  });

  it("reads clock skew as now and ignores bad timestamps", () => {
    expect(formatRelativeTime("2026-11-12T12:05:00Z", now, "en")).toBe("now");
    expect(formatRelativeTime("not a date", now, "en")).toBeNull();
    expect(formatRelativeTime(undefined, now, "en")).toBeNull();
    expect(formatDateTime("not a date", "en")).toBeNull();
    expect(formatDateTime("2026-11-12T11:58:00Z", "de")).toMatch(/2026/);
  });
});
