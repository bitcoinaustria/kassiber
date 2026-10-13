import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import type { AssistantScreenContext } from "@/components/ai/assistantSession";

/**
 * A page's own request to the assistant (see `AssistantHandoff`) that waits
 * in the composer because no model was chosen yet. It keeps the screen
 * context of the page that made it, and the book it was made in, so sending
 * the draft later from another page still gets that page's tool pack.
 */
export interface ParkedHandoff {
  context: AssistantScreenContext;
  daemonSession: number;
}

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
  /** A handoff parked with the draft; edits keep it, clearing drops it. */
  handoff: ParkedHandoff | null;
  /** The user's own edit of the draft. */
  setDraft: (draft: string) => void;
  /** New text from elsewhere (Ask about this, a rewound prompt, a suggestion). */
  replaceDraft: (draft: string) => void;
  /** Park a handoff's prompt and context until a model is chosen. */
  parkHandoff: (prompt: string, handoff: ParkedHandoff) => void;
  /** Take the parked handoff for the send that is happening now. */
  takeHandoff: () => ParkedHandoff | null;
}

export const useAssistantDraftStore = create<AssistantDraftState>()(
  persist(
    (set, get) => ({
      draft: "",
      handoff: null,
      setDraft: (draft) =>
        set((state) => ({
          draft,
          handoff: draft.trim() ? state.handoff : null,
        })),
      replaceDraft: (draft) => set({ draft, handoff: null }),
      parkHandoff: (prompt, handoff) => set({ draft: prompt, handoff }),
      takeHandoff: () => {
        const { handoff } = get();
        if (handoff) set({ handoff: null });
        return handoff;
      },
    }),
    {
      name: "kb.assistantDraft",
      storage: createJSONStorage(() => sessionStorage),
    },
  ),
);
