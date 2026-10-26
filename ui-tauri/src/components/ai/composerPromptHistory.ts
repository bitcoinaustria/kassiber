/**
 * Terminal-style prompt recall for the chat composer. ArrowUp on an empty
 * composer walks back through the conversation's sent prompts; ArrowDown walks
 * forward and clears the composer past the newest entry.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/composerPromptHistory.ts`. Kassiber's user
 * messages hold only the typed text (screen context and attachments travel
 * separately), so T3's send-time stripping is not needed. History is derived
 * from the visible transcript on each keypress; nothing is stored.
 */

export interface ComposerPromptHistoryMessage {
  readonly id: string;
  readonly role: string;
  readonly content: string;
}

export interface ComposerPromptHistoryEntry {
  readonly id: string;
  readonly prompt: string;
}

/**
 * Active recall. `entryId` is resolved against the current entries on every
 * step, so a transcript refresh cannot move the position. `recalled` is the
 * text put in the composer; once the composer no longer matches it the user
 * has edited or sent, and browsing is over.
 */
export interface ComposerPromptHistoryPosition {
  readonly entryId: string;
  readonly recalled: string;
}

export interface ComposerPromptHistoryStep {
  readonly position: ComposerPromptHistoryPosition | null;
  readonly prompt: string;
}

/**
 * Oldest first. Consecutive identical prompts collapse into the newest one,
 * like shell `HISTCONTROL=ignoredups`.
 */
export function buildComposerPromptHistoryEntries(
  messages: ReadonlyArray<ComposerPromptHistoryMessage>,
): ComposerPromptHistoryEntry[] {
  const entries: ComposerPromptHistoryEntry[] = [];
  for (const message of messages) {
    if (message.role !== "user") continue;
    const prompt = message.content.trim();
    if (prompt.length === 0) continue;
    const previous = entries[entries.length - 1];
    if (previous && previous.prompt === prompt) {
      entries[entries.length - 1] = { id: message.id, prompt };
      continue;
    }
    entries.push({ id: message.id, prompt });
  }
  return entries;
}

function findActive(
  entries: ReadonlyArray<ComposerPromptHistoryEntry>,
  position: ComposerPromptHistoryPosition,
): number {
  const byId = entries.findIndex((entry) => entry.id === position.entryId);
  if (byId >= 0) return byId;
  // A duplicate collapse can retire the recalled id while the same text lives
  // on under a newer one.
  for (let index = entries.length - 1; index >= 0; index -= 1) {
    if (entries[index].prompt === position.recalled) return index;
  }
  return -1;
}

/**
 * Returns null when the key should fall through to normal caret movement.
 * Backward starts only from an empty composer and stops at the oldest entry.
 * Forward past the newest entry empties the composer and ends browsing.
 */
export function stepComposerPromptHistory(input: {
  readonly direction: "backward" | "forward";
  readonly entries: ReadonlyArray<ComposerPromptHistoryEntry>;
  readonly position: ComposerPromptHistoryPosition | null;
  readonly currentPrompt: string;
}): ComposerPromptHistoryStep | null {
  const { entries, position, currentPrompt } = input;
  const activeIndex =
    position && position.recalled === currentPrompt
      ? findActive(entries, position)
      : -1;

  if (input.direction === "backward") {
    if (activeIndex < 0 && currentPrompt.length > 0) return null;
    const entry = entries[activeIndex < 0 ? entries.length - 1 : activeIndex - 1];
    if (!entry) return null;
    return {
      position: { entryId: entry.id, recalled: entry.prompt },
      prompt: entry.prompt,
    };
  }

  if (activeIndex < 0) return null;
  const entry = entries[activeIndex + 1];
  if (!entry) return { position: null, prompt: "" };
  return {
    position: { entryId: entry.id, recalled: entry.prompt },
    prompt: entry.prompt,
  };
}

/**
 * Whether a collapsed caret sits on the first (`start`) or last (`end`)
 * logical line of a textarea, so ArrowUp/ArrowDown would otherwise leave the
 * text rather than move within it. A selection never counts.
 */
export function isCaretOnTextEdge(
  value: string,
  selectionStart: number,
  selectionEnd: number,
  edge: "start" | "end",
): boolean {
  if (selectionStart !== selectionEnd) return false;
  return edge === "start"
    ? !value.slice(0, selectionStart).includes("\n")
    : !value.slice(selectionEnd).includes("\n");
}
