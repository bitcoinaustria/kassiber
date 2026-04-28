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

export interface Identity {
  name: string;
  workspace: string;
  country: string;
  encrypted: boolean;
  profile?: string;
  taxCountry?: "at" | "generic";
  fiatCurrency?: string;
  taxLongTermDays?: number;
  gainsAlgorithm?: "FIFO" | "LIFO" | "HIFO" | "LOFO";
  databaseMode?: "sqlcipher" | "plaintext";
  migrateCredentials?: boolean;
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
