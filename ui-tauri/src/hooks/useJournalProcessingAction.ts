import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

export type JournalProcessResult = {
  profile?: string;
  entries_created?: number;
  quarantined?: number;
  transfers_detected?: number;
  cross_asset_pairs?: number;
  auto_priced?: number;
  processed_transactions?: number;
  processed_at?: string;
};

type JournalProcessingActionOptions = {
  entryLabel?: "entries" | "events";
  notifyStart?: boolean;
  notifyAlreadyRunning?: boolean;
  beforeRun?: () => boolean;
};

function journalProcessBody(
  payload: JournalProcessResult | undefined,
  entryLabel: "entries" | "events",
) {
  const parts = [
    payload?.processed_transactions !== undefined
      ? `${payload.processed_transactions} transactions`
      : null,
    payload?.entries_created !== undefined
      ? `${payload.entries_created} ${entryLabel}`
      : null,
    payload?.quarantined !== undefined
      ? `${payload.quarantined} quarantined`
      : null,
  ].filter(Boolean);
  return parts.join(", ") || "Journal state refreshed.";
}

export function useJournalProcessingAction(
  options: JournalProcessingActionOptions = {},
) {
  const {
    entryLabel = "entries",
    notifyStart = false,
    notifyAlreadyRunning = false,
    beforeRun,
  } = options;
  const dataMode = useUiStore((s) => s.dataMode);
  const addNotification = useUiStore((s) => s.addNotification);
  const queryClient = useQueryClient();
  const processJournals =
    useDaemonMutation<JournalProcessResult>("ui.journals.process");
  const mutationKey = daemonMutationKey(dataMode, "ui.journals.process");
  const activeJournalRuns = useIsMutating({ mutationKey });
  const isProcessingJournals =
    processJournals.isPending || activeJournalRuns > 0;

  const runJournalProcessing = React.useCallback(() => {
    if (beforeRun && !beforeRun()) return;
    if (processJournals.isPending || activeJournalRuns > 0) {
      if (notifyAlreadyRunning) {
        addNotification({
          title: "Journal processing already running",
          body: "Kassiber is already refreshing the journal state.",
          tone: "info",
          dedupeKey: "journal-processing",
        });
      }
      return;
    }
    if (notifyStart) {
      addNotification({
        title: "Journal processing started",
        body: "Kassiber is rebuilding report-ready journal state.",
        tone: "warning",
        dedupeKey: "journal-processing",
      });
    }
    processJournals.mutate(undefined, {
      onSuccess: (envelope) => {
        const payload = envelope.data;
        addNotification({
          title: "Journals processed",
          body: journalProcessBody(payload, entryLabel),
          tone: payload?.quarantined ? "warning" : "success",
          dedupeKey: "journal-processing",
        });
      },
      onError: (error) => {
        addNotification({
          title: "Journal processing failed",
          body:
            error instanceof Error
              ? error.message
              : "Could not process journals.",
          tone: "error",
          dedupeKey: "journal-processing",
        });
      },
      onSettled: () => {
        void queryClient.invalidateQueries({ queryKey: ["daemon"] });
      },
    });
  }, [
    activeJournalRuns,
    addNotification,
    beforeRun,
    entryLabel,
    notifyAlreadyRunning,
    notifyStart,
    processJournals,
    queryClient,
  ]);

  return {
    runJournalProcessing,
    isProcessingJournals,
  };
}
