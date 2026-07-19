import * as React from "react";
import { useIsMutating } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

const JOURNAL_PROCESSING_PROGRESS_ID = "journal-processing";

export type JournalProcessWarning = {
  code?: string;
  label?: string;
  wallet_ids?: string[];
  message?: string;
};

export type JournalProcessResult = {
  profile?: string;
  entries_created?: number;
  quarantined?: number;
  transfers_detected?: number;
  cross_asset_pairs?: number;
  auto_priced?: number;
  processed_transactions?: number;
  processed_at?: string;
  warnings?: JournalProcessWarning[];
};

type JournalProcessProgress = {
  phase?: string;
};

function journalProcessPhaseKey(phase: string | undefined) {
  switch (phase) {
    case "writer_wait":
      return "processing.phase.writerWait" as const;
    case "preparing":
      return "processing.phase.preparing" as const;
    case "repairing":
      return "processing.phase.repairing" as const;
    case "pricing":
      return "processing.phase.pricing" as const;
    case "building":
      return "processing.phase.building" as const;
    case "storing":
      return "processing.phase.storing" as const;
    case "complete":
      return "processing.phase.complete" as const;
    default:
      return "processing.phase.fallback" as const;
  }
}

export function warningSummary(warnings: JournalProcessWarning[]) {
  const first = warnings[0]?.message ?? "Review required";
  return warnings.length > 1
    ? `${warnings.length} warnings — ${first}`
    : first;
}

/** Notification body + tone for a finished journal-processing run. */
export function journalProcessOutcome(
  payload: JournalProcessResult | undefined,
  entryLabel: "entries" | "events" = "entries",
): { body: string; tone: "success" | "warning" } {
  const warnings = payload?.warnings ?? [];
  const summary = journalProcessBody(payload, entryLabel);
  return {
    body: warnings.length
      ? `${summary} — ${warningSummary(warnings)}`
      : summary,
    // A non-blocking warning (e.g. duplicate wallet labels merging per-wallet
    // attribution) must not read as a clean success.
    tone: warnings.length || payload?.quarantined ? "warning" : "success",
  };
}

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
  const { t } = useTranslation("journals");
  const {
    entryLabel = "entries",
    notifyStart = false,
    notifyAlreadyRunning = false,
    beforeRun,
  } = options;
  const dataMode = useUiStore((s) => s.dataMode);
  const addNotification = useUiStore((s) => s.addNotification);
  const setActiveMaintenanceProgress = useUiStore(
    (s) => s.setActiveMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (s) => s.clearActiveMaintenanceProgress,
  );
  const processJournals = useDaemonStreamMutation<
    JournalProcessResult,
    JournalProcessProgress
  >("ui.journals.process", {
    onProgress: (progress) => {
      const now = new Date().toISOString();
      const current = useUiStore.getState().activeMaintenanceProgress;
      const phaseLabel = t(journalProcessPhaseKey(progress.phase));
      setActiveMaintenanceProgress({
        id: JOURNAL_PROCESSING_PROGRESS_ID,
        title: t("processing.title"),
        body: phaseLabel,
        tone: "warning",
        progress: {
          indeterminate: true,
          label: phaseLabel,
        },
        details: [t("processing.reportHint")],
        active: true,
        startedAt: current?.startedAt ?? now,
        updatedAt: now,
      });
    },
  });
  const mutationKey = daemonMutationKey(dataMode, "ui.journals.process");
  const activeJournalRuns = useIsMutating({ mutationKey });
  const isProcessingJournals =
    processJournals.isPending || activeJournalRuns > 0;

  const runJournalProcessing = React.useCallback(() => {
    if (beforeRun && !beforeRun()) return;
    if (processJournals.isPending || activeJournalRuns > 0) {
      if (notifyAlreadyRunning) {
        addNotification({
          title: t("processing.alreadyTitle"),
          body: t("processing.alreadyBody"),
          tone: "info",
          dedupeKey: "journal-processing",
        });
      }
      return;
    }
    if (notifyStart) {
      addNotification({
        title: t("processing.startedTitle"),
        body: t("processing.startedBody"),
        tone: "warning",
        dedupeKey: "journal-processing",
      });
    }
    const startedAt = new Date().toISOString();
    setActiveMaintenanceProgress({
      id: JOURNAL_PROCESSING_PROGRESS_ID,
      title: t("processing.title"),
      body: t("processing.startedBody"),
      tone: "warning",
      progress: {
        indeterminate: true,
        label: t("processing.phase.fallback"),
      },
      details: [t("processing.reportHint")],
      active: true,
      startedAt,
      updatedAt: startedAt,
    });
    processJournals.mutate(undefined, {
      onSuccess: (envelope) => {
        const { body, tone } = journalProcessOutcome(envelope.data, entryLabel);
        addNotification({
          title: t("processing.successTitle"),
          body,
          tone,
          dedupeKey: "journal-processing",
        });
        clearActiveMaintenanceProgress(JOURNAL_PROCESSING_PROGRESS_ID);
      },
      onError: (error) => {
        addNotification({
          title: t("processing.failedTitle"),
          body:
            error instanceof Error
              ? error.message
              : t("processing.failedBody"),
          tone: "error",
          dedupeKey: "journal-processing",
        });
        clearActiveMaintenanceProgress(JOURNAL_PROCESSING_PROGRESS_ID);
      },
    });
  }, [
    activeJournalRuns,
    addNotification,
    beforeRun,
    clearActiveMaintenanceProgress,
    entryLabel,
    notifyAlreadyRunning,
    notifyStart,
    processJournals,
    setActiveMaintenanceProgress,
    t,
  ]);

  return {
    runJournalProcessing,
    isProcessingJournals,
  };
}
