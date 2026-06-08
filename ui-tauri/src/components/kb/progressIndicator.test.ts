import { describe, expect, it } from "vitest";

import type { AppNotification } from "@/store/ui";

import {
  routeProgressFromActiveMaintenance,
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
        active: true,
        startedAt: "2026-06-07T00:00:00Z",
        updatedAt: "2026-06-07T00:01:00Z",
      }),
    ).toEqual({
      indeterminate: false,
      label: "Refreshing book: Cold: Fetching source history · 300 / 600",
      value: 37.5,
    });
  });
});
