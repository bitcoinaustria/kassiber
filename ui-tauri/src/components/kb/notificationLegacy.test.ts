import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { describe, expect, it } from "vitest";

import { useUiStore } from "@/store/ui";

/**
 * The legacy notification path is gone: every sender says what it is (a
 * toast, a condition, a job, or a result) through `lib/feedback.ts` and
 * `store/notifications.ts`, and nothing routes by guessing from a title.
 * This keeps it gone.
 */
const SRC = join(__dirname, "..", "..");
const THIS_FILE = relative(SRC, __filename);
const RETIRED = [
  "addNotification",
  "updateNotification",
  "AppNotification",
  "LegacyRow",
  "projectLegacy",
  "notificationRouteFor",
  "notificationTarget(",
  "resolveNotificationAction",
  "NOTIFICATION_COMMANDS",
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx)$/.test(name) ? [path] : [];
  });
}

describe("legacy notification path", () => {
  it("is not referenced anywhere in the desktop source", () => {
    const hits: string[] = [];
    for (const file of sourceFiles(SRC)) {
      const name = relative(SRC, file);
      if (name === THIS_FILE) continue;
      const text = readFileSync(file, "utf8");
      for (const symbol of RETIRED) {
        if (text.includes(symbol)) hits.push(`${name}: ${symbol}`);
      }
    }
    expect(hits).toEqual([]);
  });

  it("has no sender on the UI store", () => {
    const state = useUiStore.getState() as unknown as Record<string, unknown>;
    expect(state.addNotification).toBeUndefined();
    expect(state.notifications).toBeUndefined();
    // @ts-expect-error The UI store no longer has a notification list.
    void useUiStore.getState().addNotification;
  });
});
