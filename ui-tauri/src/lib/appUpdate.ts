import * as React from "react";

import { DAEMON_MODE, openExternalUrl } from "@/daemon/transport";
import i18n from "@/i18n";
import { useUiStore } from "@/store/ui";

export const APP_UPDATE_START_DELAY_MS = 10_000;
export const APP_UPDATE_PERIOD_MS = 24 * 60 * 60 * 1_000;
export const APP_UPDATE_CONSENT_REFRESH_MS = 30_000;

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

export async function readAppUpdateChecksEnabled(): Promise<boolean> {
  if (!canCheckAppUpdates()) return false;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<boolean>("get_app_update_checks_enabled");
}

export async function resolveAppUpdateChecksEnabled(
  read: () => Promise<boolean> = readAppUpdateChecksEnabled,
): Promise<boolean> {
  try {
    return (await read()) === true;
  } catch {
    return false;
  }
}

export async function syncAppUpdateChecksEnabled(
  setEnabled: (enabled: boolean) => void = (enabled) =>
    useUiStore.getState().setAutomaticUpdateChecks(enabled),
  read: () => Promise<boolean> = readAppUpdateChecksEnabled,
): Promise<boolean> {
  const enabled = await resolveAppUpdateChecksEnabled(read);
  setEnabled(enabled);
  return enabled;
}

/**
 * Persist the global consent before exposing the new state to schedulers. The
 * native command and packaged CLI read the same owner-only preference file.
 */
export async function setAppUpdateChecksEnabled(
  enabled: boolean,
): Promise<void> {
  await persistAppUpdateChecksEnabled(enabled);
  useUiStore.getState().setAutomaticUpdateChecks(enabled);
}

type ManualUpdateDialogOptions = {
  title: string;
  kind: "info" | "error";
  buttons: { ok: string } | { ok: string; cancel: string };
};

export interface ManualAppUpdateDeps {
  isEnabled: () => boolean | Promise<boolean>;
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
    isEnabled: syncAppUpdateChecksEnabled,
    check: checkForAppUpdate,
    setUpdate: (update) => useUiStore.getState().setAppUpdate(update),
    showDialog: showNativeUpdateDialog,
    openUrl: openExternalUrl,
    ...overrides,
  };

  if (!(await deps.isEnabled())) {
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
  const hasIdentity = useUiStore((state) => state.identity !== null);
  const setAutomaticUpdateChecks = useUiStore(
    (state) => state.setAutomaticUpdateChecks,
  );
  const setAppUpdate = useUiStore((state) => state.setAppUpdate);
  const [consentLoaded, setConsentLoaded] = React.useState(
    () => !canCheckAppUpdates(),
  );

  React.useEffect(() => {
    if (!canCheckAppUpdates()) return;
    let disposed = false;

    // The owner-only native/CLI preference is canonical. Renderer persistence
    // is deliberately ignored so imports, upgrades, malformed files, and CLI
    // changes cannot be converted into consent by merely starting the app.
    // Refreshing this local file also keeps a running desktop synchronized
    // when the user changes consent through the CLI.
    const refreshConsent = async () => {
      const canonicalEnabled = await resolveAppUpdateChecksEnabled();
      if (disposed) return;
      setAutomaticUpdateChecks(canonicalEnabled);
      setConsentLoaded(true);
    };
    void refreshConsent();
    const refreshId = globalThis.setInterval(
      () => void refreshConsent(),
      APP_UPDATE_CONSENT_REFRESH_MS,
    );

    return () => {
      disposed = true;
      globalThis.clearInterval(refreshId);
    };
  }, [setAutomaticUpdateChecks]);

  React.useEffect(() => {
    if (
      !consentLoaded ||
      !enabled ||
      !hasIdentity ||
      !canCheckAppUpdates()
    ) {
      return;
    }
    return startAppUpdateScheduler(checkForAppUpdate, setAppUpdate);
  }, [consentLoaded, enabled, hasIdentity, setAppUpdate]);
}
