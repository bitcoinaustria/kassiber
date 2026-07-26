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

export async function readAppUpdateChecksEnabled(): Promise<boolean> {
  if (!canCheckAppUpdates()) return false;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<boolean>("get_app_update_checks_enabled");
}

/**
 * Read the canonical native consent and mirror it into the store, failing
 * closed. `read` stays injectable because that is how the fail-closed path is
 * tested — outside Tauri the real reader can only ever answer `false`.
 */
export async function syncAppUpdateChecksEnabled(
  setEnabled: (enabled: boolean) => void = (enabled) =>
    useUiStore.getState().setAutomaticUpdateChecks(enabled),
  read: () => Promise<boolean> = readAppUpdateChecksEnabled,
): Promise<boolean> {
  let enabled = false;
  try {
    enabled = (await read()) === true;
  } catch {
    // An unreadable preference is not consent.
  }
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
  buttons: { ok: string; cancel?: string };
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
  } catch (error) {
    // The native command and the CLI both return a specific, already
    // user-facing reason (disabled consent, network, oversized response).
    // Swallowing it left "try again later" as the only diagnosis available.
    const reason =
      typeof error === "string"
        ? error
        : error instanceof Error
          ? error.message
          : "";
    await deps
      .showDialog(
        reason.trim() || i18n.t("shell.version.checkFailed", { ns: "chrome" }),
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

  // Consent can be revoked from another CLI process while the native check is
  // in flight. Re-read the canonical file before exposing its result.
  if (!(await deps.isEnabled())) {
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
  isEnabled: () => boolean | Promise<boolean> = syncAppUpdateChecksEnabled,
): () => void {
  let disposed = false;
  let periodId: ReturnType<typeof globalThis.setInterval> | undefined;
  const run = async () => {
    try {
      // Mirror a CLI-side revocation before invoking the native checker. The
      // native gate fails closed too, but its rejection would otherwise skip
      // the post-check sync below and leave the renderer toggle stale.
      if (!(await isEnabled())) return;
      const result = await check();
      // Re-read after the request as well so a revocation that landed during
      // the check suppresses its result.
      const stillEnabled = await isEnabled();
      if (disposed || !stillEnabled) return;
      setUpdate(result);
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
    // Read once: the Privacy toggle updates the store itself, and every check
    // re-reads consent before using its result, so polling this file each
    // second only bought noticing a `kassiber update --disable-checks` run in
    // another terminal a little sooner — at 86,400 IPC round-trips a day.
    void syncAppUpdateChecksEnabled().then(() => {
      if (!disposed) setConsentLoaded(true);
    });

    return () => {
      disposed = true;
    };
  }, []);

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
