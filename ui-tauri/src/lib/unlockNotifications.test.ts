import { beforeEach, describe, expect, it } from "vitest";

import { resolveEntryAction } from "@/components/kb/notificationRouting";
import { useFeedbackStore } from "@/lib/feedback";
import { resetNotificationStore, useNotificationStore } from "@/store/notifications";

import {
  recordTouchIdOff,
  recordUnlockCredentialRemains,
  resolveTouchIdOff,
  resolveUnlockCredentialRemains,
} from "./unlockNotifications";

beforeEach(() => {
  resetNotificationStore();
  useFeedbackStore.setState({ items: [] });
});

describe("saved unlock conditions", () => {
  it("opens the Touch ID row in Security settings from any book", () => {
    recordTouchIdOff("Keychain refused the item");
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition).toMatchObject({
      code: "touch-id.off",
      scope: { book: null },
      details: [{ message: "Keychain refused the item" }],
    });
    expect(
      resolveEntryAction(condition.action, condition.scope, "any-book", false),
    ).toEqual({
      kind: "navigate",
      to: "/settings/security",
      stale: true,
      anchor: "biometricUnlock",
    });
    resolveTouchIdOff();
    expect(useNotificationStore.getState().conditions).toEqual({});
  });

  it("keeps a leftover credential until a removal finishes", () => {
    recordUnlockCredentialRemains(["not removed", "still there"]);
    recordUnlockCredentialRemains(["not removed"]);
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition).toMatchObject({
      code: "unlock.credential-remains",
      occurrenceCount: 2,
    });
    expect(useFeedbackStore.getState().items).toHaveLength(1);
    resolveUnlockCredentialRemains();
    expect(useNotificationStore.getState().conditions).toEqual({});
  });
});
