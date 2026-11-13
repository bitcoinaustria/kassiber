import * as React from "react";

import { useActiveBook } from "@/lib/activeBook";
import {
  useNotificationStore,
  type ConditionEntry,
} from "@/store/notifications";

/**
 * Sources whose failed refresh blocks reports in `book`: the very conditions
 * the bell shows (reconciled from `ui.freshness.status`), so the readiness
 * pill and the bell can never disagree about them.
 */
export function blockingSourceCount(
  conditions: Record<string, ConditionEntry>,
  book: string | null,
): number {
  if (book === null) return 0;
  return Object.values(conditions).filter(
    (entry) =>
      entry.code === "source.refresh-failed" &&
      entry.priority === "blocking" &&
      entry.scope.book === book,
  ).length;
}

/** `blockingSourceCount` for the open book. */
export function useBlockingSourceCount(): number {
  const book = useActiveBook(true)?.key ?? null;
  const conditions = useNotificationStore((s) => s.conditions);
  return React.useMemo(() => blockingSourceCount(conditions, book), [conditions, book]);
}
