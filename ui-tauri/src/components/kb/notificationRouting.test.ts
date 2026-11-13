import { describe, it, expect } from "vitest";

import type { ActiveBook } from "@/lib/activeBook";

import { resolveEntryAction } from "./notificationRouting";

const BOOK_A: ActiveBook = {
  key: "book-a",
  expected: { workspace_id: "ws", profile_id: "pf-a" },
};
const BOOK_B: ActiveBook = {
  key: "book-b",
  expected: { workspace_id: "ws", profile_id: "pf-b" },
};

describe("resolveEntryAction records and anchors", () => {
  const here = { book: "book-a", expected: BOOK_A.expected };

  it("opens a connection or transaction of the open book", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: "wallet-1" },
        here,
        BOOK_A,
        false,
      ),
    ).toEqual({ kind: "open-connection", connectionId: "wallet-1" });
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "tx-1" },
        here,
        BOOK_A,
        false,
      ),
    ).toEqual({ kind: "open-transaction", transactionId: "tx-1" });
  });

  it("opens the list instead when the record belongs to another book", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: "wallet-1" },
        here,
        BOOK_B,
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/connections", stale: true });
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "tx-1" },
        here,
        BOOK_B,
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/transactions", stale: true });
  });

  it("refuses blank or oversized record ids", () => {
    expect(
      resolveEntryAction(
        { type: "open-connection", connectionId: " " },
        here,
        BOOK_A,
        false,
      ),
    ).toBeNull();
    expect(
      resolveEntryAction(
        { type: "open-transaction", transactionId: "x".repeat(300) },
        here,
        BOOK_A,
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
        BOOK_A,
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/settings/market", stale: false });
    expect(
      resolveEntryAction(
        { type: "navigate", to: "/settings/market", anchor: "no-such-row" },
        here,
        BOOK_A,
        false,
      ),
    ).toEqual({ kind: "navigate", to: "/settings/market", stale: false });
  });

  it("refuses unknown action types", () => {
    expect(
      resolveEntryAction(
        { type: "run-shell", command: "ls" } as never,
        here,
        BOOK_A,
        true,
      ),
    ).toBeNull();
  });
});
