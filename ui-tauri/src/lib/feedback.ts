import { create } from "zustand";

/**
 * Transient feedback: a toast that confirms what the user just did. It never
 * enters the notification center, never reaches disk, and is never forwarded
 * to the operating system. Form errors belong inline, beside the form;
 * anything the user still has to act on belongs in the center as a
 * condition.
 */
export type FeedbackTone = "success" | "info" | "warning" | "error";

export interface FeedbackInput {
  tone: FeedbackTone;
  /**
   * Fixed, translated text with no amounts, labels, paths, or raw errors:
   * it shows in every privacy mode.
   */
  title: string;
  description?: string;
  /**
   * `safe` marks a description built like the title. Anything else is free
   * text: Hide all drops it and Hide amounts withholds it if it contains a
   * digit.
   */
  descriptionPrivacy?: "safe" | "free-text";
  /**
   * Equivalent receipts share a key; a new one replaces the visible toast
   * and restarts its timer instead of stacking. Defaults to the tone and
   * text.
   */
  key?: string;
  /** Milliseconds before it expires; defaults by tone. */
  durationMs?: number;
}

export interface FeedbackItem extends Required<Pick<FeedbackInput, "tone" | "title">> {
  id: string;
  key: string;
  description?: string;
  descriptionPrivacy: "safe" | "free-text";
  durationMs: number;
  /** Bumped when an equivalent receipt arrives; restarts the timer. */
  revision: number;
  /** How many equivalent receipts this toast stands for. */
  count: number;
}

/** At most this many toasts at once; a new one retires the oldest. */
export const MAX_VISIBLE_FEEDBACK = 3;
export const SUCCESS_FEEDBACK_MS = 4_000;
const ATTENTION_FEEDBACK_MS = 8_000;

interface FeedbackState {
  items: FeedbackItem[];
}

export const useFeedbackStore = create<FeedbackState>()(() => ({ items: [] }));

let sequence = 0;

function defaultDuration(tone: FeedbackTone): number {
  return tone === "warning" || tone === "error"
    ? ATTENTION_FEEDBACK_MS
    : SUCCESS_FEEDBACK_MS;
}

/** Show a toast; returns its id. */
export function showFeedback(input: FeedbackInput): string {
  const key =
    input.key ?? JSON.stringify([input.tone, input.title, input.description ?? ""]);
  const { items } = useFeedbackStore.getState();
  const existing = items.find((item) => item.key === key);
  if (existing) {
    useFeedbackStore.setState({
      items: items.map((item) =>
        item.id === existing.id
          ? {
              ...item,
              tone: input.tone,
              title: input.title,
              description: input.description,
              descriptionPrivacy: input.descriptionPrivacy ?? "free-text",
              durationMs: input.durationMs ?? defaultDuration(input.tone),
              revision: item.revision + 1,
              count: item.count + 1,
            }
          : item,
      ),
    });
    return existing.id;
  }
  sequence += 1;
  const item: FeedbackItem = {
    id: `feedback-${sequence}`,
    key,
    tone: input.tone,
    title: input.title,
    description: input.description,
    descriptionPrivacy: input.descriptionPrivacy ?? "free-text",
    durationMs: input.durationMs ?? defaultDuration(input.tone),
    revision: 0,
    count: 1,
  };
  useFeedbackStore.setState({
    items: [...items, item].slice(-MAX_VISIBLE_FEEDBACK),
  });
  return item.id;
}

export function dismissFeedback(id: string): void {
  useFeedbackStore.setState((state) => ({
    items: state.items.filter((item) => item.id !== id),
  }));
}

/** Drops every toast at once, e.g. when the app locks. */
export function clearFeedback(): void {
  useFeedbackStore.setState({ items: [] });
}
