import { describe, expect, it } from "vitest";

import {
  filterNotifications,
  notificationNeedsAction,
  shouldHideNotificationProgressLabel,
} from "./notificationDisplay";

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
