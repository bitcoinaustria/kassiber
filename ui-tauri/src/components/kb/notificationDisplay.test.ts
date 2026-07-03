import { describe, expect, it } from "vitest";

import { shouldHideNotificationProgressLabel } from "./notificationDisplay";

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
