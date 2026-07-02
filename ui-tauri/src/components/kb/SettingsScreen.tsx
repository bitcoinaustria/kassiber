/**
 * SettingsScreen - workspace-wide preferences.
 *
 * Most controls are local UI state until the daemon-backed settings surface
 * lands. Hide-sensitive data is wired to the shared UI store.
 */
import * as React from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  canResetRegtestDemo,
  canUseTouchIdPassphraseUnlock,
  clearImportProject,
  forgetTouchIdPassphrase,
  getTransport,
  resetRegtestDemo,
  installTerminalCommand,
  removeTerminalCommand,
  storeTouchIdPassphrase,
  touchIdPassphraseStatus,
  terminalCommandStatus,
  type TerminalCommandStatus,
  type TouchIdPassphraseStatus,
} from "@/daemon/transport";
import { screenPanelClassName } from "@/lib/screen-layout";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";
import { useUiStore } from "@/store/ui";
import { databasePassphraseHint } from "@/components/kb/Onboarding/constants";
import {
  PENDING_SETTINGS_BACKEND_EDIT_KEY,
  settingsSectionForHash,
  type SettingsSectionId,
} from "./settingsSections";
import { AppearanceSettingsPanel } from "./settings/AppearanceSettingsPanel";
import { AiProvidersSettingsPanel } from "./settings/AiProvidersSettingsPanel";
import { SyncBackendSettingsModal, type SyncBackendNetwork } from "./settings/SyncBackendSettingsModal";
import { backendTypeIdForConnectionSetup } from "./settings/SyncBackendSettingsModel";
import { DataStorageSettingsPanel } from "./settings/DataStorageSettingsPanel";
import { DeveloperToolsSettingsPanel } from "./settings/DeveloperToolsSettingsPanel";
import { MarketDataSettingsPanel } from "./settings/MarketDataSettingsPanel";
import { NetworkLayerSettingsPanel } from "./settings/NetworkLayerSettingsPanel";
import { PrivacySettingsPanel } from "./settings/PrivacySettingsPanel";
import { SecuritySettingsPanel } from "./settings/SecuritySettingsPanel";
import { TerminalCommandSettingsPanel } from "./settings/TerminalCommandSettingsPanel";
import { DEFAULT_SETTINGS_SECTION, SettingsRail, sectionMeta } from "./settings/SettingsNavigation";
import {
  PLAINTEXT_DELETE_ACK,
  backendPayload,
  backendRowToSettingsBackend,
  backendsForLayer,
  deriveExplorerSettings,
  formatCount,
  marketRateBackends,
  type Backend,
  type BackendSettingsData,
  type BackendSettingsRow,
  type MaintenanceSettingsData,
  type ResetBookData,
  type StatusData,
} from "./settings/SettingsModel";

interface SettingsScreenProps {
  onLock?: () => void;
}

export function SettingsScreen({ onLock }: SettingsScreenProps) {
  const { t } = useTranslation(["settings", "common"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const clearClipboard = useUiStore((s) => s.clearClipboard);
  const setClearClipboard = useUiStore((s) => s.setClearClipboard);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);
  const lang = useUiStore((s) => s.lang);
  const setLang = useUiStore((s) => s.setLang);
  const theme = useUiStore((s) => s.theme);
  const setTheme = useUiStore((s) => s.setTheme);
  const appScale = useUiStore((s) => s.appScale);
  const increaseAppScale = useUiStore((s) => s.increaseAppScale);
  const decreaseAppScale = useUiStore((s) => s.decreaseAppScale);
  const resetAppScale = useUiStore((s) => s.resetAppScale);
  const setExplorerSettings = useUiStore((s) => s.setExplorerSettings);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setAppLockPolicy = useUiStore((s) => s.setAppLockPolicy);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const setAiFeaturesEnabled = useUiStore((s) => s.setAiFeaturesEnabled);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const setDeveloperToolsEnabled = useUiStore(
    (s) => s.setDeveloperToolsEnabled,
  );
  const identity = useUiStore((s) => s.identity);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const addNotification = useUiStore((s) => s.addNotification);
  const dataMode = useUiStore((s) => s.dataMode);
  const deferredConnectionSetup = useUiStore(
    (s) => s.deferredConnectionSetup,
  );
  const clearDeferredConnectionSetup = useUiStore(
    (s) => s.clearDeferredConnectionSetup,
  );
  const navigate = useNavigate();
  const settingsHash = useRouterState({ select: (s) => s.location.hash });
  const routeSectionId = React.useMemo(
    () => settingsSectionForHash(settingsHash),
    [settingsHash],
  );
  const statusQuery = useDaemon<StatusData>("status", undefined, {
    enabled: true,
  });
  const status =
    statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const statusLoaded = statusQuery.data?.kind === "status";
  const touchIdDataRoot = identity?.importedProject?.dataRoot ?? null;
  const touchIdPlatformSupported = canUseTouchIdPassphraseUnlock();
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const [touchIdStatus, setTouchIdStatus] =
    React.useState<TouchIdPassphraseStatus | null>(null);
  const [touchIdStatusPending, setTouchIdStatusPending] =
    React.useState(false);
  const touchIdConfigured = touchIdStatus?.configured === true;
  const touchIdStatusReason = touchIdStatus?.reason ?? null;
  const deleteWorkspace = useDaemonMutation("ui.workspace.delete", {
    dataMode: "real",
  });
  const resetBookData = useDaemonMutation<ResetBookData>("ui.profiles.reset_data", {
    dataMode: "real",
  });
  const changePassphrase = useDaemonMutation("ui.secrets.change_passphrase", {
    dataMode: "real",
  });
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    { refetchOnMount: "always" },
  );
  const maintenanceSettingsQuery = useDaemon<MaintenanceSettingsData>(
    "ui.maintenance.settings",
    undefined,
    { refetchOnMount: "always" },
  );
  const createBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.create");
  const updateBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.update");
  const setDefaultBackend = useDaemonMutation<{ default_backend: string }>(
    "ui.backends.set_default",
  );
  const createWallet = useDaemonMutation("ui.wallets.create");
  const deleteBackend = useDaemonMutation<{
    name: string;
    deleted: boolean;
    detached_wallet_refs?: string[];
  }>("ui.backends.delete");
  const [settingDefaultBackendId, setSettingDefaultBackendId] =
    React.useState<string | null>(null);
  const [backendDialogOpen, setBackendDialogOpen] = React.useState(false);
  const [editingBackendId, setEditingBackendId] = React.useState<string | null>(
    null,
  );
  const [initialBackendTypeId, setInitialBackendTypeId] =
    React.useState<SyncBackendNetwork["id"] | null>(null);
  const deferredBackendDialogKeyRef = React.useRef<string | null>(null);
  const [deleteOpen, setDeleteOpen] = React.useState(false);
  const [deletePassphrase, setDeletePassphrase] = React.useState("");
  const [deleteConfirm, setDeleteConfirm] = React.useState("");
  const [deletePlaintextAck, setDeletePlaintextAck] = React.useState("");
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  const [resetDataOpen, setResetDataOpen] = React.useState(false);
  const [resetDataPassphrase, setResetDataPassphrase] = React.useState("");
  const [resetDataConfirm, setResetDataConfirm] = React.useState("");
  const [resetDataClearSharedRates, setResetDataClearSharedRates] =
    React.useState(false);
  const [resetDataPlaintextAck, setResetDataPlaintextAck] =
    React.useState("");
  const [resetDataError, setResetDataError] = React.useState<string | null>(
    null,
  );
  const [passphraseOpen, setPassphraseOpen] = React.useState(false);
  const [currentPassphrase, setCurrentPassphrase] = React.useState("");
  const [newPassphrase, setNewPassphrase] = React.useState("");
  const [newPassphraseConfirm, setNewPassphraseConfirm] = React.useState("");
  const [passphraseError, setPassphraseError] = React.useState<string | null>(
    null,
  );
  const [touchIdEnrollOpen, setTouchIdEnrollOpen] = React.useState(false);
  const [touchIdEnrollPassphrase, setTouchIdEnrollPassphrase] =
    React.useState("");
  const [touchIdEnrollError, setTouchIdEnrollError] = React.useState<
    string | null
  >(null);
  const [touchIdEnrollPending, setTouchIdEnrollPending] =
    React.useState(false);
  const [terminalStatus, setTerminalStatus] =
    React.useState<TerminalCommandStatus | null>(null);
  const [terminalStatusError, setTerminalStatusError] = React.useState<
    string | null
  >(null);
  const [terminalCommandPending, setTerminalCommandPending] =
    React.useState(false);
  const [activeSectionId, setActiveSectionId] = React.useState<SettingsSectionId>(
    () => routeSectionId ?? DEFAULT_SETTINGS_SECTION,
  );
  const [pendingBackendEditId, setPendingBackendEditId] = React.useState<
    string | null
  >(() =>
    typeof window === "undefined"
      ? null
      : window.sessionStorage.getItem(PENDING_SETTINGS_BACKEND_EDIT_KEY),
  );

  const openTouchIdEnrollment = React.useCallback(() => {
    setTouchIdEnrollError(null);
    setTouchIdEnrollPassphrase("");
    setTouchIdEnrollOpen(true);
  }, []);

  const refreshTouchIdStatus = React.useCallback(async () => {
    if (!encryptedWorkspace || !touchIdPlatformSupported) {
      setTouchIdStatus(null);
      return null;
    }
    setTouchIdStatusPending(true);
    try {
      const status = await touchIdPassphraseStatus(touchIdDataRoot);
      setTouchIdStatus(status);
      return status;
    } catch (error) {
      const status: TouchIdPassphraseStatus = {
        platform: "macos",
        available: false,
        configured: false,
        reason: error instanceof Error ? error.message : String(error),
      };
      setTouchIdStatus(status);
      return status;
    } finally {
      setTouchIdStatusPending(false);
    }
  }, [encryptedWorkspace, touchIdDataRoot, touchIdPlatformSupported]);

  const forgetTouchIdUnlock = React.useCallback(
    async () => {
      try {
        const status = await forgetTouchIdPassphrase(touchIdDataRoot);
        setTouchIdStatus(status);
        setAppLockPolicy({ touchIdUnlock: false });
      } catch (error: unknown) {
        addNotification({
          title: t("touchId.removedTitle"),
          body:
            error instanceof Error
              ? error.message
              : t("touchId.removedBody"),
          tone: "warning",
        });
      }
    },
    [addNotification, setAppLockPolicy, t, touchIdDataRoot],
  );

  React.useEffect(() => {
    void refreshTouchIdStatus();
  }, [refreshTouchIdStatus]);

  React.useEffect(() => {
    if (!appLockPolicy.touchIdUnlock) return;
    if (touchIdStatus?.configured !== false) return;
    setAppLockPolicy({ touchIdUnlock: false });
  }, [appLockPolicy.touchIdUnlock, setAppLockPolicy, touchIdStatus?.configured]);

  React.useEffect(() => {
    setActiveSectionId(routeSectionId ?? DEFAULT_SETTINGS_SECTION);
  }, [routeSectionId]);

  const refreshTerminalCommandStatus = React.useCallback(async () => {
    try {
      const next = await terminalCommandStatus();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      return next;
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : t("terminal.inspectError");
      setTerminalStatusError(message);
      return null;
    }
  }, [t]);

  React.useEffect(() => {
    void refreshTerminalCommandStatus();
  }, [refreshTerminalCommandStatus]);

  // Native menu may re-fire for the same section while the URL hash is
  // unchanged (user already on /settings#privacy, clicks Privacy again after
  // closing the panel). The hash effect won't see a diff, so listen for an
  // explicit `kassiber:settings-section` event and force re-selection.
  React.useEffect(() => {
    const handler = (event: Event) => {
      const detail = (
        event as CustomEvent<{
          section?: string | null;
          backendId?: string | null;
        }>
      ).detail;
      const next = settingsSectionForHash(detail?.section ?? "");
      if (next) setActiveSectionId(next);
      if (detail?.backendId) {
        setPendingBackendEditId(detail.backendId);
      }
    };
    window.addEventListener("kassiber:settings-section", handler);
    return () => {
      window.removeEventListener("kassiber:settings-section", handler);
    };
  }, []);

  const backends = React.useMemo<Backend[]>(() => {
    const syncRows = backendSettingsQuery.data?.data?.backends ?? [];
    const syncBackends = syncRows.map(backendRowToSettingsBackend);
    const hasFxRows = syncBackends.some((backend) => backend.net === "FX");
    if (hasFxRows) return syncBackends;
    return [
      ...syncBackends,
      ...marketRateBackends(
        maintenanceSettingsQuery.data?.data?.settings ?? null,
        syncBackends,
      ),
    ];
  }, [backendSettingsQuery.data, maintenanceSettingsQuery.data]);

  React.useEffect(() => {
    if (!backendSettingsQuery.data?.data?.backends) return;
    setExplorerSettings(deriveExplorerSettings(backends));
  }, [backendSettingsQuery.data, backends, setExplorerSettings]);

  const editingBackend = React.useMemo(
    () => backends.find((backend) => backend.id === editingBackendId) ?? null,
    [backends, editingBackendId],
  );

  React.useEffect(() => {
    if (!pendingBackendEditId) return;
    const backend = backends.find((candidate) => candidate.id === pendingBackendEditId);
    if (!backend) {
      if (!backendSettingsQuery.isFetched) return;
      window.sessionStorage.removeItem(PENDING_SETTINGS_BACKEND_EDIT_KEY);
      setPendingBackendEditId(null);
      return;
    }
    setEditingBackendId(backend.id);
    setInitialBackendTypeId(null);
    setBackendDialogOpen(true);
    window.sessionStorage.removeItem(PENDING_SETTINGS_BACKEND_EDIT_KEY);
    setPendingBackendEditId(null);
  }, [backendSettingsQuery.isFetched, backends, pendingBackendEditId]);

  const onResetWorkspace = () => {
    const ok = window.confirm(t("resetWelcome.confirm"));
    if (!ok) return;
    void (async () => {
      if (identity?.importedProject) {
        await clearImportProject();
      }
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    })().catch(() => {
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    });
  };

  const lockNow = () => {
    window.requestAnimationFrame(() => {
      if (onLock) {
        onLock();
        return;
      }
      window.dispatchEvent(new CustomEvent("kassiber:lock-app"));
    });
  };

  const workspaceLabel =
    status?.current_workspace || identity?.workspace || t("workspace.fallback");
  const currentBookLabel =
    statusLoaded
      ? status?.current_profile ?? null
      : identity?.profile || identity?.name || null;
  const bookLabel = currentBookLabel || t("workspace.bookFallback");
  const resetBookAvailable = Boolean(currentBookLabel);

  const openResetBookData = () => {
    setResetDataPassphrase("");
    setResetDataConfirm("");
    setResetDataClearSharedRates(false);
    setResetDataPlaintextAck("");
    setResetDataError(null);
    setResetDataOpen(true);
  };

  const openDeleteWorkspace = () => {
    setDeletePassphrase("");
    setDeleteConfirm("");
    setDeletePlaintextAck("");
    setDeleteError(null);
    setDeleteOpen(true);
  };

  const [resetRegtestPending, setResetRegtestPending] = React.useState(false);
  const regtestResetAvailable =
    dataMode === "regtest" && canResetRegtestDemo(status?.data_root ?? null);
  const onResetRegtestEnv = () => {
    if (resetRegtestPending) return;
    if (!window.confirm(t("data.resetRegtestConfirm"))) return;
    setResetRegtestPending(true);
    void resetRegtestDemo()
      .then(() => {
        addNotification({
          tone: "success",
          title: t("data.resetRegtestDoneTitle"),
          body: t("data.resetRegtestDoneBody"),
        });
      })
      .catch((error: unknown) => {
        addNotification({
          tone: "error",
          title: t("data.resetRegtestErrorTitle"),
          body: error instanceof Error ? error.message : t("data.resetRegtestErrorBody"),
        });
      })
      .finally(() => setResetRegtestPending(false));
  };

  const openChangePassphrase = () => {
    setCurrentPassphrase("");
    setNewPassphrase("");
    setNewPassphraseConfirm("");
    setPassphraseError(null);
    setPassphraseOpen(true);
  };

  const openAddBackend = React.useCallback((typeId?: SyncBackendNetwork["id"]) => {
    setEditingBackendId(null);
    setInitialBackendTypeId(typeId ?? null);
    setBackendDialogOpen(true);
  }, []);

  const openEditBackend = (backend: Backend) => {
    setEditingBackendId(backend.id);
    setInitialBackendTypeId(null);
    setBackendDialogOpen(true);
  };

  React.useEffect(() => {
    const typeId = backendTypeIdForConnectionSetup(deferredConnectionSetup);
    if (!typeId) return;
    const key = `${deferredConnectionSetup?.sourceId ?? ""}:${
      deferredConnectionSetup?.backendKind ?? ""
    }`;
    if (deferredBackendDialogKeyRef.current === key) return;
    deferredBackendDialogKeyRef.current = key;
    setActiveSectionId(
      typeId === "liquid"
        ? "network-liquid"
        : typeId === "coreln" || typeId === "lnd"
          ? "network-lightning"
          : "network-bitcoin",
    );
    openAddBackend(typeId);
  }, [deferredConnectionSetup, openAddBackend]);

  const onSaveBackend = async (backend: Backend) => {
    const payload = backendPayload(backend);
    if (editingBackend) {
      await updateBackend.mutateAsync({ ...payload, name: editingBackend.id });
    } else {
      await createBackend.mutateAsync(payload);
      if (backend.kind === "coreln") {
        await createWallet.mutateAsync({
          label: backend.name,
          kind: "coreln",
          backend: backend.name,
          chain: "bitcoin",
          network: "main",
        });
      }
    }
    const refreshed = await backendSettingsQuery.refetch();
    const refreshedBackends = (refreshed.data?.data?.backends ?? []).map(
      backendRowToSettingsBackend,
    );
    setExplorerSettings(deriveExplorerSettings(refreshedBackends));
    setBackendDialogOpen(false);
    setEditingBackendId(null);
    setInitialBackendTypeId(null);
  };

  const onDeleteBackend = async (backend: Backend) => {
    const affectedWallets = backend.walletRefs ?? [];
    const walletWarning =
      affectedWallets.length > 0
        ? `\n\n${t("deleteBackend.walletWarning", {
            wallets: affectedWallets.join("\n- "),
          })}`
        : "";
    const ok = window.confirm(
      `${t("deleteBackend.confirm", { name: backend.name })}${walletWarning}`,
    );
    if (!ok) return;
    await deleteBackend.mutateAsync({ name: backend.id });
    const refreshed = await backendSettingsQuery.refetch();
    const refreshedBackends = (refreshed.data?.data?.backends ?? []).map(
      backendRowToSettingsBackend,
    );
    setExplorerSettings(deriveExplorerSettings(refreshedBackends));
    setBackendDialogOpen(false);
    setEditingBackendId(null);
    setInitialBackendTypeId(null);
  };

  const onSetDefaultBackend = async (backend: Backend) => {
    setSettingDefaultBackendId(backend.id);
    try {
      await setDefaultBackend.mutateAsync({ name: backend.id });
      const refreshed = await backendSettingsQuery.refetch();
      const refreshedBackends = (refreshed.data?.data?.backends ?? []).map(
        backendRowToSettingsBackend,
      );
      setExplorerSettings(deriveExplorerSettings(refreshedBackends));
      addNotification({
        title: t("defaultBackend.updatedTitle"),
        body: t("defaultBackend.updatedBody", { name: backend.name }),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("defaultBackend.failedTitle"),
        body:
          error instanceof Error
            ? error.message
            : t("defaultBackend.failedBody"),
        tone: "warning",
      });
    } finally {
      setSettingDefaultBackendId(null);
    }
  };

  const onInstallTerminalCommand = async () => {
    const wasRepair = Boolean(terminalStatus?.needsRepair);
    setTerminalCommandPending(true);
    try {
      const next = await installTerminalCommand();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      addNotification({
        title: wasRepair
          ? t("terminal.repairedTitle")
          : t("terminal.installedTitle"),
        body: next.pathOnPath
          ? t("terminal.installedBodyOnPath")
          : t("terminal.installedBodyOffPath", { binDir: next.binDir }),
        tone: next.pathOnPath ? "success" : "warning",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : t("terminal.installError"),
      );
    } finally {
      setTerminalCommandPending(false);
    }
  };

  const onRemoveTerminalCommand = async () => {
    setTerminalCommandPending(true);
    try {
      const next = await removeTerminalCommand();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      addNotification({
        title: t("terminal.removedTitle"),
        body: t("terminal.removedBody", {
          command: next.commandPath || t("terminal.removedBodyFallback"),
        }),
        tone: "success",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : t("terminal.removeError"),
      );
    } finally {
      setTerminalCommandPending(false);
    }
  };

  const goToSection = React.useCallback(
    (id: SettingsSectionId) => {
      setActiveSectionId(id);
      void navigate({
        to: "/settings",
        hash: sectionMeta(id).slug,
        replace: true,
      });
    },
    [navigate],
  );

  const sectionCounts = React.useMemo<
    Partial<Record<SettingsSectionId, number>>
  >(
    () => ({
      "network-bitcoin": backendsForLayer(backends, "bitcoin").length,
      "network-lightning": backendsForLayer(backends, "lightning").length,
      "network-liquid": backendsForLayer(backends, "liquid").length,
      "network-market": backends.filter((backend) => backend.net === "FX")
        .length,
    }),
    [backends],
  );

  const onDeleteWorkspace = async () => {
    setDeleteError(null);
    if (encryptedWorkspace && !deletePassphrase) {
      setDeleteError(t("deleteBooks.errorPassphrase"));
      return;
    }
    if (
      !encryptedWorkspace &&
      deletePlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setDeleteError(
        t("deleteBooks.errorPlaintext", { challenge: PLAINTEXT_DELETE_ACK }),
      );
      return;
    }
    if (deleteConfirm.trim() !== workspaceLabel) {
      setDeleteError(
        t("deleteBooks.errorConfirm", { workspace: workspaceLabel }),
      );
      return;
    }
    try {
      await deleteWorkspace.mutateAsync({
        confirm: "DELETE",
        confirm_workspace: workspaceLabel,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: deletePassphrase }
          : { plaintext_delete_ack: PLAINTEXT_DELETE_ACK },
      });
      if (identity?.importedProject) {
        await clearImportProject().catch(() => {});
      }
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    } catch (error) {
      window.alert(
        error instanceof Error
          ? error.message
          : t("deleteBooks.errorGeneric"),
      );
    }
  };

  const onResetBookData = async () => {
    setResetDataError(null);
    if (encryptedWorkspace && !resetDataPassphrase) {
      setResetDataError(t("resetBook.errorPassphrase"));
      return;
    }
    if (
      !encryptedWorkspace &&
      resetDataPlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setResetDataError(
        t("resetBook.errorPlaintext", { challenge: PLAINTEXT_DELETE_ACK }),
      );
      return;
    }
    if (resetDataConfirm.trim() !== bookLabel) {
      setResetDataError(t("resetBook.errorConfirm", { book: bookLabel }));
      return;
    }
    try {
      const envelope = await resetBookData.mutateAsync({
        confirm: "RESET",
        confirm_profile: bookLabel,
        clear_shared_rates: resetDataClearSharedRates,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: resetDataPassphrase }
          : { plaintext_delete_ack: PLAINTEXT_DELETE_ACK },
      });
      const removed = envelope.data?.removed ?? {};
      const optionalSummary: string[] = [];
      const attachmentsRemoved =
        Number(removed.attachments ?? 0) +
        Number(removed.attachment_files ?? 0);
      const sourceFundsRemoved =
        Number(removed.source_funds_sources ?? 0) +
        Number(removed.source_funds_links ?? 0) +
        Number(removed.source_funds_cases ?? 0) +
        Number(removed.source_funds_snapshots ?? 0);
      const summary = [
        t("resetBook.summary.transactions", {
          value: formatCount(removed.transactions ?? 0),
        }),
        t("resetBook.summary.journalRows", {
          value: formatCount(removed.journal_entries ?? 0),
        }),
        t("resetBook.summary.swapPairs", {
          value: formatCount(removed.transaction_pairs ?? 0),
        }),
        t("resetBook.summary.labels", {
          value: formatCount(removed.bip329_labels ?? 0),
        }),
      ];
      if (attachmentsRemoved > 0) {
        optionalSummary.push(
          t("resetBook.summary.attachments", {
            value: formatCount(attachmentsRemoved),
          }),
        );
      }
      if (sourceFundsRemoved > 0) {
        optionalSummary.push(
          t("resetBook.summary.sourceFunds", {
            value: formatCount(sourceFundsRemoved),
          }),
        );
      }
      if (envelope.data?.shared_rates_cleared) {
        optionalSummary.push(
          t("resetBook.summary.rates", {
            value: formatCount(removed.rates_cache ?? 0),
          }),
        );
      }
      summary.push(...optionalSummary);
      addNotification({
        title: t("resetBook.notifyTitle"),
        body: t("resetBook.notifyBody", { summary: summary.join(", ") }),
        tone: "success",
      });
      setResetDataOpen(false);
      setResetDataPassphrase("");
      setResetDataConfirm("");
      setResetDataClearSharedRates(false);
      setResetDataPlaintextAck("");
    } catch (error) {
      setResetDataError(
        error instanceof Error ? error.message : t("resetBook.errorGeneric"),
      );
    }
  };

  const onChangePassphrase = async () => {
    setPassphraseError(null);
    if (!currentPassphrase) {
      setPassphraseError(t("changePassphrase.errorCurrent"));
      return;
    }
    const hint = databasePassphraseHint(newPassphrase, newPassphraseConfirm);
    if (hint) {
      setPassphraseError(hint);
      return;
    }

    try {
      await changePassphrase.mutateAsync({
        auth_response: { passphrase_secret: currentPassphrase },
        new_passphrase_secret: newPassphrase,
      });
      await setSessionUnlockPassphrase(newPassphrase);
      if (appLockPolicy.touchIdUnlock && touchIdPlatformSupported) {
        try {
          const status = await storeTouchIdPassphrase(
            newPassphrase,
            touchIdDataRoot,
          );
          setTouchIdStatus(status);
          if (!status.configured) {
            throw new Error(
              status.reason
                ? t("touchIdEnroll.errorNotSetUpReason", {
                    reason: status.reason,
                  })
                : t("touchIdEnroll.errorNotSetUp"),
            );
          }
        } catch (error) {
          setAppLockPolicy({ touchIdUnlock: false });
          await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
          await refreshTouchIdStatus();
          addNotification({
            title: t("touchId.disabledTitle"),
            body:
              error instanceof Error
                ? error.message
                : t("touchId.disabledBody"),
            tone: "warning",
          });
        }
      }
      setPassphraseOpen(false);
      setCurrentPassphrase("");
      setNewPassphrase("");
      setNewPassphraseConfirm("");
    } catch (error) {
      setPassphraseError(
        error instanceof Error
          ? error.message
          : t("changePassphrase.errorGeneric"),
      );
    }
  };

  const onEnrollTouchId = async () => {
    setTouchIdEnrollError(null);
    if (!touchIdEnrollPassphrase) {
      setTouchIdEnrollError(t("touchIdEnroll.errorPassphrase"));
      return;
    }

    setTouchIdEnrollPending(true);
    try {
      const envelope = await getTransport("real").invoke({
        kind: "daemon.unlock",
        args: {
          ...(identity?.importedProject
            ? { require_existing_project: true }
            : {}),
          auth_response: { passphrase_secret: touchIdEnrollPassphrase },
        },
      });
      if (envelope.kind !== "daemon.unlock") {
        throw new Error(t("touchIdEnroll.errorUnlock"));
      }
      const status = await storeTouchIdPassphrase(
        touchIdEnrollPassphrase,
        touchIdDataRoot,
      );
      setTouchIdStatus(status);
      if (!status.configured) {
        throw new Error(
          status.reason
            ? t("touchIdEnroll.errorNotSetUpReason", { reason: status.reason })
            : t("touchIdEnroll.errorNotSetUp"),
        );
      }
      await setSessionUnlockPassphrase(touchIdEnrollPassphrase);
      setAppLockPolicy({ touchIdUnlock: true });
      setTouchIdEnrollOpen(false);
      setTouchIdEnrollPassphrase("");
      addNotification({
        title: t("touchIdEnroll.notifyTitle"),
        body: t("touchIdEnroll.notifyBody"),
        tone: "success",
      });
    } catch (error) {
      setAppLockPolicy({ touchIdUnlock: false });
      await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
      await refreshTouchIdStatus();
      setTouchIdEnrollError(
        error instanceof Error
          ? error.message
          : t("touchIdEnroll.errorGeneric"),
      );
    } finally {
      setTouchIdEnrollPending(false);
    }
  };

  const activeMeta = sectionMeta(activeSectionId);
  const sectionContent = (() => {
    switch (activeSectionId) {
      case "general-appearance":
        return (
          <AppearanceSettingsPanel
            theme={theme}
            setTheme={setTheme}
            appScale={appScale}
            increaseAppScale={increaseAppScale}
            decreaseAppScale={decreaseAppScale}
            resetAppScale={resetAppScale}
            currency={currency}
            setCurrency={setCurrency}
            lang={lang}
            setLang={setLang}
          />
        );
      case "network-bitcoin":
        return (
          <NetworkLayerSettingsPanel
            layer="bitcoin"
            backends={backends}
            onAdd={() => openAddBackend("bitcoin")}
            onEdit={openEditBackend}
            onSetDefault={onSetDefaultBackend}
            settingDefaultBackendId={settingDefaultBackendId}
          />
        );
      case "network-lightning":
        return (
          <NetworkLayerSettingsPanel
            layer="lightning"
            backends={backends}
            onAdd={() => openAddBackend("lnd")}
            onEdit={openEditBackend}
            onSetDefault={onSetDefaultBackend}
            settingDefaultBackendId={settingDefaultBackendId}
          />
        );
      case "network-liquid":
        return (
          <NetworkLayerSettingsPanel
            layer="liquid"
            backends={backends}
            onAdd={() => openAddBackend("liquid")}
            onEdit={openEditBackend}
            onSetDefault={onSetDefaultBackend}
            settingDefaultBackendId={settingDefaultBackendId}
          />
        );
      case "network-market":
        return <MarketDataSettingsPanel backends={backends} />;
      case "security-privacy":
        return (
          <PrivacySettingsPanel
            hideSensitive={hideSensitive}
            setHideSensitive={setHideSensitive}
            clearClipboard={clearClipboard}
            setClearClipboard={setClearClipboard}
            backends={backends}
            aiFeaturesEnabled={aiFeaturesEnabled}
            onEditBackend={openEditBackend}
            onManageMarketData={() => goToSection("network-market")}
            onManageAi={() => goToSection("assistant-ai")}
          />
        );
      case "security-lock":
        return (
          <SecuritySettingsPanel
            appLockPolicy={appLockPolicy}
            setAppLockPolicy={setAppLockPolicy}
            onEnrollTouchId={openTouchIdEnrollment}
            onForgetTouchId={forgetTouchIdUnlock}
            encryptedWorkspace={encryptedWorkspace}
            touchIdPlatformSupported={touchIdPlatformSupported}
            touchIdConfigured={touchIdConfigured}
            touchIdStatusPending={touchIdStatusPending}
            touchIdStatusReason={touchIdStatusReason}
            onRefreshTouchId={() => void refreshTouchIdStatus()}
            onLockNow={lockNow}
            onChangePassphrase={openChangePassphrase}
          />
        );
      case "assistant-ai":
        return (
          <AiProvidersSettingsPanel
            aiFeaturesEnabled={aiFeaturesEnabled}
            setAiFeaturesEnabled={setAiFeaturesEnabled}
          />
        );
      case "data-storage":
        return (
          <DataStorageSettingsPanel
            status={status ?? null}
            onOpenImports={() => void navigate({ to: "/imports" })}
            onResetWelcome={onResetWorkspace}
            onResetBook={openResetBookData}
            resetBookDisabled={resetBookData.isPending || !resetBookAvailable}
            onDeleteBooks={openDeleteWorkspace}
            deleteBooksDisabled={deleteWorkspace.isPending}
            resetRegtestAvailable={regtestResetAvailable}
            onResetRegtest={onResetRegtestEnv}
            resetRegtestPending={resetRegtestPending}
          />
        );
      case "desktop-terminal":
        return (
          <TerminalCommandSettingsPanel
            status={terminalStatus}
            error={terminalStatusError}
            pending={terminalCommandPending}
            onRefresh={() => void refreshTerminalCommandStatus()}
            onInstall={() => void onInstallTerminalCommand()}
            onRemove={() => void onRemoveTerminalCommand()}
          />
        );
      case "desktop-developer":
        return (
          <DeveloperToolsSettingsPanel
            enabled={developerToolsEnabled}
            setEnabled={setDeveloperToolsEnabled}
            onOpenLogs={() => void navigate({ to: "/logs" })}
          />
        );
      default:
        return null;
    }
  })();

  return (
    <>
      <div className={screenPanelClassName}>
        <div className="mx-auto flex w-full max-w-[1500px] min-w-0 flex-col gap-5">
          <div className="space-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">
              {t("page.title")}
            </h1>
            <p className="text-sm text-muted-foreground">
              {t("page.description")}
            </p>
          </div>

          {deferredConnectionSetup ? (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-primary/30 bg-primary/5 p-3 text-sm">
              <span>
                {deferredConnectionSetup.reason
                  ? t("page.deferred.bodyWithReason", {
                      reason: deferredConnectionSetup.reason,
                    })
                  : t("page.deferred.body")}
              </span>
              <div className="flex gap-2">
                <Button
                  type="button"
                  size="sm"
                  onClick={() => {
                    void navigate({ to: "/connections" });
                  }}
                >
                  {t("page.deferred.resume")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={clearDeferredConnectionSetup}
                >
                  {t("page.deferred.dismiss")}
                </Button>
              </div>
            </div>
          ) : null}

          <div className="flex min-w-0 flex-col gap-6 lg:flex-row lg:gap-8">
            <SettingsRail
              activeId={activeSectionId}
              onSelect={goToSection}
              counts={sectionCounts}
            />
            <div className="min-w-0 flex-1">
              <div className="mb-5 space-y-1 border-b pb-4">
                {/* dynamic key */}
                <p className="kb-mono-caption">{t(activeMeta.groupKey as never)}</p>
                <h2 className="text-lg font-semibold tracking-tight">
                  {/* dynamic key */}
                  {t(activeMeta.labelKey as never)}
                </h2>
                <p className="max-w-2xl text-sm text-muted-foreground">
                  {/* dynamic key */}
                  {t(activeMeta.descriptionKey as never)}
                </p>
              </div>
              {sectionContent}
            </div>
          </div>
        </div>
      </div>

        <SyncBackendSettingsModal
          open={backendDialogOpen}
          initial={editingBackend}
          initialTypeId={initialBackendTypeId ?? undefined}
          onClose={() => {
            setBackendDialogOpen(false);
            setEditingBackendId(null);
            setInitialBackendTypeId(null);
          }}
          onDelete={onDeleteBackend}
          onSave={onSaveBackend}
        />
        <Dialog
          open={resetDataOpen}
          onOpenChange={(next) => {
            if (!next) {
              setResetDataOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>{t("resetBook.title")}</DialogTitle>
              <DialogDescription>
                {t("resetBook.description", { book: bookLabel })}
                {encryptedWorkspace
                  ? t("resetBook.passphraseHint")
                  : t("resetBook.plaintextHint")}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onResetBookData();
              }}
            >
              {encryptedWorkspace ? (
                <div className="space-y-2">
                  <Label htmlFor="reset-data-passphrase">
                    {t("resetBook.passphraseLabel")}
                  </Label>
                  <Input
                    id="reset-data-passphrase"
                    type="password"
                    autoComplete="current-password"
                    value={resetDataPassphrase}
                    onChange={(event) =>
                      setResetDataPassphrase(event.target.value)
                    }
                  />
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="reset-data-plaintext-ack">
                    {t("resetBook.plaintextLabel")}
                  </Label>
                  <Input
                    id="reset-data-plaintext-ack"
                    value={resetDataPlaintextAck}
                    placeholder={PLAINTEXT_DELETE_ACK}
                    onChange={(event) =>
                      setResetDataPlaintextAck(event.target.value)
                    }
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="reset-data-confirm">
                  {t("resetBook.bookNameLabel")}
                </Label>
                <Input
                  id="reset-data-confirm"
                  value={resetDataConfirm}
                  placeholder={bookLabel}
                  onChange={(event) => setResetDataConfirm(event.target.value)}
                />
              </div>
              <div className="flex items-start gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-3">
                <Checkbox
                  id="reset-data-clear-shared-rates"
                  checked={resetDataClearSharedRates}
                  onCheckedChange={(checked) =>
                    setResetDataClearSharedRates(checked === true)
                  }
                />
                <Label
                  htmlFor="reset-data-clear-shared-rates"
                  className="grid gap-1 text-sm leading-relaxed"
                >
                  <span>{t("resetBook.clearRatesLabel")}</span>
                  <span className="font-normal text-muted-foreground">
                    {t("resetBook.clearRatesHint")}
                  </span>
                </Label>
              </div>
              {resetDataError && (
                <p className="m-0 text-sm text-destructive">
                  {resetDataError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setResetDataOpen(false)}
                >
                  {t("common:actions.cancel")}
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={resetBookData.isPending}
                >
                  {resetBookData.isPending
                    ? t("resetBook.submitPending")
                    : t("resetBook.submit")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={deleteOpen}
          onOpenChange={(next) => {
            if (!next) {
              setDeleteOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>{t("deleteBooks.title")}</DialogTitle>
              <DialogDescription>
                {t("deleteBooks.description", { workspace: workspaceLabel })}
                {encryptedWorkspace
                  ? t("deleteBooks.passphraseHint")
                  : t("deleteBooks.plaintextHint")}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onDeleteWorkspace();
              }}
            >
              {encryptedWorkspace ? (
                <div className="space-y-2">
                  <Label htmlFor="delete-passphrase">
                    {t("deleteBooks.passphraseLabel")}
                  </Label>
                  <Input
                    id="delete-passphrase"
                    type="password"
                    autoComplete="current-password"
                    value={deletePassphrase}
                    onChange={(event) =>
                      setDeletePassphrase(event.target.value)
                    }
                  />
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="delete-plaintext-ack">
                    {t("deleteBooks.plaintextLabel")}
                  </Label>
                  <Input
                    id="delete-plaintext-ack"
                    value={deletePlaintextAck}
                    placeholder={PLAINTEXT_DELETE_ACK}
                    onChange={(event) =>
                      setDeletePlaintextAck(event.target.value)
                    }
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="delete-confirm">
                  {t("deleteBooks.nameLabel")}
                </Label>
                <Input
                  id="delete-confirm"
                  value={deleteConfirm}
                  placeholder={workspaceLabel}
                  onChange={(event) => setDeleteConfirm(event.target.value)}
                />
              </div>
              {deleteError && (
                <p className="m-0 text-sm text-destructive">{deleteError}</p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setDeleteOpen(false)}
                >
                  {t("common:actions.cancel")}
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={deleteWorkspace.isPending}
                >
                  {deleteWorkspace.isPending
                    ? t("deleteBooks.submitPending")
                    : t("deleteBooks.submit")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={passphraseOpen}
          onOpenChange={(next) => {
            if (!next) {
              setPassphraseOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>{t("changePassphrase.title")}</DialogTitle>
              <DialogDescription>
                {t("changePassphrase.description")}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onChangePassphrase();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="current-passphrase">
                  {t("changePassphrase.currentLabel")}
                </Label>
                <Input
                  id="current-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={currentPassphrase}
                  onChange={(event) =>
                    setCurrentPassphrase(event.target.value)
                  }
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-passphrase">
                  {t("changePassphrase.newLabel")}
                </Label>
                <Input
                  id="new-passphrase"
                  type="password"
                  autoComplete="new-password"
                  value={newPassphrase}
                  onChange={(event) => setNewPassphrase(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-passphrase-confirm">
                  {t("changePassphrase.confirmLabel")}
                </Label>
                <Input
                  id="new-passphrase-confirm"
                  type="password"
                  autoComplete="new-password"
                  value={newPassphraseConfirm}
                  onChange={(event) =>
                    setNewPassphraseConfirm(event.target.value)
                  }
                />
              </div>
              {passphraseError && (
                <p className="m-0 text-sm text-destructive">
                  {passphraseError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setPassphraseOpen(false)}
                >
                  {t("common:actions.cancel")}
                </Button>
                <Button type="submit" disabled={changePassphrase.isPending}>
                  {changePassphrase.isPending
                    ? t("changePassphrase.submitPending")
                    : t("changePassphrase.submit")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={touchIdEnrollOpen}
          onOpenChange={(next) => {
            if (!next && !touchIdEnrollPending) {
              setTouchIdEnrollOpen(false);
              setTouchIdEnrollPassphrase("");
              setTouchIdEnrollError(null);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>{t("touchIdEnroll.title")}</DialogTitle>
              <DialogDescription>
                {t("touchIdEnroll.description")}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onEnrollTouchId();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="touch-id-enroll-passphrase">
                  {t("touchIdEnroll.passphraseLabel")}
                </Label>
                <Input
                  id="touch-id-enroll-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={touchIdEnrollPassphrase}
                  disabled={touchIdEnrollPending}
                  onChange={(event) =>
                    setTouchIdEnrollPassphrase(event.target.value)
                  }
                />
              </div>
              {touchIdEnrollError && (
                <p className="m-0 text-sm text-destructive">
                  {touchIdEnrollError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  disabled={touchIdEnrollPending}
                  onClick={() => {
                    setTouchIdEnrollOpen(false);
                    setTouchIdEnrollPassphrase("");
                    setTouchIdEnrollError(null);
                  }}
                >
                  {t("common:actions.cancel")}
                </Button>
                <Button type="submit" disabled={touchIdEnrollPending}>
                  {touchIdEnrollPending
                    ? t("touchIdEnroll.submitPending")
                    : t("touchIdEnroll.submit")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
    </>
  );
}
