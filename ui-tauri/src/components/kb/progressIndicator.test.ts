import { describe, expect, it } from "vitest";

import type { AppNotification } from "@/store/ui";

import {
  routeProgressFromNotifications,
  routeProgressLabelFromNotifications,
} from "./progressIndicator";

function notification(
  title: string,
  progress?: AppNotification["progress"],
): AppNotification {
  return {
    id: title,
    title,
    body: "body",
    tone: "warning",
    progress,
    createdAt: "2026-06-07T00:00:00Z",
  };
}

describe("route progress indicator label", () => {
  it("stays quiet when no daemon progress notification is active", () => {
    expect(
      routeProgressLabelFromNotifications([
        notification("Route changed"),
      ]),
    ).toBeNull();
  });

  it("combines compact refresh titles with progress labels", () => {
    expect(
      routeProgressLabelFromNotifications([
        notification("Book refresh started", {
          value: 40,
          label: "Fetching source history",
        }),
      ]),
    ).toBe("Book refresh: Fetching source history");
  });

  it("returns progress rail state for the active progress notification", () => {
    expect(
      routeProgressFromNotifications([
        notification("Book refresh started", {
          value: 40,
          label: "Fetching source history",
        }),
      ]),
    ).toEqual({
      indeterminate: false,
      label: "Book refresh: Fetching source history",
      value: 40,
    });
  });

  it("uses the newest active progress notification", () => {
    expect(
      routeProgressLabelFromNotifications([
        notification("BTC price refresh started", {
          indeterminate: true,
          label: "Refreshing",
        }),
        notification("Book refresh started", {
          value: 40,
          label: "Fetching source history",
        }),
      ]),
    ).toBe("BTC price: Refreshing");
  });
});
