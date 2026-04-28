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
 * `encrypted` records the user's *intent* to use SQLCipher. The actual
 * passphrase capture and SQLCipher unlock live in the native sidecar fd
 * handoff, which is still on the live-actions backlog (see TODO.md). Until
 * that ships, `encrypted: true` does NOT mean the database is encrypted on
 * disk — it means the user opted in.
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
   * FIFO/LIFO/HIFO/LOFO; AT additionally exposes MOVING_AVERAGE and
   * MOVING_AVERAGE_AT (the AT default).
   */
  gainsAlgorithm?:
    | "FIFO"
    | "LIFO"
    | "HIFO"
    | "LOFO"
    | "MOVING_AVERAGE"
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
}

interface UiState {
  lang: Lang;
  currency: Currency;
  dataMode: DataMode;
  hideSensitive: boolean;
  identity: Identity | null;
  notifications: AppNotification[];
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setDataMode: (dataMode: DataMode) => void;
  setHideSensitive: (hideSensitive: boolean) => void;
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
      identity: null,
      notifications: [],
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setDataMode: (dataMode) => set({ dataMode }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
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
