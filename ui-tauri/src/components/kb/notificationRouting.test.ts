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

  it("prefers an explicit target over title keyword-matching", () => {
    // A localized warning title the English keyword router can't recognize
    // (e.g. German "needs attention") must still route via its explicit target.
    const germanAttention = "Buch-Aktualisierung braucht Aufmerksamkeit";
    expect(notificationTarget(germanAttention, "warning", false)).toBeUndefined();
    expect(
      notificationTarget(germanAttention, "warning", true, "/logs"),
    ).toBe("/logs");
    // The explicit /logs target still flows through the dev-tools guard.
    expect(
      notificationTarget(germanAttention, "warning", false, "/logs"),
    ).toBe("/settings");
    expect(
      notificationTarget(germanAttention, "warning", false, "/quarantine"),
    ).toBe("/quarantine");

    const germanTransferReview = "2 Swap-/Transfer-Kandidaten prüfen";
    expect(
      notificationTarget(germanTransferReview, "warning", false),
    ).toBeUndefined();
    expect(
      notificationTarget(germanTransferReview, "warning", false, "/swaps"),
    ).toBe("/swaps");
  });

  it("ignores an explicit target that is not a known route", () => {
    expect(
      notificationTarget("All caught up", "info", false, "/not-a-route"),
    ).toBeUndefined();
  });
});
