import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import { conditionKey, useNotificationStore } from "@/store/notifications";

/**
 * Saved unlock methods (desktop Touch ID, the CLI's remembered unlock) as
 * center conditions. They belong to the app, not to one book, and their one
 * action opens the Touch ID row in Settings -> Security.
 *
 * - `touch-id.off`: Touch ID unlock was turned off because the Keychain did
 *   not keep the passphrase. Enabling Touch ID again resolves it.
 * - `unlock.credential-remains`: removing a saved unlock method did not
 *   finish, so it may still unlock these books. A removal that finishes
 *   resolves it.
 */
const APP_SCOPE = { book: null };
const SECURITY_ACTION = {
  type: "navigate",
  to: "/settings/security",
  anchor: "biometricUnlock",
} as const;

export function recordTouchIdOff(reason: string | undefined): void {
  useNotificationStore.getState().raiseCondition({
    code: "touch-id.off",
    scope: APP_SCOPE,
    severity: "warning",
    priority: "attention",
    action: SECURITY_ACTION,
    details: [{ message: safeMessage(reason) }],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "warning",
    title: t("notifications.condition.touchIdOff.title"),
    key: "touch-id.off",
  });
}

export function resolveTouchIdOff(): void {
  useNotificationStore
    .getState()
    .resolveCondition(conditionKey("touch-id.off", APP_SCOPE));
}

export function recordUnlockCredentialRemains(reasons: string[]): void {
  useNotificationStore.getState().raiseCondition({
    code: "unlock.credential-remains",
    scope: APP_SCOPE,
    severity: "warning",
    priority: "attention",
    action: SECURITY_ACTION,
    details: reasons.map((reason) => ({ message: safeMessage(reason) })),
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "warning",
    title: t("notifications.condition.unlockCredentialRemains.title"),
    key: "unlock.credential-remains",
  });
}

export function resolveUnlockCredentialRemains(): void {
  useNotificationStore
    .getState()
    .resolveCondition(conditionKey("unlock.credential-remains", APP_SCOPE));
}
