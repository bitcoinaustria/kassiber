import * as React from "react";
import { useIsMutating } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import {
  conditionKey,
  useNotificationStore,
  type NotificationDetail,
  type NotificationParams,
  type NotificationScope,
} from "@/store/notifications";
import { bookIdentityKey, useUiStore } from "@/store/ui";

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
  auto_pair?: {
    applied?: number;
    skipped?: boolean;
    error?: { code?: string; message?: string } | null;
  } | null;
};

type JournalProcessProgress = {
  phase?: string;
};

function journalProcessPhaseKey(phase: string | undefined) {
  switch (phase) {
    case "writer_wait":
      return "processing.phase.writerWait" as const;
    case "auto_pair":
      return "processing.phase.autoPair" as const;
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
    tone:
      warnings.length || payload?.quarantined || payload?.auto_pair?.skipped
        ? "warning"
        : "success",
  };
}

type JournalProcessingActionOptions = {
  beforeRun?: () => boolean;
  /**
   * False recalculates without pairing exact matches automatically, for a
   * run that follows the owner's own decisions.
   */
  autoPair?: boolean;
};

/** One journal run at a time, so the running job has one id. */
const JOURNAL_JOB_ID = "journal-processing";

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
    payload?.auto_pair?.applied
      ? `${payload.auto_pair.applied} exact transfer pair${payload.auto_pair.applied === 1 ? "" : "s"} applied`
      : null,
    payload?.auto_pair?.skipped ? "automatic pairing skipped" : null,
  ].filter(Boolean);
  return parts.join(", ") || "Journal state refreshed.";
}

/**
 * Journal processing as a tracked job. Starting it adds a Running row (and
 * the shell's progress line); finishing it leaves one Recent result and a
 * toast. A failure becomes a condition whose Retry repeats this run with the
 * same options, for the book it ran in. A second request while a run is in
 * flight does nothing: the running job is already visible.
 *
 * The outcome is handled on the mutation's promise rather than per-call
 * callbacks, so it lands even if the surface that started it unmounts.
 */
export function useJournalProcessingAction(
  options: JournalProcessingActionOptions = {},
) {
  const { t } = useTranslation("journals");
  const { beforeRun, autoPair = true } = options;
  const dataMode = useUiStore((s) => s.dataMode);
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
      const phaseKey = journalProcessPhaseKey(progress.phase);
      const phaseLabel = t(phaseKey);
      useNotificationStore.getState().updateJob(JOURNAL_JOB_ID, {
        indeterminate: true,
        phaseKey: `journals:${phaseKey}`,
      });
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
        state: "running",
        startedAt: current?.startedAt ?? now,
        updatedAt: now,
      });
    },
  });
  const mutationKey = daemonMutationKey(dataMode, "ui.journals.process");
  const activeJournalRuns = useIsMutating({ mutationKey });
  const isProcessingJournals =
    processJournals.isPending || activeJournalRuns > 0;

  /** Run with explicit options, e.g. a Retry repeating the original run. */
  const runJournalProcessingWith = React.useCallback(
    ({ autoPair: runAutoPair }: { autoPair: boolean }) => {
      if (beforeRun && !beforeRun()) return;
      if (processJournals.isPending || activeJournalRuns > 0) return;
      const scope: NotificationScope = {
        book: bookIdentityKey(useUiStore.getState().identity),
      };
      const notifications = useNotificationStore.getState();
      notifications.startJob({
        id: JOURNAL_JOB_ID,
        code: "journal-processing",
        scope,
        progress: {
          indeterminate: true,
          phaseKey: "journals:processing.phase.fallback",
        },
      });
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
        state: "running",
        startedAt,
        updatedAt: startedAt,
      });
      processJournals
        .mutateAsync(runAutoPair ? undefined : { auto_pair: false })
        .then(
          (envelope) => recordJournalSuccess(envelope.data, scope),
          (error: unknown) => recordJournalFailure(error, scope, runAutoPair),
        )
        .finally(() => {
          useNotificationStore.getState().endJob(JOURNAL_JOB_ID);
          clearActiveMaintenanceProgress(JOURNAL_PROCESSING_PROGRESS_ID);
        });
    },
    [
      activeJournalRuns,
      beforeRun,
      clearActiveMaintenanceProgress,
      processJournals,
      setActiveMaintenanceProgress,
      t,
    ],
  );
  const runJournalProcessing = React.useCallback(
    () => runJournalProcessingWith({ autoPair }),
    [autoPair, runJournalProcessingWith],
  );

  return {
    runJournalProcessing,
    runJournalProcessingWith,
    isProcessingJournals,
  };
}

/** A finished run: one Recent result, a toast, and any earlier failure resolved. */
export function recordJournalSuccess(
  payload: JournalProcessResult | undefined,
  scope: NotificationScope,
): void {
  const store = useNotificationStore.getState();
  store.resolveCondition(conditionKey("journals.process-failed", scope));
  const warnings = payload?.warnings ?? [];
  const autoPairSkipped = Boolean(payload?.auto_pair?.skipped);
  const needsLook = warnings.length > 0 || autoPairSkipped;
  const quarantined = payload?.quarantined ?? 0;
  const details: NotificationDetail[] = [
    ...warnings.map((warning) => ({
      message: safeMessage(warning.message ?? warning.code),
    })),
    ...(autoPairSkipped
      ? [
          {
            phraseKey: "autoPairSkipped" as const,
            message: safeMessage(payload?.auto_pair?.error?.message),
          },
        ]
      : []),
  ];
  const result = {
    code: "journals.processed" as const,
    scope,
    severity: needsLook || quarantined ? ("warning" as const) : ("info" as const),
    params: {
      transactions: payload?.processed_transactions,
      entries: payload?.entries_created,
      quarantined,
      autoPaired: payload?.auto_pair?.applied ?? 0,
      warnings: warnings.length,
    },
    // Quarantine already has its live Review condition; warnings and a
    // skipped pairing are explained on Journals.
    action: needsLook
      ? ({ type: "navigate", to: "/journals" } as const)
      : undefined,
    details,
  };
  store.addResult(result);
  showFeedback({
    tone: result.severity === "warning" ? "warning" : "success",
    title: i18n.t("chrome:notifications.result.journalsProcessed.title"),
    description: journalCountsText(result.params),
    descriptionPrivacy: "safe",
    key: "journals.processed",
  });
}

/** A failed run: a condition whose Retry repeats this run's options. */
export function recordJournalFailure(
  error: unknown,
  scope: NotificationScope,
  autoPair: boolean,
): void {
  useNotificationStore.getState().raiseCondition({
    code: "journals.process-failed",
    scope,
    severity: "error",
    priority: "attention",
    action: {
      type: "operation",
      operation: { kind: "process-journals", autoPair },
      review: "/journals",
    },
    details: [
      {
        message: safeMessage(
          error instanceof Error ? error.message : undefined,
        ),
      },
    ],
  });
  showFeedback({
    tone: "error",
    title: i18n.t("chrome:notifications.condition.journalsFailed.title"),
    key: "journals.process-failed",
  });
}

function journalCountsText(params: NotificationParams): string | undefined {
  const parts = [
    params.transactions !== undefined
      ? i18n.t("chrome:notifications.result.journalsProcessed.transactions", {
          count: params.transactions,
        })
      : null,
    params.entries !== undefined
      ? i18n.t("chrome:notifications.result.journalsProcessed.entries", {
          count: params.entries,
        })
      : null,
    params.quarantined
      ? i18n.t("chrome:notifications.result.journalsProcessed.quarantined", {
          count: params.quarantined,
        })
      : null,
  ].filter((part): part is string => Boolean(part));
  return parts.length ? parts.join(", ") : undefined;
}
