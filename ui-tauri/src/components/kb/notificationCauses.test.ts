import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import type { ConditionGroup, NotificationDetail } from "@/store/notifications";

import { classifyCause, groupDetails } from "./notificationCauses";
import { projectConditionGroup } from "./notificationProjection";

const t = i18n.getFixedT("en", "chrome");

describe("notification causes", () => {
  it("reads the daemon's network errors as plain causes", () => {
    const cases: Array<[string, string]> = [
      ["network I/O failed", "unreachable"],
      ["electrum: connection refused (127.0.0.1:60401) after 3 attempts", "unreachable"],
      ["Backend request budget exhausted", "unreachable"],
      ["Electrum timeout", "unreachable"],
      ["Outbound requests are disabled: Kassiber is in offline mode", "offline"],
      ["esplora: HTTP 503 Service Unavailable", "rateLimited"],
      ["HTTP 429 Too Many Requests", "rateLimited"],
      ["TLS handshake failed: certificate is not trusted", "certificate"],
      ["Could not load the custom CA bundle", "certificate"],
      ["BTCPay Greenfield API returned 401 Unauthorized", "credentials"],
      ["No sync backend is configured", "noBackend"],
    ];
    for (const [message, cause] of cases) {
      expect(classifyCause({ message }), message).toBe(cause);
    }
    expect(classifyCause({ phraseKey: "coolingDown" })).toBe("rateLimited");
    expect(classifyCause({ phraseKey: "cancelled" })).toBe("cancelled");
    expect(classifyCause({ message: "Wallet not found" })).toBeNull();
  });

  it("folds wallets with the same cause into one group, biggest first", () => {
    const details: NotificationDetail[] = [
      ...Array.from({ length: 12 }, (_, index) => ({
        label: `Wallet ${index}`,
        message: "network I/O failed",
        phraseKey: "blocksReports" as const,
      })),
      { label: "Shop", message: "HTTP 429" },
      { label: "Odd", message: "Wallet not found" },
      { label: "Odder", message: "Wallet not found" },
    ];
    const groups = groupDetails(details);
    expect(groups.map((group) => [group.cause, group.count])).toEqual([
      ["unreachable", 12],
      [null, 2],
      ["rateLimited", 1],
    ]);
    expect(groups[0].messages).toEqual([{ text: "network I/O failed", count: 12 }]);
    expect(groups[1].labels).toEqual(["Odd", "Odder"]);
  });
});

function sourceGroup(details: NotificationDetail[]): ConditionGroup {
  const now = "2026-11-13T10:00:00Z";
  return {
    key: "group",
    code: "source.refresh-failed",
    severity: "warning",
    priority: "blocking",
    entries: details.map((detail, index) => ({
      id: `c${index}`,
      kind: "condition",
      code: "source.refresh-failed",
      scope: { book: "b", source: `s${index}` },
      severity: "warning",
      priority: "blocking",
      firstOccurredAt: now,
      lastOccurredAt: now,
      occurrenceCount: 1,
      params: { blocking: true },
      details: [detail],
    })),
  } as ConditionGroup;
}

describe("grouped detail lines", () => {
  const twelve = sourceGroup(
    Array.from({ length: 12 }, (_, index) => ({
      label: `Wallet ${String.fromCharCode(65 + index)}`,
      message: "network I/O failed",
      phraseKey: "blocksReports" as const,
    })),
  );

  it("says one cause once, with the first names and how many more", () => {
    const projected = projectConditionGroup(twelve, "normal", t);
    expect(projected.title).toBe("12 sources could not refresh");
    expect(projected.details).toEqual([
      "Can't reach the server: Wallet A, Wallet B +10 more",
    ]);
    expect(projected.technical).toEqual(["network I/O failed (×12)"]);
  });

  it("keeps the cause but drops names and raw text under Hide all", () => {
    const projected = projectConditionGroup(twelve, "hideAll", t);
    expect(projected.details).toEqual([]);
    expect(projected.technical).toEqual([]);
  });

  it("withholds raw text with digits under Hide amounts and masks such names", () => {
    const projected = projectConditionGroup(
      sourceGroup([
        { label: "Savings 0.42", message: "connection refused (127.0.0.1:60401)" },
      ]),
      "hideAmounts",
      t,
    );
    expect(projected.details).toEqual(["Can't reach the server: ••••"]);
    expect(projected.technical).toEqual(["Hidden while amounts are hidden."]);
  });

  it("keeps an unknown error as its own line, once per distinct message", () => {
    const projected = projectConditionGroup(
      sourceGroup([
        { label: "A", message: "Wallet not found" },
        { label: "B", message: "Wallet not found" },
      ]),
      "normal",
      t,
    );
    expect(projected.details).toEqual(["A, B: Wallet not found"]);
    expect(projected.technical).toEqual([]);
  });
});
