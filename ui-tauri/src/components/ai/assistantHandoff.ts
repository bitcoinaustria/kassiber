import { currentPageScreenContext } from "@/components/ai/assistantScreenContext";
import type {
  AssistantScreenContext,
  AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import type { BookStamp } from "@/components/ai/bookStamp";
import type { DaemonScopeBoundary } from "@/daemon/client";
import i18n from "@/i18n";
import { useUiStore } from "@/store/ui";

interface HandoffOrigin {
  /** The page that asks; defaults to the page on screen now. */
  context?: AssistantScreenContext;
  /** The page's scope boundary: its book is the one the page's content came from. */
  boundary?: DaemonScopeBoundary | null;
}

/**
 * Begin a page's own request to the assistant (the quarantine fix, Source of
 * Funds, chain analysis, PSBT and datasets) at the user's action. Call it
 * synchronously in the handler, before any await: it captures the page and
 * the book the page's content belongs to (its scope boundary's, or the book
 * open now). The returned function hands a prompt over with that capture,
 * however long building the prompt took, so a book opened meanwhile never
 * receives it (see `BookStamp`).
 *
 * Returns null, with a notice, while the book's ids are not known: the action
 * is not taken. Without a chosen model the session parks the prompt in the
 * composer, without the pin, until it is sent or the draft is replaced.
 */
export function beginAssistantHandoff(
  assistant: AssistantSessionContextValue,
  origin: HandoffOrigin = {},
): ((prompt: string) => void) | null {
  const book: BookStamp | null = origin.boundary
    ? { daemonSession: origin.boundary.daemonSession, scope: origin.boundary.expectedScope }
    : (assistant.captureBook?.() ?? null);
  if (!book) {
    useUiStore.getState().addNotification({
      title: i18n.t("queue.bookLoading.title", { ns: "assistant" }),
      body: i18n.t("queue.bookLoading.body", { ns: "assistant" }),
      tone: "warning",
      dedupeKey: "assistant-book-loading",
    });
    return null;
  }
  const context = origin.context ?? currentPageScreenContext() ?? undefined;
  return (prompt) => {
    useUiStore.getState().openAssistantPanel();
    assistant.sendPrompt(prompt, { ...(context ? { context } : {}), book });
  };
}

/** A handoff whose prompt is built in the same handler, with no await before it. */
export function handOffToAssistant(
  assistant: AssistantSessionContextValue,
  prompt: string,
  origin: HandoffOrigin = {},
) {
  beginAssistantHandoff(assistant, origin)?.(prompt);
}
