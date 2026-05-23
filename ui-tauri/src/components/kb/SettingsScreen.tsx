/**
 * SettingsScreen - workspace-wide preferences.
 *
 * Most controls are local UI state until the daemon-backed settings surface
 * lands. Hide-sensitive data is wired to the shared UI store.
 */
import * as React from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";

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
  canUseTouchIdPassphraseUnlock,
  clearImportProject,
  forgetTouchIdPassphrase,
  getTransport,
  installTerminalCommand,
  removeTerminalCommand,
  storeTouchIdPassphrase,
  touchIdPassphraseStatus,
  terminalCommandStatus,
  type TerminalCommandStatus,
  type TouchIdPassphraseStatus,
} from "@/daemon/transport";
import type { InfrastructureOwnership } from "@/lib/backendTrust";
import { screenPanelClassName } from "@/lib/screen-layout";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";
import { useUiStore } from "@/store/ui";
import { databasePassphraseHint } from "@/components/kb/Onboarding/constants";
import { settingsSectionForHash, type SettingsSectionId } from "./settingsSections";
import { AppearanceSettingsPanel } from "./settings/AppearanceSettingsPanel";
import { AiProvidersSettingsPanel } from "./settings/AiProvidersSettingsPanel";
import { SyncBackendSettingsModal, backendTypeIdForConnectionSetup, type SyncBackendNetwork } from "./settings/SyncBackendSettingsModal";
import { DataStorageSettingsPanel } from "./settings/DataStorageSettingsPanel";
import { DeveloperToolsSettingsPanel } from "./settings/DeveloperToolsSettingsPanel";
import { MarketDataSettingsPanel } from "./settings/MarketDataSettingsPanel";
import { NetworkLayerSettingsPanel } from "./settings/NetworkLayerSettingsPanel";
import { PrivacySettingsPanel } from "./settings/PrivacySettingsPanel";
import { SecuritySettingsPanel } from "./settings/SecuritySettingsPanel";
import { TerminalCommandSettingsPanel } from "./settings/TerminalCommandSettingsPanel";
import { DEFAULT_SETTINGS_SECTION, SettingsRail, sectionMeta } from "./settings/SettingsNavigation";
import {
  DEFAULT_RATE_BACKENDS,
  PLAINTEXT_DELETE_ACK,
  backendPayload,
  backendRowToSettingsBackend,
  backendsForLayer,
  deriveExplorerSettings,
  formatCount,
  type Backend,
  type BackendSettingsData,
  type BackendSettingsRow,
  type ResetBookData,
  type StatusData,
} from "./settings/SettingsModel";

interface SettingsScreenProps {
  onLock?: () => void;
}

export function SettingsScreen({ onLock }: SettingsScreenProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);
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
  const createBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.create");
  const updateBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.update");
  const createWallet = useDaemonMutation("ui.wallets.create");
  const deleteBackend = useDaemonMutation<{ name: string; deleted: boolean }>(
    "ui.backends.delete",
  );
  const [clearClipboard, setClearClipboard] = React.useState(true);
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
          title: "Touch ID passphrase was not removed",
          body:
            error instanceof Error
              ? error.message
              : "macOS Keychain did not remove the saved passphrase.",
          tone: "warning",
        });
      }
    },
    [addNotification, setAppLockPolicy, touchIdDataRoot],
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
          : "Could not inspect terminal command.";
      setTerminalStatusError(message);
      return null;
    }
  }, []);

  React.useEffect(() => {
    void refreshTerminalCommandStatus();
  }, [refreshTerminalCommandStatus]);

  // Native menu may re-fire for the same section while the URL hash is
  // unchanged (user already on /settings#privacy, clicks Privacy again after
  // closing the panel). The hash effect won't see a diff, so listen for an
  // explicit `kassiber:settings-section` event and force re-selection.
  React.useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ section?: string | null }>).detail;
      const next = settingsSectionForHash(detail?.section ?? "");
      if (next) setActiveSectionId(next);
    };
    window.addEventListener("kassiber:settings-section", handler);
    return () => {
      window.removeEventListener("kassiber:settings-section", handler);
    };
  }, []);

  const backends = React.useMemo<Backend[]>(() => {
    const syncRows = backendSettingsQuery.data?.data?.backends ?? [];
    return [
      ...syncRows.map(backendRowToSettingsBackend),
      ...DEFAULT_RATE_BACKENDS,
    ];
  }, [backendSettingsQuery.data]);

  React.useEffect(() => {
    if (!backendSettingsQuery.data?.data?.backends) return;
    setExplorerSettings(deriveExplorerSettings(backends));
  }, [backendSettingsQuery.data, backends, setExplorerSettings]);

  const editingBackend = React.useMemo(
    () => backends.find((backend) => backend.id === editingBackendId) ?? null,
    [backends, editingBackendId],
  );

  const onResetWorkspace = () => {
    const ok = window.confirm(
      "Reset Welcome state?\n\nThis clears your local identity and returns you to the Welcome screen. Encrypted data on disk is not touched.",
    );
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
    status?.current_workspace || identity?.workspace || "current books set";
  const currentBookLabel =
    statusLoaded
      ? status?.current_profile ?? null
      : identity?.profile || identity?.name || null;
  const bookLabel = currentBookLabel || "current book";
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
  };

  const onDeleteBackend = async (backend: Backend) => {
    const ok = window.confirm(
      `Delete backend '${backend.name}'?\n\nWallets using this endpoint may need another backend before they can sync.`,
    );
    if (!ok) return;
    await deleteBackend.mutateAsync({ name: backend.id });
    const refreshed = await backendSettingsQuery.refetch();
    const refreshedBackends = (refreshed.data?.data?.backends ?? []).map(
      backendRowToSettingsBackend,
    );
    setExplorerSettings(deriveExplorerSettings(refreshedBackends));
  };

  const onSetBackendOwnership = async (
    backend: Backend,
    ownership: InfrastructureOwnership,
  ) => {
    await updateBackend.mutateAsync({
      ...backendPayload({ ...backend, infrastructureOwner: ownership }),
      name: backend.id,
    });
    await backendSettingsQuery.refetch();
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
          ? "Terminal command repaired"
          : "Terminal command installed",
        body: next.pathOnPath
          ? "Open a new terminal and run kassiber status."
          : `Add ${next.binDir} to PATH, then open a new terminal.`,
        tone: next.pathOnPath ? "success" : "warning",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : "Could not install terminal command.",
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
        title: "Terminal command removed",
        body: `${next.commandPath || "The command"} was removed.`,
        tone: "success",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : "Could not remove terminal command.",
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
      setDeleteError("Enter the database passphrase.");
      return;
    }
    if (
      !encryptedWorkspace &&
      deletePlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setDeleteError(`Type ${PLAINTEXT_DELETE_ACK} to confirm local deletion.`);
      return;
    }
    if (deleteConfirm.trim() !== workspaceLabel) {
      setDeleteError(`Type ${workspaceLabel} to confirm deletion.`);
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
          : "Books delete failed.",
      );
    }
  };

  const onResetBookData = async () => {
    setResetDataError(null);
    if (encryptedWorkspace && !resetDataPassphrase) {
      setResetDataError("Enter the database passphrase.");
      return;
    }
    if (
      !encryptedWorkspace &&
      resetDataPlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setResetDataError(`Type ${PLAINTEXT_DELETE_ACK} to confirm local reset.`);
      return;
    }
    if (resetDataConfirm.trim() !== bookLabel) {
      setResetDataError(`Type ${bookLabel} to confirm reset.`);
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
        `${formatCount(removed.transactions ?? 0)} transactions`,
        `${formatCount(removed.journal_entries ?? 0)} journal rows`,
        `${formatCount(removed.transaction_pairs ?? 0)} swap pairs`,
        `${formatCount(removed.bip329_labels ?? 0)} BIP329 labels`,
      ];
      if (attachmentsRemoved > 0) {
        optionalSummary.push(`${formatCount(attachmentsRemoved)} attachment records/files`);
      }
      if (sourceFundsRemoved > 0) {
        optionalSummary.push(`${formatCount(sourceFundsRemoved)} source-funds records`);
      }
      if (envelope.data?.shared_rates_cleared) {
        optionalSummary.push(`${formatCount(removed.rates_cache ?? 0)} rate rows`);
      }
      summary.push(...optionalSummary);
      addNotification({
        title: "Book data reset",
        body: `Cleared ${summary.join(", ")}. Wallet and backend connections were kept.`,
        tone: "success",
      });
      setResetDataOpen(false);
      setResetDataPassphrase("");
      setResetDataConfirm("");
      setResetDataClearSharedRates(false);
      setResetDataPlaintextAck("");
    } catch (error) {
      setResetDataError(
        error instanceof Error ? error.message : "Book reset failed.",
      );
    }
  };

  const onChangePassphrase = async () => {
    setPassphraseError(null);
    if (!currentPassphrase) {
      setPassphraseError("Enter the current database passphrase.");
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
                ? `Touch ID unlock is not set up: ${status.reason}`
                : "macOS Keychain did not report the saved Touch ID passphrase.",
            );
          }
        } catch (error) {
          setAppLockPolicy({ touchIdUnlock: false });
          await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
          await refreshTouchIdStatus();
          addNotification({
            title: "Touch ID unlock was disabled",
            body:
              error instanceof Error
                ? error.message
                : "The database passphrase changed, but macOS Keychain did not accept the updated Touch ID passphrase.",
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
          : "Could not change database passphrase.",
      );
    }
  };

  const onEnrollTouchId = async () => {
    setTouchIdEnrollError(null);
    if (!touchIdEnrollPassphrase) {
      setTouchIdEnrollError("Enter the database passphrase.");
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
        throw new Error("Database passphrase did not unlock these books.");
      }
      const status = await storeTouchIdPassphrase(
        touchIdEnrollPassphrase,
        touchIdDataRoot,
      );
      setTouchIdStatus(status);
      if (!status.configured) {
        throw new Error(
          status.reason
            ? `Touch ID unlock is not set up: ${status.reason}`
            : "macOS Keychain did not report the saved Touch ID passphrase.",
        );
      }
      await setSessionUnlockPassphrase(touchIdEnrollPassphrase);
      setAppLockPolicy({ touchIdUnlock: true });
      setTouchIdEnrollOpen(false);
      setTouchIdEnrollPassphrase("");
      addNotification({
        title: "Touch ID unlock enabled",
        body: "The database passphrase was saved in macOS Keychain behind local user presence.",
        tone: "success",
      });
    } catch (error) {
      setAppLockPolicy({ touchIdUnlock: false });
      await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
      await refreshTouchIdStatus();
      setTouchIdEnrollError(
        error instanceof Error
          ? error.message
          : "Could not save the database passphrase for Touch ID unlock.",
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
          />
        );
      case "network-bitcoin":
        return (
          <NetworkLayerSettingsPanel
            layer="bitcoin"
            backends={backends}
            onAdd={() => openAddBackend("bitcoin")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
            onSetOwnership={onSetBackendOwnership}
          />
        );
      case "network-lightning":
        return (
          <NetworkLayerSettingsPanel
            layer="lightning"
            backends={backends}
            onAdd={() => openAddBackend("lnd")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
            onSetOwnership={onSetBackendOwnership}
          />
        );
      case "network-liquid":
        return (
          <NetworkLayerSettingsPanel
            layer="liquid"
            backends={backends}
            onAdd={() => openAddBackend("liquid")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
            onSetOwnership={onSetBackendOwnership}
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
            onSetBackendOwnership={onSetBackendOwnership}
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
            <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
            <p className="text-sm text-muted-foreground">
              Configure how Kassiber reaches the Bitcoin network, what stays on
              this machine, and how your books are stored.
            </p>
          </div>

          {deferredConnectionSetup ? (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-primary/30 bg-primary/5 p-3 text-sm">
              <span>
                You came here from connection setup
                {deferredConnectionSetup.reason
                  ? ` (${deferredConnectionSetup.reason})`
                  : ""}
                . Configure the backend below, then resume.
              </span>
              <div className="flex gap-2">
                <Button
                  type="button"
                  size="sm"
                  onClick={() => {
                    void navigate({ to: "/connections" });
                  }}
                >
                  Resume connection setup
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={clearDeferredConnectionSetup}
                >
                  Dismiss
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
                <p className="kb-mono-caption">{activeMeta.group}</p>
                <h2 className="text-lg font-semibold tracking-tight">
                  {activeMeta.label}
                </h2>
                <p className="max-w-2xl text-sm text-muted-foreground">
                  {activeMeta.description}
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
              <DialogTitle>Reset book data</DialogTitle>
              <DialogDescription>
                This keeps wallet and backend connections for {bookLabel}, then
                clears imported/synced rows, journals, swap review state, labels,
                attachments, and source-funds work. The shared fiat-rate cache
                is kept unless you explicitly include it below.
                {encryptedWorkspace
                  ? " Enter the database passphrase and the book name to continue."
                  : " These plaintext books have no database passphrase; type the explicit local-reset challenge and book name to continue."}
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
                  <Label htmlFor="reset-data-passphrase">Passphrase</Label>
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
                    Plaintext reset challenge
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
                <Label htmlFor="reset-data-confirm">Book name</Label>
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
                  <span>Also clear shared fiat-rate cache</span>
                  <span className="font-normal text-muted-foreground">
                    This cache is shared across every book in this local data
                    root. Leave it off to keep existing BTC-EUR/BTC-USD history.
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
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={resetBookData.isPending}
                >
                  {resetBookData.isPending ? "Resetting..." : "Reset"}
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
              <DialogTitle>Delete books set</DialogTitle>
              <DialogDescription>
                This removes {workspaceLabel} from the local Kassiber database.
                {encryptedWorkspace
                  ? " Enter the database passphrase and the books set name to continue."
                  : " These plaintext books have no database passphrase; type the explicit local-delete challenge and books set name to continue."}
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
                  <Label htmlFor="delete-passphrase">Passphrase</Label>
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
                    Plaintext delete challenge
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
                <Label htmlFor="delete-confirm">Books set name</Label>
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
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={deleteWorkspace.isPending}
                >
                  {deleteWorkspace.isPending ? "Deleting..." : "Delete"}
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
              <DialogTitle>Change database passphrase</DialogTitle>
              <DialogDescription>
                Rekey the local SQLCipher database. The current daemon session
                is reopened with the new passphrase after rotation.
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
                <Label htmlFor="current-passphrase">Current passphrase</Label>
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
                <Label htmlFor="new-passphrase">New passphrase</Label>
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
                  Confirm new passphrase
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
                  Cancel
                </Button>
                <Button type="submit" disabled={changePassphrase.isPending}>
                  {changePassphrase.isPending ? "Changing..." : "Change"}
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
              <DialogTitle>Enable Touch ID unlock</DialogTitle>
              <DialogDescription>
                Enter the database passphrase once. Kassiber will verify it
                locally, then save it in macOS Keychain behind local user
                presence for these books.
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
                  Database passphrase
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
                  Cancel
                </Button>
                <Button type="submit" disabled={touchIdEnrollPending}>
                  {touchIdEnrollPending ? "Saving..." : "Enable Touch ID"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
    </>
  );
}
