import { create } from "zustand";
import { persist } from "zustand/middleware";

type Lang = "en" | "de";
type Currency = "btc" | "eur";

export interface Identity {
  name: string;
  workspace: string;
  country: string;
  encrypted: boolean;
}

interface UiState {
  lang: Lang;
  currency: Currency;
  hideSensitive: boolean;
  identity: Identity | null;
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setHideSensitive: (hideSensitive: boolean) => void;
  setIdentity: (identity: Identity | null) => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      hideSensitive: false,
      identity: null,
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
      setIdentity: (identity) => set({ identity }),
    }),
    { name: "kb.ui" },
  ),
);
