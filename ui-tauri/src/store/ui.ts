import { create } from "zustand";
import { persist } from "zustand/middleware";

type Lang = "en" | "de";
type Currency = "btc" | "eur";
export type DataMode = "mock" | "real";
export type NotificationTone = "info" | "success" | "warning" | "error";

export interface AppNotification {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  createdAt: string;
}

export interface AppLockPolicy {
  autoLockWhenIdle: boolean;
  idleMinutes: number;
  requirePassphraseOnLaunch: boolean;
  lockOnWindowClose: boolean;
}

/**
 * Captured onboarding intent for the local profile. Only `name`/`workspace`
 * are read by the running UI today (see AppHeader); the remaining fields are
 * captured in the webview so the upcoming native sidecar handoff can seed a
 * profile/backend without re-prompting.
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
}

interface UiState {
  lang: Lang;
  currency: Currency;
  dataMode: DataMode;
  hideSensitive: boolean;
  appLockPolicy: AppLockPolicy;
  identity: Identity | null;
  notifications: AppNotification[];
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setDataMode: (dataMode: DataMode) => void;
  setHideSensitive: (hideSensitive: boolean) => void;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  setIdentity: (identity: Identity | null) => void;
  addNotification: (
    notification: Omit<AppNotification, "id" | "createdAt">,
  ) => void;
  clearNotification: (id: string) => void;
  clearNotifications: () => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      dataMode: "real",
      hideSensitive: false,
      appLockPolicy: {
        autoLockWhenIdle: true,
        idleMinutes: 5,
        requirePassphraseOnLaunch: true,
        lockOnWindowClose: true,
      },
      identity: null,
      notifications: [],
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setDataMode: (dataMode) => set({ dataMode }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
      setAppLockPolicy: (policy) =>
        set((state) => ({
          appLockPolicy: { ...state.appLockPolicy, ...policy },
        })),
      setIdentity: (identity) => set({ identity }),
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
    }),
    { name: "kb.ui" },
  ),
);
