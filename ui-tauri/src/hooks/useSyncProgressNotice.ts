import * as React from "react";

import { useUiStore } from "@/store/ui";

const LONG_SYNC_NOTICE_MS = 8_000;

export function useSyncProgressNotice() {
  const addNotification = useUiStore((state) => state.addNotification);
  const timerRef = React.useRef<number | null>(null);

  const clearSyncNotice = React.useCallback(() => {
    if (timerRef.current === null) return;
    window.clearTimeout(timerRef.current);
    timerRef.current = null;
  }, []);

  const startSyncNotice = React.useCallback(
    (body = "Live wallet sync is still running. Large descriptors or slow backends can take a bit; Kassiber will update when the daemon returns.") => {
      clearSyncNotice();
      timerRef.current = window.setTimeout(() => {
        addNotification({
          title: "Wallet sync still running",
          body,
          tone: "info",
        });
        timerRef.current = null;
      }, LONG_SYNC_NOTICE_MS);
    },
    [addNotification, clearSyncNotice],
  );

  React.useEffect(() => clearSyncNotice, [clearSyncNotice]);

  return { startSyncNotice, clearSyncNotice };
}
