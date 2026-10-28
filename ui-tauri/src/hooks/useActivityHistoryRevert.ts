import * as React from "react";
import { useTranslation } from "react-i18next";

import { useDaemonMutation } from "@/daemon/client";
import { confirmAction } from "@/lib/confirmAction";
import type { HistoryRevertTarget } from "@/lib/transactionHistory";
import { useUiStore } from "@/store/ui";

/**
 * Reverts an edit (or one field of it) from the book's edit history, as the
 * Activity page and the right panel's Changes surface both do: a new history
 * entry with the reverted value, and a notification either way. Confirming
 * first is the caller's.
 */
export function useActivityHistoryRevert() {
  const { t } = useTranslation("review");
  const addNotification = useUiStore((state) => state.addNotification);
  const mutation = useDaemonMutation("ui.transactions.history.revert");
  const { mutateAsync } = mutation;
  const revert = React.useCallback(
    async (target: HistoryRevertTarget) => {
      const dedupeKey = `activity-history-revert-${target.event.id}-${target.field?.field ?? "event"}`;
      try {
        await mutateAsync({
          transaction: target.event.transaction_id,
          event: target.event.id,
          ...(target.field ? { field: target.field.field } : {}),
          reason: target.field
            ? t("activity.revert.reasonField", { label: target.field.label })
            : t("activity.revert.reasonEvent"),
        });
      } catch (error) {
        addNotification({
          title: t("activity.revert.failedTitle"),
          body: error instanceof Error && error.message ? error.message : t("activity.revert.failedBody"),
          tone: "error",
          dedupeKey,
        });
        return;
      }
      addNotification({
        title: t("activity.revert.title"),
        body: t("activity.revert.body"),
        tone: "success",
        dedupeKey,
      });
    },
    [addNotification, mutateAsync, t],
  );
  return { revert, isReverting: mutation.isPending };
}

/**
 * Asks before reverting an edit or one of its fields, then hands the target
 * on. `t` reads the `transactions` namespace.
 */
export async function confirmRevert(
  target: HistoryRevertTarget,
  t: (key: string, opts?: Record<string, unknown>) => string,
  onRevert?: (target: HistoryRevertTarget) => void,
) {
  if (!onRevert) return;
  const fieldLabel = target.field
    ? target.field.label
    : t("history.confirmRevertAll");
  const ok = await confirmAction(
    t("history.confirmRevertField", { label: fieldLabel }),
  );
  if (!ok) return;
  onRevert(target);
}
