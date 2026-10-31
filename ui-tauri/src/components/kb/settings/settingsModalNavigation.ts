import * as React from "react";
import { useRouter } from "@tanstack/react-router";

import { useUiStore } from "@/store/ui";

import { isDevOnlySettingsSection } from "../devMode";
import {
  DEFAULT_SETTINGS_SECTION_ID,
  settingsSectionForHash,
  settingsSectionForPathname,
  type SettingsSectionId,
} from "../settingsSections";

/**
 * Settings opens as a large modal over the page that asked for it, after
 * T3 Code. Its categories keep their `/settings/<slug>` routes so links,
 * the native menu, search results and deep links stay typed and unchanged;
 * a navigation to one opens (or switches) the modal instead of leaving the
 * page, and adds no history entry.
 */

/**
 * The category a `/settings` location names: its `/settings/<slug>` path, a
 * bare `/settings` with a legacy `#slug`, or the default. Null for any
 * location outside Settings.
 */
export function settingsSectionForLocation(
  pathname: string,
  hash: string | null | undefined,
): SettingsSectionId | null {
  const trimmed = pathname.replace(/\/+$/, "");
  if (trimmed === "/settings") {
    return (hash ? settingsSectionForHash(hash) : null) ?? DEFAULT_SETTINGS_SECTION_ID;
  }
  if (!trimmed.startsWith("/settings/")) return null;
  return settingsSectionForPathname(trimmed) ?? DEFAULT_SETTINGS_SECTION_ID;
}

/**
 * Opens the modal on `section`. A developer-only category stays shut while
 * developer tools are off, whichever way it was asked for: the hidden nav row
 * is a signpost, not the barrier. Returns whether the modal opened.
 */
export function openSettingsSection(section: SettingsSectionId): boolean {
  const state = useUiStore.getState();
  if (!state.developerToolsEnabled && isDevOnlySettingsSection(section)) {
    return false;
  }
  state.setSettingsSection(section);
  return true;
}

export function closeSettings() {
  useUiStore.getState().setSettingsSection(null);
}

/**
 * Turns every in-app navigation to a Settings route into the modal. Mounted
 * once by the app shell; the routes themselves only handle a cold load.
 * Behind the lock screen a request for Settings (a notification, the native
 * menu, a deep link) is dropped rather than kept to pop up after unlock.
 */
export function useSettingsModalNavigation(unlocked: boolean) {
  const router = useRouter();
  const unlockedRef = React.useRef(unlocked);
  React.useLayoutEffect(() => {
    unlockedRef.current = unlocked;
  });
  React.useLayoutEffect(
    () =>
      router.history.block({
        enableBeforeUnload: false,
        blockerFn: ({ nextLocation }) => {
          const section = settingsSectionForLocation(
            nextLocation.pathname,
            nextLocation.hash,
          );
          if (!section) return false;
          if (unlockedRef.current) openSettingsSection(section);
          return true;
        },
      }),
    [router],
  );
}
