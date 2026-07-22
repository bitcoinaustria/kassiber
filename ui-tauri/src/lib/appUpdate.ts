import * as React from "react";

import { DAEMON_MODE, openExternalUrl } from "@/daemon/transport";
import i18n from "@/i18n";
import { useUiStore } from "@/store/ui";

export const APP_UPDATE_START_DELAY_MS = 10_000;
export const APP_UPDATE_PERIOD_MS = 24 * 60 * 60 * 1_000;

export interface AppUpdateCheck {
  currentVersion: string;
  latestVersion: string | null;
  releaseUrl: string | null;
  updateAvailable: boolean;
  prerelease: boolean;
  checkedAt: number;
}

export function canCheckAppUpdates(): boolean {
  return DAEMON_MODE === "tauri";
}

export async function checkForAppUpdate(): Promise<AppUpdateCheck> {
  if (!canCheckAppUpdates()) {
    throw new Error("Update checks are available in the desktop app.");
  }
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<AppUpdateCheck>("check_app_update");
}

async function persistAppUpdateChecksEnabled(enabled: boolean): Promise<void> {
  if (!canCheckAppUpdates()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke<boolean>("set_app_update_checks_enabled", { enabled });
}

/**
 * Persist the global consent before exposing the new state to schedulers. The
 * native command and packaged CLI read the same owner-only preference file.
 */
export async function setAppUpdateChecksEnabled(
  enabled: boolean,
): Promise<void> {
  await persistAppUpdateChecksEnabled(enabled);
  const store = useUiStore.getState();
  store.setAutomaticUpdateChecks(enabled);
  if (!enabled) store.setAppUpdate(null);
}

type ManualUpdateDialogOptions = {
  title: string;
  kind: "info" | "error";
  buttons: { ok: string } | { ok: string; cancel: string };
};

export interface ManualAppUpdateDeps {
  isEnabled: () => boolean;
  check: () => Promise<AppUpdateCheck>;
  setUpdate: (update: AppUpdateCheck) => void;
  showDialog: (
    body: string,
    options: ManualUpdateDialogOptions,
  ) => Promise<string>;
  openUrl: (url: string) => Promise<void>;
}

async function showNativeUpdateDialog(
  body: string,
  options: ManualUpdateDialogOptions,
): Promise<string> {
  const { message } = await import("@tauri-apps/plugin-dialog");
  return message(body, options);
}

/**
 * Explicit native-menu checks always report a result. Automatic checks stay
 * silent, but a user choosing "Check for Updates…" should never have to infer
 * whether the command ran. Downloads remain manual on the trusted GitHub page.
 */
export async function runManualAppUpdateCheck(
  overrides: Partial<ManualAppUpdateDeps> = {},
): Promise<void> {
  const deps: ManualAppUpdateDeps = {
    isEnabled: () => useUiStore.getState().automaticUpdateChecks,
    check: checkForAppUpdate,
    setUpdate: (update) => useUiStore.getState().setAppUpdate(update),
    showDialog: showNativeUpdateDialog,
    openUrl: openExternalUrl,
    ...overrides,
  };

  if (!deps.isEnabled()) {
    await deps
      .showDialog(
        i18n.t("shell.version.disabled", { ns: "chrome" }),
        {
          title: "Kassiber",
          kind: "info",
          buttons: {
            ok: i18n.t("shell.version.ok", { ns: "chrome" }),
          },
        },
      )
      .catch(() => undefined);
    return;
  }

  let result: AppUpdateCheck;
  try {
    result = await deps.check();
  } catch {
    await deps
      .showDialog(
        i18n.t("shell.version.checkFailed", { ns: "chrome" }),
        {
          title: "Kassiber",
          kind: "error",
          buttons: {
            ok: i18n.t("shell.version.ok", { ns: "chrome" }),
          },
        },
      )
      .catch(() => undefined);
    return;
  }

  deps.setUpdate(result);
  if (result.updateAvailable && result.latestVersion && result.releaseUrl) {
    const openGitHub = i18n.t("shell.version.openGitHub", { ns: "chrome" });
    const notNow = i18n.t("shell.version.notNow", { ns: "chrome" });
    const response = await deps
      .showDialog(
        i18n.t("shell.version.availablePrompt", {
          ns: "chrome",
          version: result.latestVersion,
        }),
        {
          title: "Kassiber",
          kind: "info",
          buttons: { ok: openGitHub, cancel: notNow },
        },
      )
      .catch(() => notNow);
    if (response === openGitHub) {
      await deps.openUrl(result.releaseUrl).catch(() => undefined);
    }
    return;
  }

  await deps
    .showDialog(
      i18n.t("shell.version.current", {
        ns: "chrome",
        version: result.currentVersion,
      }),
      {
        title: "Kassiber",
        kind: "info",
        buttons: {
          ok: i18n.t("shell.version.ok", { ns: "chrome" }),
        },
      },
    )
    .catch(() => undefined);
}

/**
 * Sparrow-style release notifier: wait briefly after launch, then make one
 * small check per day. Errors stay quiet because update availability must
 * never block startup or normal accounting work.
 */
export function startAppUpdateScheduler(
  check: () => Promise<AppUpdateCheck>,
  setUpdate: (update: AppUpdateCheck) => void,
): () => void {
  let disposed = false;
  let periodId: ReturnType<typeof globalThis.setInterval> | undefined;
  const run = async () => {
    try {
      const result = await check();
      if (!disposed) setUpdate(result);
    } catch {
      // A release check is advisory; failures never interrupt the app.
    }
  };
  const startId = globalThis.setTimeout(() => {
    void run();
    periodId = globalThis.setInterval(() => void run(), APP_UPDATE_PERIOD_MS);
  }, APP_UPDATE_START_DELAY_MS);

  return () => {
    disposed = true;
    globalThis.clearTimeout(startId);
    if (periodId !== undefined) globalThis.clearInterval(periodId);
  };
}

export function useAppUpdateScheduler(): void {
  const enabled = useUiStore((state) => state.automaticUpdateChecks);
  const identity = useUiStore((state) => state.identity);
  const setAppUpdate = useUiStore((state) => state.setAppUpdate);

  React.useEffect(() => {
    if (!identity || !canCheckAppUpdates()) return;
    let disposed = false;
    let stopScheduler: (() => void) | undefined;

    // Existing installs used renderer-local persistence. Mirror that value to
    // the global native/CLI consent before any new scheduler can contact GitHub.
    void persistAppUpdateChecksEnabled(enabled)
      .then(() => {
        if (!disposed && enabled) {
          stopScheduler = startAppUpdateScheduler(
            checkForAppUpdate,
            setAppUpdate,
          );
        }
      })
      .catch(() => {
        // Fail closed: the native command also refuses checks when consent is
        // absent or malformed, so persistence failure cannot create traffic.
      });

    return () => {
      disposed = true;
      stopScheduler?.();
    };
  }, [enabled, identity, setAppUpdate]);
}
