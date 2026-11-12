import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

import {
  candidateForOccurrence,
  configureOsNotifications,
  liveOsNotificationDeps,
  offerOsNotification,
  OS_NOTIFICATION_ACTIVATED_EVENT,
  recordOsNotificationActivation,
  resolveActivation,
  syncOsNotificationPreferences,
  useOsNotificationActivation,
} from "@/lib/osNotifications";
import { safeTauriUnlisten } from "@/lib/tauriUnlisten";
import { onNewOccurrence, useNotificationStore } from "@/store/notifications";
import { bookIdentityKey, useUiStore } from "@/store/ui";

/**
 * The shell's side of OS notifications: offers new center occurrences to
 * the OS (when the user opted in), keeps the native side's opt-in and
 * language in step, and routes an activated notification once the shell is
 * unlocked. A click never syncs, approves, opens a file, or unlocks.
 */
export function useOsNotifications(shellUnlocked: boolean): void {
  const navigate = useNavigate();
  const enabled = useUiStore((s) => s.desktopNotifications);
  const lang = useUiStore((s) => s.lang);
  const pending = useOsNotificationActivation((s) => s.pending);

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

  React.useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<{ token?: unknown }>(OS_NOTIFICATION_ACTIVATED_EVENT, (event) => {
          if (disposed) return;
          recordOsNotificationActivation(event.payload?.token);
        }),
      )
      .then((next) => {
        if (disposed) {
          safeTauriUnlisten(next);
          return;
        }
        unlisten = next;
      })
      .catch((error) => {
        console.warn("Could not attach OS notification listener", error);
      });
    return () => {
      disposed = true;
      safeTauriUnlisten(unlisten);
    };
  }, []);

  // A locked shell shows its lock screen; the destination waits in RAM.
  React.useEffect(() => {
    if (!shellUnlocked || !pending) return;
    useOsNotificationActivation.setState({ pending: null });
    const ui = useUiStore.getState();
    const notifications = useNotificationStore.getState();
    const route = resolveActivation(pending, {
      currentBook: bookIdentityKey(ui.identity),
      developerToolsEnabled: ui.developerToolsEnabled,
      conditions: notifications.conditions,
      results: notifications.results,
    });
    if (route.kind === "navigate") {
      void navigate({ to: route.to });
      return;
    }
    notifications.requestOpenCenter();
  }, [navigate, pending, shellUnlocked]);
}
