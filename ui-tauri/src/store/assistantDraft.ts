import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

/**
 * Unsent assistant composer text.
 *
 * Deliberately backed by `sessionStorage`, not the persisted `kb.ui`
 * (localStorage) store: a typed-but-unsent prompt should survive a page
 * reload/navigation, but it must not be written to disk past the app session.
 * Chat content lives behind the SQLCipher boundary; a raw draft (which may
 * contain sensitive accounting details, including in Incognito) stays in the
 * volatile session store and is gone when the window closes.
 */
interface AssistantDraftState {
  draft: string;
  setDraft: (draft: string) => void;
}

export const useAssistantDraftStore = create<AssistantDraftState>()(
  persist(
    (set) => ({
      draft: "",
      setDraft: (draft) => set({ draft }),
    }),
    {
      name: "kb.assistantDraft",
      storage: createJSONStorage(() => sessionStorage),
    },
  ),
);
