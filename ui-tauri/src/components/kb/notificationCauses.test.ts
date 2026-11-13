import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import type { ConditionGroup, NotificationDetail } from "@/store/notifications";

import { classifyCause, groupDetails } from "./notificationCauses";
import {
  assistantTroubleshootPrompt,
  projectConditionGroup,
} from "./notificationProjection";

const t = i18n.getFixedT("en", "chrome");

describe("notification causes", () => {
  it("reads the daemon's errors as plain causes", () => {
    const cases: Array<[string, string]> = [
      ["network I/O failed", "connection"],
      ["electrum: connection refused (127.0.0.1:60401) after 3 attempts", "connection"],
      ["Backend request budget exhausted", "connection"],
      ["Electrum timeout", "connection"],
      ["Outbound requests are disabled: Kassiber is in offline mode", "offline"],
      ["Backend rate limited the request for mempool (HTTP 429)", "rateLimited"],
      ["Backend is temporarily unavailable for mempool (HTTP 503)", "unavailable"],
      ["esplora: HTTP 503 Service Unavailable", "unavailable"],
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

  it("uses the error code, and never reads the hint as the cause", () => {
    expect(classifyCause({ message: "something odd", code: "network_error" })).toBe(
      "connection",
    );
    // The Electrum format error's hint mentions "tcp/ssl port": not a TLS failure.
    const groups = groupDetails([
      {
        label: "Node",
        message: "Backend 'node' did not respond with Electrum-format JSON",
        hint: "Check that the backend URL points to an Electrum server and uses the correct tcp/ssl port.",
        code: "app_error",
      },
    ]);
    expect(groups[0].cause).toBeNull();
    expect(groups[0].messages[0].text).toContain("tcp/ssl port");
  });

  it("does not read a cause out of a backend or wallet name", () => {
    // The Electrum format error names the backend; "tls-node" is a name.
    expect(
      classifyCause({
        message: "Backend 'tls-node' did not respond with Electrum-format JSON",
        code: "app_error",
      }),
    ).toBeNull();
    expect(
      classifyCause({
        label: "tls-node",
        message: "tls-node returned HTTP 401",
      }),
    ).toBe("credentials");
    expect(
      classifyCause({ message: "anything", code: "network_egress_disabled" }),
    ).toBe("offline");
    // An apostrophe inside a quoted name does not leak its words.
    expect(
      classifyCause({
        label: "Wallet",
        message: "Backend 'Bob's timeout' did not respond with Electrum-format JSON",
      }),
    ).toBeNull();
    // The status outranks the server's own wording.
    expect(
      classifyCause({ message: "HTTP 401 from backend for host: service unavailable" }),
    ).toBe("credentials");
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
      ["connection", 12],
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

  it("says one cause once, as a count when many wallets share it", () => {
    const projected = projectConditionGroup(twelve, "normal", t);
    expect(projected.title).toBe("12 sources could not refresh");
    expect(projected.details).toEqual(["Connection to the server failed (12)"]);
    expect(projected.technical).toEqual(["network I/O failed (×12)"]);
  });

  it("names one or two wallets", () => {
    const projected = projectConditionGroup(
      sourceGroup([
        { label: "Cold", message: "network I/O failed" },
        { label: "Hot", message: "network I/O failed" },
      ]),
      "normal",
      t,
    );
    expect(projected.details).toEqual(["Connection to the server failed: Cold, Hot"]);
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
    expect(projected.details).toEqual(["Connection to the server failed: ••••"]);
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
    expect(projected.technical).toEqual(["Wallet not found (×2)"]);
  });
});

describe("the assistant's question", () => {
  it("carries causes and wallet names, never raw messages, hints, or paths", () => {
    const prompt = assistantTroubleshootPrompt(
      sourceGroup([
        { label: "Cold", message: "connection refused (10.0.0.5:50002)", hint: "Check host 10.0.0.5" },
        { label: "Backup", message: "Could not write /Users/alice/private/recovery.kassiber" },
      ]),
      "normal",
      t,
    );
    expect(prompt).toContain("2 sources could not refresh");
    expect(prompt).toContain("Connection to the server failed: Cold");
    expect(prompt).toContain("1 other error (its text is not shared)");
    expect(prompt).not.toContain("10.0.0.5");
    expect(prompt).not.toContain("/Users/alice");
    expect(prompt).not.toContain("Backup");
  });

  it("never sends a name that is a path or URL, nor a title that names one", () => {
    for (const mode of ["normal", "hideAmounts"] as const) {
      const prompt = assistantTroubleshootPrompt(
        sourceGroup([
          { label: "/Users/alice/private/recovery.kassiber", message: "network I/O failed" },
          { label: "https://node.example/private", message: "network I/O failed" },
          { label: "[fdab::abcd]", message: "network I/O failed" },
          { label: "node:8", message: "network I/O failed" },
          { label: "Cold", message: "network I/O failed" },
        ]),
        mode,
        t,
      );
      expect(prompt, mode).not.toContain("/Users/alice");
      expect(prompt, mode).not.toContain("node.example");
      expect(prompt, mode).not.toContain("fdab");
      expect(prompt, mode).not.toContain("node:8");
      // Hide amounts masks "node:8" (a digit) to ••••, which reveals nothing.
      expect(prompt, mode).toContain(
        mode === "normal"
          ? "Connection to the server failed: Cold (+4 more)"
          : "Connection to the server failed: ••••, Cold (+3 more)",
      );
    }
    const named = {
      ...sourceGroup([{ label: "https://node.example/x", message: "network I/O failed" }]),
      code: "connection.sync-failed" as const,
    };
    named.entries = named.entries.map((entry) => ({
      ...entry,
      code: "connection.sync-failed" as const,
      params: { sources: ["https://node.example/x"] },
    }));
    const prompt = assistantTroubleshootPrompt(named, "normal", t);
    expect(prompt).not.toContain("node.example");
    expect(prompt).toContain("A wallet could not sync");
  });

  it("drops wallet names under Hide all", () => {
    const prompt = assistantTroubleshootPrompt(
      sourceGroup([{ label: "Cold", message: "network I/O failed" }]),
      "hideAll",
      t,
    );
    expect(prompt).toContain("Connection to the server failed");
    expect(prompt).not.toContain("Cold");
  });
});
