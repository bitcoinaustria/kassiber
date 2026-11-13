import { useEffect } from "react";

import { useDaemon } from "@/daemon/client";
import {
  conditionKey,
  useNotificationStore,
  type ConditionInput,
} from "@/store/notifications";
import { bookIdentityKey, useUiStore } from "@/store/ui";

/**
 * The evidence-watch condition for `book`: unread changes wait under
 * Watches in Chain Analysis. It carries no evidence, only that there is
 * something to read there.
 */
export function watchCondition(book: string | null): ConditionInput {
  return {
    code: "watch.evidence-changed",
    scope: { book },
    severity: "info",
    priority: "attention",
    action: { type: "navigate", to: "/chain-analysis" },
  };
}

/**
 * Recover missed wake-ups after lock or restart from the local watch inbox:
 * unread changes keep the condition, an inbox read empty resolves it. A
 * reconcile, not a new occurrence: polling the same unread count never
 * counts up. Only a successful read decides anything.
 */
export function useLocalWatchNotifications(enabled: boolean) {
  const inbox = useDaemon<{
    unread_count: number;
  }>("ui.chain_analysis.watches.inbox", { limit: 1 }, { enabled, refetchInterval: enabled ? 15000 : false });
  const unread = inbox.isSuccess ? inbox.data?.data?.unread_count : undefined;
  const identity = useUiStore((state) => state.identity);
  const book = bookIdentityKey(identity);
  useEffect(() => {
    if (!enabled || unread === undefined) return;
    const condition = watchCondition(book);
    useNotificationStore.getState().reconcileConditions(
      unread > 0
        ? { present: [condition] }
        : { resolved: [conditionKey(condition.code, condition.scope)] },
    );
  }, [book, enabled, unread]);
}
