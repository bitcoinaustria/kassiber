import { create } from "zustand";
import { persist } from "zustand/middleware";

import {
  DEFAULT_EXPLORER_SETTINGS,
  type ExplorerSettings,
} from "@/lib/explorer";

type Lang = "en" | "de";
type Currency = "btc" | "eur";
export type DataMode = "mock" | "real";
export type ThemePreference = "system" | "light" | "dark";
export type NotificationTone = "info" | "success" | "warning" | "error";

export const DEFAULT_APP_SCALE = 0.9;
export const MIN_APP_SCALE = 0.8;
export const MAX_APP_SCALE = 1.2;
export const APP_SCALE_STEP = 0.05;

export interface NotificationProgress {
  value?: number;
  indeterminate?: boolean;
  label?: string;
}

export interface AppNotification {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  dedupeKey?: string;
  progress?: NotificationProgress;
  createdAt: string;
}

export interface ActiveMaintenanceProgress {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  progress: NotificationProgress;
  details?: string[];
  active: boolean;
  startedAt: string;
  updatedAt: string;
}

export interface AppLockPolicy {
  autoLockWhenIdle: boolean;
  idleMinutes: number;
  requirePassphraseOnLaunch: boolean;
  lockOnWindowClose: boolean;
  touchIdUnlock: boolean;
}

export interface ImportedProjectIdentity {
  stateRoot: string;
  dataRoot: string;
  database: string;
}

export interface AiModelSelection {
  provider: string;
  model: string;
}

/**
 * Captured onboarding intent for the local books. Only `name`/`workspace`
 * are read by the running UI today (see AppHeader); the remaining fields are
 * captured in the webview so the upcoming native sidecar handoff can seed a
 * books/backend without re-prompting.
 *
 * `country` is the legacy identity field; new callers should prefer
 * `taxCountry`. Today only `at` and `generic` map to a real rp2 plugin
 * (see `kassiber.tax_policy`), so `country` resolves to `"AT" | "Generic"`.
 *
 * `encrypted` means the UI requested daemon-backed SQLCipher initialization
 * during onboarding. The passphrase is never written into this persisted
 * store; the user must re-enter it when a locked daemon session needs to open
 * the database.
 *
 * `aiSetupMode` records welcome-flow intent for a newly-created books set.
 * The running UI uses `aiFeaturesEnabled` as the global feature switch after
 * onboarding, so users can enable or disable the assistant later.
 */
export interface Identity {
  name: string;
  workspace: string;
  country: string;
  encrypted: boolean;
  profile?: string;
  taxCountry?: "at" | "generic";
  fiatCurrency?: string;
  taxLongTermDays?: number;
  /**
   * Token matches `kassiber.tax_policy` / rp2 plugin tokens. Generic supports
   * FIFO/LIFO/HIFO/LOFO; new AT onboarding uses MOVING_AVERAGE_AT only.
   */
  gainsAlgorithm?:
    | "FIFO"
    | "LIFO"
    | "HIFO"
    | "LOFO"
    | "MOVING_AVERAGE_AT";
  databaseMode?: "sqlcipher" | "plaintext";
  migrateCredentials?: boolean;
  backendSetupMode?: "default" | "custom" | "skip";
  backendKind?:
    | "esplora"
    | "electrum"
    | "bitcoinrpc"
    | "btcpay"
    | "liquid-esplora"
    | "custom";
  backendName?: string;
  backendUrl?: string;
  backendTrustSsl?: boolean;
  backendCertificate?: string;
  backendProxy?: {
    host: string;
    port: string;
  } | null;
  aiSetupMode?: "local" | "remote" | "disabled";
  aiProviderKind?: "local" | "remote" | "tee";
  aiProviderName?: string;
  aiBaseUrl?: string;
  importedProject?: ImportedProjectIdentity;
}

export interface SourceFundsDraft {
  target?: string;
  targetAmount?: string;
  reportPurpose?: "existing_transaction" | "planned_exchange_sale";
  plannedDestination?: string;
  plannedNote?: string;
  revealMode?: string;
  selectedRecipientId?: string;
  currentStep?: "setup" | "review" | "export";
}

/** Captures a "I was setting up X, came here to add a backend, take me back" hop. */
export interface DeferredConnectionSetup {
  sourceId: string;
  reason: string;
  backendKind?: string;
}

export interface UiState {
  lang: Lang;
  currency: Currency;
  dataMode: DataMode;
  theme: ThemePreference;
  appScale: number;
  hideSensitive: boolean;
  clearClipboard: boolean;
  explorerSettings: ExplorerSettings;
  appLockPolicy: AppLockPolicy;
  identity: Identity | null;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
  assistantModelSelection: AiModelSelection | null;
  daemonSession: number;
  notifications: AppNotification[];
  activeMaintenanceProgress: ActiveMaintenanceProgress | null;
  sourceFundsDrafts: Record<string, SourceFundsDraft>;
  deferredConnectionSetup: DeferredConnectionSetup | null;
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setDataMode: (dataMode: DataMode) => void;
  setTheme: (theme: ThemePreference) => void;
  setAppScale: (appScale: number) => void;
  increaseAppScale: () => void;
  decreaseAppScale: () => void;
  resetAppScale: () => void;
  setHideSensitive: (hideSensitive: boolean) => void;
  setClearClipboard: (clearClipboard: boolean) => void;
  setExplorerSettings: (settings: Partial<ExplorerSettings>) => void;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  setIdentity: (identity: Identity | null) => void;
  setAiFeaturesEnabled: (enabled: boolean) => void;
  setDeveloperToolsEnabled: (enabled: boolean) => void;
  setAssistantModelSelection: (selection: AiModelSelection | null) => void;
  bumpDaemonSession: () => void;
  addNotification: (
    notification: Omit<AppNotification, "id" | "createdAt">,
  ) => string;
  updateNotification: (
    id: string,
    patch: Partial<Omit<AppNotification, "id" | "createdAt">>,
  ) => void;
  setActiveMaintenanceProgress: (
    progress: ActiveMaintenanceProgress | null,
  ) => void;
  clearActiveMaintenanceProgress: (id?: string) => void;
  clearNotification: (id: string) => void;
  clearNotifications: () => void;
  setSourceFundsDraft: (profileKey: string, draft: SourceFundsDraft) => void;
  clearSourceFundsDraft: (profileKey: string) => void;
  setDeferredConnectionSetup: (intent: DeferredConnectionSetup | null) => void;
  clearDeferredConnectionSetup: () => void;
}

const DEFAULT_APP_LOCK_POLICY: AppLockPolicy = {
  autoLockWhenIdle: false,
  idleMinutes: 5,
  requirePassphraseOnLaunch: false,
  lockOnWindowClose: false,
  touchIdUnlock: false,
};

function normalizeIdentity(identity: Identity | null): Identity | null {
  if (!identity) return identity;
  const isLegacyMockIdentity =
    !identity.importedProject &&
    identity.workspace === "Demo Workspace" &&
    (identity.name === "mock profile" || identity.profile === "mock");

  if (!isLegacyMockIdentity) return identity;

  return {
    ...identity,
    name: identity.name === "mock profile" ? "mock books" : identity.name,
    profile: identity.profile === "mock" ? "mock books" : identity.profile,
    workspace: "My Books",
  };
}

function stripNotificationProgress(
  notifications: AppNotification[] | undefined,
): AppNotification[] {
  return (notifications ?? []).map((notification) => {
    const clone = { ...notification };
    delete clone.progress;
    return clone;
  });
}

export function normalizeAppScale(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return DEFAULT_APP_SCALE;
  }
  const stepped = Math.round(value / APP_SCALE_STEP) * APP_SCALE_STEP;
  const clamped = Math.min(MAX_APP_SCALE, Math.max(MIN_APP_SCALE, stepped));
  return Number(clamped.toFixed(2));
}

export function uiStatePartialForStorage(state: UiState) {
  return {
    lang: state.lang,
    currency: state.currency,
    dataMode: state.dataMode,
    theme: state.theme,
    hideSensitive: state.hideSensitive,
    clearClipboard: state.clearClipboard,
    appScale: state.appScale,
    explorerSettings: state.explorerSettings,
    appLockPolicy: state.appLockPolicy,
    identity: state.identity,
    aiFeaturesEnabled: state.aiFeaturesEnabled,
    developerToolsEnabled: state.developerToolsEnabled,
    assistantModelSelection: state.assistantModelSelection,
    daemonSession: state.daemonSession,
    notifications: stripNotificationProgress(state.notifications),
    sourceFundsDrafts: state.sourceFundsDrafts,
  };
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      dataMode: "real",
      theme: "system",
      appScale: DEFAULT_APP_SCALE,
      hideSensitive: false,
      clearClipboard: true,
      explorerSettings: DEFAULT_EXPLORER_SETTINGS,
      appLockPolicy: DEFAULT_APP_LOCK_POLICY,
      identity: null,
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      assistantModelSelection: null,
      daemonSession: 0,
      notifications: [],
      activeMaintenanceProgress: null,
      sourceFundsDrafts: {},
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setDataMode: (dataMode) => set({ dataMode }),
      setTheme: (theme) => set({ theme }),
      setAppScale: (appScale) =>
        set({ appScale: normalizeAppScale(appScale) }),
      increaseAppScale: () =>
        set((state) => ({
          appScale: normalizeAppScale(state.appScale + APP_SCALE_STEP),
        })),
      decreaseAppScale: () =>
        set((state) => ({
          appScale: normalizeAppScale(state.appScale - APP_SCALE_STEP),
        })),
      resetAppScale: () => set({ appScale: DEFAULT_APP_SCALE }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
      setClearClipboard: (clearClipboard) => set({ clearClipboard }),
      setExplorerSettings: (settings) =>
        set((state) => ({
          explorerSettings: { ...state.explorerSettings, ...settings },
        })),
      setAppLockPolicy: (policy) =>
        set((state) => ({
          appLockPolicy: { ...state.appLockPolicy, ...policy },
        })),
      setIdentity: (identity) =>
        set((state) => {
          const normalized = normalizeIdentity(identity);
          return {
            identity: normalized,
            aiFeaturesEnabled: normalized?.aiSetupMode
              ? normalized.aiSetupMode !== "disabled"
              : state.aiFeaturesEnabled,
          };
        }),
      setAiFeaturesEnabled: (enabled) => set({ aiFeaturesEnabled: enabled }),
      setDeveloperToolsEnabled: (enabled) =>
        set({ developerToolsEnabled: enabled }),
      setAssistantModelSelection: (assistantModelSelection) =>
        set({ assistantModelSelection }),
      bumpDaemonSession: () =>
        set((state) => ({ daemonSession: state.daemonSession + 1 })),
      addNotification: (notification) => {
        const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
        let existingId: string | null = null;
        set((state) => ({
          notifications: (() => {
            const createdAt = new Date().toISOString();
            if (notification.dedupeKey) {
              const existing = state.notifications.find(
                (item) => item.dedupeKey === notification.dedupeKey,
              );
              if (existing) {
                existingId = existing.id;
                return [
                  {
                    ...existing,
                    ...notification,
                    progress: notification.progress,
                    createdAt,
                  },
                  ...state.notifications.filter((item) => item.id !== existing.id),
                ].slice(0, 12);
              }
            }
            return [
              {
                ...notification,
                id,
                createdAt,
              },
              ...state.notifications,
            ].slice(0, 12);
          })(),
        }));
        return existingId ?? id;
      },
      updateNotification: (id, patch) =>
        set((state) => ({
          notifications: state.notifications.map((notification) =>
            notification.id === id ? { ...notification, ...patch } : notification,
          ),
        })),
      setActiveMaintenanceProgress: (activeMaintenanceProgress) =>
        set({ activeMaintenanceProgress }),
      clearActiveMaintenanceProgress: (id) =>
        set((state) => {
          const current = state.activeMaintenanceProgress;
          if (!current) return state;
          if (id && current.id !== id) return state;
          return { activeMaintenanceProgress: null };
        }),
      clearNotification: (id) =>
        set((state) => ({
          notifications: state.notifications.filter(
            (notification) => notification.id !== id,
          ),
        })),
      clearNotifications: () => set({ notifications: [] }),
      setSourceFundsDraft: (profileKey, draft) =>
        set((state) => {
          const existing = state.sourceFundsDrafts[profileKey] ?? {};
          return {
            sourceFundsDrafts: {
              ...state.sourceFundsDrafts,
              [profileKey]: { ...existing, ...draft },
            },
          };
        }),
      clearSourceFundsDraft: (profileKey) =>
        set((state) => {
          if (!(profileKey in state.sourceFundsDrafts)) return state;
          const next = { ...state.sourceFundsDrafts };
          delete next[profileKey];
          return { sourceFundsDrafts: next };
        }),
      deferredConnectionSetup: null,
      setDeferredConnectionSetup: (intent) =>
        set({ deferredConnectionSetup: intent }),
      clearDeferredConnectionSetup: () =>
        set({ deferredConnectionSetup: null }),
    }),
    {
      name: "kb.ui",
      partialize: uiStatePartialForStorage,
      merge: (persisted, current) => {
        const restored = persisted as Partial<UiState>;
        const identity = normalizeIdentity(restored.identity ?? current.identity);
        const aiFeaturesEnabled =
          restored.aiFeaturesEnabled ??
          (identity?.aiSetupMode === "disabled"
            ? false
            : current.aiFeaturesEnabled);
        return {
          ...current,
          ...restored,
          appScale: normalizeAppScale(restored.appScale ?? current.appScale),
          clearClipboard: restored.clearClipboard ?? current.clearClipboard,
          explorerSettings: {
            ...DEFAULT_EXPLORER_SETTINGS,
            ...(restored.explorerSettings ?? current.explorerSettings),
          },
          appLockPolicy: {
            ...DEFAULT_APP_LOCK_POLICY,
            ...(restored.appLockPolicy ?? current.appLockPolicy),
          },
          identity,
          aiFeaturesEnabled,
          developerToolsEnabled:
            restored.developerToolsEnabled ?? current.developerToolsEnabled,
          assistantModelSelection:
            restored.assistantModelSelection ??
            current.assistantModelSelection,
          notifications: stripNotificationProgress(
            restored.notifications ?? current.notifications,
          ),
          sourceFundsDrafts:
            restored.sourceFundsDrafts ?? current.sourceFundsDrafts,
        };
      },
    },
  ),
);
