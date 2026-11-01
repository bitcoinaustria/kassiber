import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { invalidateDaemonQueriesForMutation } from "@/daemon/client";
import { getTransport } from "@/daemon/transport";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useUiStore } from "@/store/ui";

/** How long a run waits for further decisions before it starts. */
const SETTLE_MS = 800;

/**
 * Recalculates the journals once the owner stops deciding, so a decision
 * shows up in the counts without a separate step. Processing is local and
 * makes no network request, and it pairs nothing automatically: only what
 * the owner decided changes. A decision made while a run is going schedules
 * one more run after it, and one still waiting when the screen closes runs
 * anyway.
 */
export function useJournalRefreshAfterDecisions() {
  const { runJournalProcessing, isProcessingJournals } = useJournalProcessingAction({
    autoPair: false,
  });
  const queryClient = useQueryClient();
  const dataMode = useUiStore((s) => s.dataMode);
  const [pending, setPending] = React.useState(false);
  const pendingRef = React.useRef(false);
  pendingRef.current = pending;
  const scope = React.useRef({ queryClient, dataMode });
  scope.current = { queryClient, dataMode };

  React.useEffect(() => {
    if (!pending || isProcessingJournals) return;
    const timer = window.setTimeout(() => {
      setPending(false);
      runJournalProcessing();
    }, SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [isProcessingJournals, pending, runJournalProcessing]);

  // Leaving before the run started: run it without the progress row, which
  // belongs to this screen, and refresh what reads the journals after.
  React.useEffect(
    () => () => {
      if (!pendingRef.current) return;
      const { queryClient: client, dataMode: mode } = scope.current;
      void getTransport()
        .stream({ kind: "ui.journals.process", args: { auto_pair: false } })
        .catch(() => undefined)
        .finally(() => invalidateDaemonQueriesForMutation(client, mode, "ui.journals.process"));
    },
    [],
  );

  const requestRefresh = React.useCallback(() => {
    setPending(true);
    // The sidebar and the bell show the journals as stale at once.
    void queryClient.invalidateQueries({
      queryKey: ["daemon", dataMode],
      predicate: (query) => query.queryKey.includes("ui.review.badges"),
    });
  }, [dataMode, queryClient]);
  return { requestRefresh, runJournalProcessing, isProcessingJournals, refreshPending: pending };
}
