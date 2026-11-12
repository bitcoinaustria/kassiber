import { describe, expect, it } from "vitest";

import {
  filterNotifications,
  notificationIsExpandable,
  notificationNeedsAction,
  shouldHideNotificationProgressLabel,
} from "./notificationDisplay";
import { formatDateTime, formatRelativeTime } from "@/lib/localeFormat";

describe("notification progress display", () => {
  it("hides progress captions that repeat the notification body lead", () => {
    expect(
      shouldHideNotificationProgressLabel(
        "Events-Liquid: Fetching source history; 36 / 36 scan targets checked.",
        "Events-Liquid: Fetching source history · 36 / 36",
      ),
    ).toBe(true);
  });

  it("keeps progress captions that add non-redundant context", () => {
    expect(
      shouldHideNotificationProgressLabel(
        "Kassiber is rescanning configured sources and journals.",
        "Preparing source refresh",
      ),
    ).toBe(false);
  });
});

describe("notification needs-action filter", () => {
  const items = [
    { id: "ok", tone: "success" },
    { id: "info", tone: "info" },
    { id: "warn", tone: "warning" },
    { id: "err", tone: "error" },
    { id: "running", tone: "warning", progress: { indeterminate: true } },
    { id: "journals", tone: "info", action: "process-journals" },
  ];

  it("keeps every item for All", () => {
    expect(filterNotifications(items, "all").map((item) => item.id)).toEqual([
      "ok",
      "info",
      "warn",
      "err",
      "running",
      "journals",
    ]);
  });

  it("keeps settled warnings, errors, and inline actions for Needs action", () => {
    expect(
      filterNotifications(items, "needsAction").map((item) => item.id),
    ).toEqual(["warn", "err", "journals"]);
  });

  it("does not treat a running job as needing action", () => {
    expect(
      notificationNeedsAction({ tone: "warning", progress: { value: 40 } }),
    ).toBe(false);
  });
});

describe("notification expansion", () => {
  it("expands for detail lines, long bodies, and multi-line bodies", () => {
    expect(notificationIsExpandable({ body: "Done." })).toBe(false);
    expect(notificationIsExpandable({ body: "Done.", details: [" "] })).toBe(false);
    expect(notificationIsExpandable({ body: "Done.", details: ["Cold: timeout"] })).toBe(true);
    expect(notificationIsExpandable({ body: "x".repeat(121) })).toBe(true);
    expect(notificationIsExpandable({ body: "Failed\n\nCheck the backend." })).toBe(true);
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
