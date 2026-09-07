import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
/** Recover missed wake-ups after lock/restart; never persist evidence in toasts. */
export function useLocalWatchNotifications(enabled: boolean) {
    const { t } = useTranslation("chainAnalysis");
    const inbox = useDaemon<{
        unread_count: number;
    }>("ui.chain_analysis.watches.inbox", { limit: 1 }, { enabled, refetchInterval: enabled ? 15000 : false });
    const unread = inbox.data?.data?.unread_count;
    useEffect(() => {
        if (!enabled || unread === undefined)
            return;
        const store = useUiStore.getState();
        const existing = store.notifications.find(item => item.dedupeKey === "local-evidence-watches");
        if (unread > 0) {
            store.addNotification({ title: t("watch.notificationTitle"), body: t("watch.notificationBody"), tone: "info", dedupeKey: "local-evidence-watches", target: "/chain-analysis" });
        }
        else if (existing)
            store.clearNotification(existing.id);
    }, [enabled, unread, t]);
}
