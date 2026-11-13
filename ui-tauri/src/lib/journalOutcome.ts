import type { QueryClient } from "@tanstack/react-query";

import { readActiveBook } from "@/lib/activeBook";
import { journalStepSucceeded } from "@/lib/notificationConditions";
import { conditionKey, useNotificationStore } from "@/store/notifications";
import { useUiStore } from "@/store/ui";

/**
 * Any mutation whose result shows a successful journal step (a wallet sync,
 * an import, an exchange sync, a refresh) resolves an earlier journal
 * processing failure for the book it ran in. Kinds without a book binding
 * are attributed to the open book only if the daemon session is the one the
 * mutation started in; a switch since then proves nothing about this book.
 */
export function resolveJournalFailureAfter(
  queryClient: QueryClient,
  startedSession: number,
  data: unknown,
): void {
  if (!journalStepSucceeded(data)) return;
  if (useUiStore.getState().daemonSession !== startedSession) return;
  const active = readActiveBook(queryClient);
  if (!active) return;
  useNotificationStore
    .getState()
    .resolveCondition(conditionKey("journals.process-failed", { book: active.key }));
}
