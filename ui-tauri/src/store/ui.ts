import { create } from "zustand";
import { persist } from "zustand/middleware";

type Lang = "en" | "de";
type Currency = "btc" | "eur";

interface UiState {
  lang: Lang;
  currency: Currency;
  hideSensitive: boolean;
  setLang: (lang: Lang) => void;
  setCurrency: (currency: Currency) => void;
  setHideSensitive: (hideSensitive: boolean) => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      lang: "en",
      currency: "btc",
      hideSensitive: false,
      setLang: (lang) => set({ lang }),
      setCurrency: (currency) => set({ currency }),
      setHideSensitive: (hideSensitive) => set({ hideSensitive }),
    }),
    { name: "kb.ui" },
  ),
);
