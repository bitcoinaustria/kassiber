import { describe, it, expect } from "vitest";

import { resolveEntryAction } from "./notificationRouting";

describe("resolveEntryAction records and anchors", () => {
  const here = { book: "book-a" };

  it("opens a connection or transaction of the open book", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: "wallet-1" },
        here,
        "book-a",
        false,
      ),
    ).toEqual({ kind: "open-connection", connectionId: "wallet-1" });
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "tx-1" },
        here,
        "book-a",
        false,
      ),
    ).toEqual({ kind: "open-transaction", transactionId: "tx-1" });
  });

  it("opens the list instead when the record belongs to another book", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: "wallet-1" },
        here,
        "book-b",
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/connections", stale: true });
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "tx-1" },
        here,
        "book-b",
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/transactions", stale: true });
  });

  it("refuses blank or oversized record ids", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: " " },
        here,
        "book-a",
        false,
      ),
    ).toBeNull();
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "x".repeat(300) },
        here,
        "book-a",
        false,
      ),
    ).toBeNull();
  });

  it("keeps a settings anchor only on its own section's route", () => {
    expect(
      resolveEntryAction(
        { type: "navigate", to: "/settings/security", anchor: "biometricUnlock" },
        { book: null },
        null,
        false,
      ),
    ).toEqual({
      kind: "navigate",
      to: "/settings/security",
      stale: false,
      anchor: "biometricUnlock",
    });
    expect(
      resolveEntryAction(
        { type: "navigate", to: "/settings/market", anchor: "biometricUnlock" },
        here,
        "book-a",
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/settings/market", stale: false });
    expect(
      resolveEntryAction(
        { type: "navigate", to: "/settings/market", anchor: "no-such-row" },
        here,
        "book-a",
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/settings/market", stale: false });
  });

  it("refuses unknown action types", () => {
    expect(
      resolveEntryAction(
        { type: "run-shell", command: "ls" } as never,
        here,
        "book-a",
        true,
      ),
    ).toBeNull();
  });
});
