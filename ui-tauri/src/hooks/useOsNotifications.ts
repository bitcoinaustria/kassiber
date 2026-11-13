import * as React from "react";

import {
  candidateForOccurrence,
  configureOsNotifications,
  liveOsNotificationDeps,
  offerOsNotification,
  syncOsNotificationPreferences,
} from "@/lib/osNotifications";
import { onNewOccurrence } from "@/store/notifications";
import { useUiStore } from "@/store/ui";

/**
 * The shell's side of OS notifications: offers new center occurrences to
 * the OS (when the user opted in) and keeps the native side's opt-in and
 * language in step. A click only brings Kassiber to the front; the bell
 * shows what happened.
 */
export function useOsNotifications(): void {
  const enabled = useUiStore((s) => s.desktopNotifications);
  const lang = useUiStore((s) => s.lang);

  React.useEffect(() => {
    configureOsNotifications(
      liveOsNotificationDeps(() => useUiStore.getState().desktopNotifications),
    );
    // Only genuinely new occurrences: never a reconcile or a rebuild.
    return onNewOccurrence((occurrence) => {
      void offerOsNotification(candidateForOccurrence(occurrence));
    });
  }, []);

  React.useEffect(() => {
    void syncOsNotificationPreferences(enabled, lang);
  }, [enabled, lang]);
}
