import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

import { useUiStore } from "@/store/ui";
import {
  dispatchMenuIntent,
  type NativeMenuPayload,
} from "./menuIntent";

const NATIVE_MENU_EVENT = "kassiber:intent";
const unexpectedWorkspaceAction = () => {
  console.error("RootIntentListener should not receive workspace actions");
};

/**
 * Listens for `kassiber:intent` events on the root layout — above the
 * workspace shell — so route navigation, settings panel deep links, and
 * sensitive-toggle still work before AppShell has mounted (Welcome screen
 * users, cold-start deep links into `/diagnostics` for support cases, etc.).
 *
 * Strictly handles `global` scope. Workspace-required actions (lock,
 * sync-all, process-journals) flow through AppShell's listener, which only
 * fires once an identity exists.
 */
export function RootIntentListener() {
  const navigate = useNavigate();

  React.useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let disposed = false;
    let unlisten: (() => void) | null = null;

    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<NativeMenuPayload>(NATIVE_MENU_EVENT, (event) => {
          const store = useUiStore.getState();
          dispatchMenuIntent(
            event.payload,
            {
              hasWorkspace: store.identity !== null,
              aiFeaturesEnabled: store.aiFeaturesEnabled,
              hideSensitive: store.hideSensitive,
              navigate: ({ to, hash }) => {
                void navigate({ to, hash: hash ?? undefined });
              },
              // Workspace-required deps are unused at this scope — the
              // dispatcher's scope filter guarantees workspace actions are
              // dropped before they reach these. Log loudly if a future
              // scope-filter regression reaches this root listener.
              lockApp: unexpectedWorkspaceAction,
              setHideSensitive: store.setHideSensitive,
              decreaseAppScale: store.decreaseAppScale,
              increaseAppScale: store.increaseAppScale,
              resetAppScale: store.resetAppScale,
              runWalletSync: unexpectedWorkspaceAction,
              runJournalProcessing: unexpectedWorkspaceAction,
              addNotification: store.addNotification,
              emitSettingsSection: (section) => {
                window.dispatchEvent(
                  new CustomEvent("kassiber:settings-section", {
                    detail: { section },
                  }),
                );
              },
            },
            "global",
          );
        }),
      )
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
          return;
        }
        unlisten = nextUnlisten;
      })
      .catch((error) => {
        console.warn(
          "Could not attach root Kassiber native menu listener",
          error,
        );
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [navigate]);

  return null;
}
