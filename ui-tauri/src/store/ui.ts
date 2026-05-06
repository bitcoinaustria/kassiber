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

export interface AppNotification {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  createdAt: string;
}

export type AppLogLevel = "debug" | "info" | "warning" | "error";

export interface AppLogEntry {
  id: string;
  createdAt: string;
  level: AppLogLevel;
  source: string;
  message: string;
  details?: unknown;
}

export interface AppLockPolicy {
  autoLockWhenIdle: boolean;
  idleMinutes: number;
  requirePassphraseOnLaunch: boolean;
  lockOnWindowClose: boolean;
}

export interface ImportedProjectIdentity {
  stateRoot: string;
  dataRoot: string;
  database: string;
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
 * `aiSetupMode` records welcome-flow intent only. In particular,
 * `aiSetupMode: "disabled"` does not yet hide or disable the assistant dock.
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
  aiSetupMode?: "local" | "remote" | "disabled";
  aiProviderKind?: "local" | "remote" | "tee";
  aiProviderName?: string;
  aiBaseUrl?: string;
  importedProject?: ImportedProjectIdentity;
}

interface UiState {
  lang: Lang;
  currency: Currency;
  dataMode: DataMode;
  theme: ThemePreference;
  hideSensitive: boolean;
  explorerSettings: ExplorerSettings;
  appLockPolicy: AppLockPolicy;
  identity: Identity | null;
  daemonSession: number;
  notifications: AppNotification[];
  logEntries: AppLogEntry[];
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setDataMode: (dataMode: DataMode) => void;
  setTheme: (theme: ThemePreference) => void;
  setHideSensitive: (hideSensitive: boolean) => void;
  setExplorerSettings: (settings: Partial<ExplorerSettings>) => void;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  setIdentity: (identity: Identity | null) => void;
  bumpDaemonSession: () => void;
  addNotification: (
    notification: Omit<AppNotification, "id" | "createdAt">,
  ) => void;
  clearNotification: (id: string) => void;
  clearNotifications: () => void;
  addLogEntry: (entry: Omit<AppLogEntry, "id" | "createdAt">) => void;
  clearLogEntries: () => void;
}

const DEFAULT_APP_LOCK_POLICY: AppLockPolicy = {
  autoLockWhenIdle: true,
  idleMinutes: 5,
  requirePassphraseOnLaunch: true,
  lockOnWindowClose: true,
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

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      dataMode: "real",
      theme: "system",
      hideSensitive: false,
      explorerSettings: DEFAULT_EXPLORER_SETTINGS,
      appLockPolicy: DEFAULT_APP_LOCK_POLICY,
      identity: null,
      daemonSession: 0,
      notifications: [],
      logEntries: [],
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setDataMode: (dataMode) => set({ dataMode }),
      setTheme: (theme) => set({ theme }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
      setExplorerSettings: (settings) =>
        set((state) => ({
          explorerSettings: { ...state.explorerSettings, ...settings },
        })),
      setAppLockPolicy: (policy) =>
        set((state) => ({
          appLockPolicy: { ...state.appLockPolicy, ...policy },
        })),
      setIdentity: (identity) => set({ identity: normalizeIdentity(identity) }),
      bumpDaemonSession: () =>
        set((state) => ({ daemonSession: state.daemonSession + 1 })),
      addNotification: (notification) =>
        set((state) => ({
          notifications: [
            {
              ...notification,
              id: `${Date.now()}-${state.notifications.length}`,
              createdAt: new Date().toISOString(),
            },
            ...state.notifications,
          ].slice(0, 12),
        })),
      clearNotification: (id) =>
        set((state) => ({
          notifications: state.notifications.filter(
            (notification) => notification.id !== id,
          ),
        })),
      clearNotifications: () => set({ notifications: [] }),
      addLogEntry: (entry) =>
        set((state) => ({
          logEntries: [
            {
              ...entry,
              id: `${Date.now()}-${state.logEntries.length}`,
              createdAt: new Date().toISOString(),
            },
            ...state.logEntries,
          ].slice(0, 300),
        })),
      clearLogEntries: () => set({ logEntries: [] }),
    }),
    {
      name: "kb.ui",
      partialize: (state) => ({
        lang: state.lang,
        currency: state.currency,
        dataMode: state.dataMode,
        hideSensitive: state.hideSensitive,
        explorerSettings: state.explorerSettings,
        appLockPolicy: state.appLockPolicy,
        identity: state.identity,
        daemonSession: state.daemonSession,
        notifications: state.notifications,
      }),
      merge: (persisted, current) => {
        const restored = persisted as Partial<UiState>;
        return {
          ...current,
          ...restored,
          explorerSettings: {
            ...DEFAULT_EXPLORER_SETTINGS,
            ...(restored.explorerSettings ?? current.explorerSettings),
          },
          appLockPolicy: {
            ...DEFAULT_APP_LOCK_POLICY,
            ...(restored.appLockPolicy ?? current.appLockPolicy),
          },
          identity: normalizeIdentity(restored.identity ?? current.identity),
          logEntries: current.logEntries,
        };
      },
    },
  ),
);
