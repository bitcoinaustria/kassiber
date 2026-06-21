import { describe, it, expect } from "vitest";

import { notificationRouteFor, notificationTarget } from "./notificationRouting";

describe("notificationRouteFor", () => {
  it("routes failures / needs-attention to /logs", () => {
    expect(notificationRouteFor("Book refresh failed")).toBe("/logs");
    expect(notificationRouteFor("Book refresh needs attention")).toBe("/logs");
    expect(notificationRouteFor("Daemon disconnected")).toBe("/logs");
  });

  it("routes domain titles to their own screens", () => {
    expect(notificationRouteFor("Transactions quarantined")).toBe("/quarantine");
    expect(
      notificationRouteFor("2 Transaktionen in Quarantäne prüfen"),
    ).toBe("/quarantine");
    expect(notificationRouteFor("Review 2 swap/transfer candidates")).toBe(
      "/swaps",
    );
    expect(
      notificationRouteFor("2 Swap-/Transfer-Kandidaten prüfen"),
    ).toBe("/swaps");
    expect(notificationRouteFor("Journal processing complete")).toBe("/journals");
  });
});

describe("notificationTarget", () => {
  it("keeps /logs failures on /logs when developer tools are ON", () => {
    expect(notificationTarget("Book refresh failed", "error", true)).toBe(
      "/logs",
    );
  });

  it("reroutes /logs failures to /settings when developer tools are OFF", () => {
    // /logs is developer-tools-gated; its route guard bounces to /overview, so
    // a failure notification must not dead-end there when dev tools are off.
    expect(notificationTarget("Book refresh failed", "error", false)).toBe(
      "/settings",
    );
    expect(
      notificationTarget("Book refresh needs attention", "warning", false),
    ).toBe("/settings");
  });

  it("routes non-/logs titles to their screen regardless of dev tools", () => {
    expect(notificationTarget("Transactions quarantined", "warning", false)).toBe(
      "/quarantine",
    );
    expect(
      notificationTarget("2 Transaktionen in Quarantäne prüfen", "warning", false),
    ).toBe("/quarantine");
    expect(
      notificationTarget("Review 2 swap/transfer candidates", "warning", false),
    ).toBe("/swaps");
    expect(notificationTarget("Transactions quarantined", "warning", true)).toBe(
      "/quarantine",
    );
  });

  it("uses a dev-tools-aware fallback for unmatched error-tone titles", () => {
    expect(notificationTarget("Something odd happened", "error", true)).toBe(
      "/logs",
    );
    expect(notificationTarget("Something odd happened", "error", false)).toBe(
      "/settings",
    );
  });

  it("returns undefined for unmatched non-error titles", () => {
    expect(notificationTarget("All caught up", "info", false)).toBeUndefined();
  });
});
