import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { applyFreshnessRun, safeMessage } from "@/lib/notificationConditions";
import {
  describeJournalStep,
  freshnessRunNeedsAttention,
  freshnessRunHasPendingJobs,
  freshnessRunNeedsBackend,
  freshnessRunOnlyJournalsPending,
  journalStepNeedsAttention,
  freshnessRunQuarantineCount,
  summarizeFreshnessRun,
  type FreshnessRunData,
} from "@/lib/syncResults";
import {
  activeSyncMaintenanceProgress,
  BOOK_REFRESH_PROGRESS_ID,
  failedSyncProgressForRun,
  STARTING_SYNC_PROGRESS_VALUE,
  startingSyncProgress,
  failedSyncMaintenanceProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import {
  bindRunBook,
  isStaleBookError,
  reportBookUnknown,
  reportStaleBook,
  type DaemonBookScope,
} from "@/lib/activeBook";
import {
  conditionKey,
  useNotificationStore,
  type NotificationScope,
} from "@/store/notifications";
import { newActivityFromRun, recordNewActivity } from "@/lib/newActivity";
import { bookIdentityKey, useUiStore } from "@/store/ui";

/** One book refresh at a time, so the running job has one id. */
const BOOK_REFRESH_JOB_ID = "book-refresh";

type WalletSyncOptions = {
  onTrustedSuccess?: () => void;
  forceFull?: boolean;
  /**
   * The book a Retry belongs to. Sent as `expected_scope`, so the daemon
   * refuses the run if another book is open by then. Omitted, the run binds
   * the book open now.
   */
  expectedScope?: DaemonBookScope;
};

export function useWalletSyncAction() {
  const { t } = useTranslation("overview");
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const freshnessRunMutationKey = React.useMemo(
    () => daemonMutationKey(dataMode, "ui.freshness.run"),
    [dataMode],
  );
  const walletSyncMutationKey = React.useMemo(
    () => daemonMutationKey(dataMode, "ui.wallets.sync"),
    [dataMode],
  );
  const freshnessRunsInFlight = useIsMutating({
    mutationKey: freshnessRunMutationKey,
  });
  const walletSyncsInFlight = useIsMutating({ mutationKey: walletSyncMutationKey });
  const setActiveMaintenanceProgress = useUiStore(
    (state) => state.setActiveMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (state) => state.clearActiveMaintenanceProgress,
  );
  const markFirstSyncDone = useUiStore((state) => state.markFirstSyncDone);
  const startedAtRef = React.useRef<string | null>(null);
  const progressValueRef = React.useRef(STARTING_SYNC_PROGRESS_VALUE);
  const lastProgressRef = React.useRef<WalletSyncProgress | null>(null);
  const jobProgressRef = React.useRef(new Map<string, WalletSyncProgress>());
  const refreshBook = useDaemonStreamMutation<
    FreshnessRunData,
    WalletSyncProgress
  >("ui.freshness.run", {
    onProgress: (progress) => {
      if (progress.phase !== "error" && progress.phase !== "done") {
        lastProgressRef.current = progress;
        if (progress.job_id) {
          jobProgressRef.current.set(progress.job_id, progress);
        }
      }
      if (!startedAtRef.current) return;
      const previousValue = progressValueRef.current;
      const nextProgress = syncProgressNotification(
        progress,
        previousValue,
      );
      progressValueRef.current = nextProgress.value;
      const now = new Date().toISOString();
      setActiveMaintenanceProgress(
        activeSyncMaintenanceProgress(progress, previousValue, {
          id: BOOK_REFRESH_PROGRESS_ID,
          startedAt: startedAtRef.current ?? now,
          updatedAt: now,
        }),
      );
      useNotificationStore.getState().updateJob(BOOK_REFRESH_JOB_ID, {
        value: nextProgress.progress.value,
        indeterminate: nextProgress.progress.indeterminate,
        label: nextProgress.progress.label,
      });
    },
  });

  const startRefresh = React.useCallback(
    (scope: NotificationScope, options?: WalletSyncOptions) => {
      const forceFull = Boolean(options?.forceFull);
      // The project's first-sync marker; the sync card is per project.
      const runBookKey = bookIdentityKey(useUiStore.getState().identity);
      progressValueRef.current = STARTING_SYNC_PROGRESS_VALUE;
      lastProgressRef.current = null;
      jobProgressRef.current.clear();
      const startedAt = new Date().toISOString();
      startedAtRef.current = startedAt;
      setActiveMaintenanceProgress({
        id: BOOK_REFRESH_PROGRESS_ID,
        title: forceFull
          ? t("bookRefresh.rescanInProgressTitle")
          : t("bookRefresh.refreshInProgressTitle"),
        body: forceFull
          ? t("bookRefresh.rescanStartedBody")
          : t("bookRefresh.refreshStartedBody"),
        tone: "warning",
        progress: startingSyncProgress(),
        details: [
          t("bookRefresh.configuredSourcesQueued"),
          t("bookRefresh.autoPairIncluded"),
          t("bookRefresh.journalsIncluded"),
        ],
        state: "running",
        startedAt,
        updatedAt: startedAt,
      });
      const starting = startingSyncProgress();
      useNotificationStore.getState().startJob({
        id: BOOK_REFRESH_JOB_ID,
        code: "book-refresh",
        scope,
        params: { forceFull },
        progress: {
          value: starting.value,
          indeterminate: starting.indeterminate,
          label: starting.label,
        },
      });
      refreshBook
        .mutateAsync({
          all: true,
          journals: true,
          auto_pair: true,
          run: true,
          force_full: forceFull,
          expected_scope: scope.expected,
        })
        .then(
          (envelope) => {
            const data = envelope.data;
            const needsAttention = freshnessRunNeedsAttention(data);
            const hasPendingJobs = freshnessRunHasPendingJobs(data);
            const quarantineCount = freshnessRunQuarantineCount(data);
            const blocksFirstSync =
              needsAttention || hasPendingJobs || quarantineCount > 0;
            recordBookRefreshOutcome(data, scope, forceFull);
            const failedMaintenance = needsAttention
              ? failedSyncMaintenanceProgress(
                  failedSyncProgressForRun(
                    data,
                    jobProgressRef.current,
                    lastProgressRef.current,
                  ),
                  progressValueRef.current,
                  summarizeFreshnessRun(data),
                  {
                    title: t("bookRefresh.needsAttentionTitle"),
                    startedAt: startedAtRef.current ?? undefined,
                    updatedAt: new Date().toISOString(),
                  },
                )
              : null;
            if (!blocksFirstSync) {
              options?.onTrustedSuccess?.();
              // The book has completed a clean full run, so subsequent
              // refreshes are ordinary background syncs rather than a
              // first-time setup. Reviewable swap/transfer candidates still
              // show as a Review condition, but they should not keep setup
              // mode alive forever once the core sync and journals are clean.
              if (runBookKey) markFirstSyncDone(runBookKey);
            }
            if (failedMaintenance) {
              setActiveMaintenanceProgress(failedMaintenance);
            } else {
              clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
            }
          },
          (error: unknown) => {
            if (isStaleBookError(error)) {
              // Refused before it ran: nothing changed in either book.
              clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
              reportStaleBook();
              return;
            }
            recordBookRefreshFailure(error, scope, forceFull);
            const now = new Date().toISOString();
            setActiveMaintenanceProgress(
              failedSyncMaintenanceProgress(
                lastProgressRef.current ?? { phase: "error" },
                progressValueRef.current,
                error instanceof Error ? error.message : t("bookRefresh.failedBody"),
                {
                  title: t("bookRefresh.failedTitle"),
                  startedAt: startedAtRef.current ?? now,
                  updatedAt: now,
                },
              ),
            );
          },
        )
        .finally(() => {
          useNotificationStore.getState().endJob(BOOK_REFRESH_JOB_ID);
          startedAtRef.current = null;
        });
    },
    [
      clearActiveMaintenanceProgress,
      markFirstSyncDone,
      refreshBook,
      setActiveMaintenanceProgress,
      t,
    ],
  );

  const syncAll = React.useCallback(
    (options?: WalletSyncOptions) => {
      // Honor in-flight refreshes from any surface. This hook is mounted in the
      // transactions dashboard, overview dashboard, and AppShell, so one book
      // refresh should block duplicates while connection-detail syncs are also
      // running. The running job is already visible, so there is no notice.
      const inFlight = () =>
        refreshBook.isPending ||
        queryClient.isMutating({
          mutationKey: freshnessRunMutationKey,
        }) > 0 ||
        queryClient.isMutating({
          mutationKey: walletSyncMutationKey,
        }) > 0;
      if (inFlight()) return;
      // The book this run belongs to, fixed at the start: a completion that
      // lands after a book switch still reports against this book, and the
      // daemon refuses the run outright if that book is no longer open.
      bindRunBook(
        queryClient,
        options?.expectedScope,
        (scope) => {
          if (!inFlight()) startRefresh(scope, options);
        },
        reportBookUnknown,
      );
    },
    [
      freshnessRunMutationKey,
      queryClient,
      refreshBook.isPending,
      startRefresh,
      walletSyncMutationKey,
    ],
  );


  return {
    syncAll,
    isSyncing: freshnessRunsInFlight + walletSyncsInFlight > 0,
  };
}

/**
 * What a finished book refresh leaves behind: per-source conditions from its
 * failed attempts and snapshot, a backend-required condition when wallets
 * have nowhere to sync from, one Recent result, and a toast. A returned run
 * resolves an earlier whole-run failure for the same book. Quarantine and
 * transfer review are not repeated here: the live Review condition counts
 * them.
 */
export function recordBookRefreshOutcome(
  data: FreshnessRunData | undefined,
  scope: NotificationScope,
  forceFull: boolean,
): void {
  const store = useNotificationStore.getState();
  store.resolveCondition(conditionKey("book-refresh.failed", scope));
  // A refresh that ran to the end shows the book's freshness work runs again.
  store.resolveCondition(
    conditionKey("freshness.worker-error", { book: scope.book }),
  );
  applyFreshnessRun(data, scope.book);
  const backendScope = { book: scope.book };
  if (freshnessRunNeedsBackend(data)) {
    store.raiseCondition({
      code: "book-refresh.backend-required",
      scope: backendScope,
      severity: "warning",
      priority: "attention",
      action: { type: "navigate", to: "/settings/bitcoin" },
    });
  } else {
    store.resolveCondition(
      conditionKey("book-refresh.backend-required", backendScope),
    );
  }
  if (freshnessRunNeedsAttention(data) || freshnessRunHasPendingJobs(data)) {
    // What did arrive is worth knowing even when something else failed.
    recordNewActivity(newActivityFromRun(data), scope);
  }
  const journalsOnly =
    journalStepNeedsAttention(data?.journals) &&
    !freshnessRunNeedsAttention({ ...data, journals: null });
  const t = i18n.getFixedT(null, "chrome");
  if (freshnessRunNeedsAttention(data)) {
    store.addResult({
      code: "book-refresh.problems",
      scope,
      severity: "warning",
      params: { forceFull },
      // Source problems carry their own conditions; a journal step that
      // did not finish is explained on Journals.
      action: journalsOnly ? { type: "navigate", to: "/journals" } : undefined,
      details: journalsOnly
        ? [{ message: safeMessage(describeJournalStep(data?.journals)) }]
        : undefined,
    });
    showFeedback({
      tone: "warning",
      title: t("notifications.result.bookRefreshProblems.title"),
      key: "book-refresh.outcome",
    });
    return;
  }
  if (freshnessRunHasPendingJobs(data)) {
    store.addResult({
      code: "book-refresh.pending",
      scope,
      severity: "info",
      params: { forceFull },
      action: {
        type: "navigate",
        to: freshnessRunOnlyJournalsPending(data) ? "/journals" : "/connections",
      },
    });
    showFeedback({
      tone: "info",
      title: t("notifications.result.bookRefreshPending.title"),
      key: "book-refresh.outcome",
    });
    return;
  }
  // New transactions say more than "finished": they replace that row (the
  // toast still confirms the refresh).
  const activity = newActivityFromRun(data);
  if (activity) {
    recordNewActivity(activity, scope);
  } else {
    store.addResult({
      code: "book-refresh.finished",
      scope,
      severity: "info",
      params: { forceFull },
    });
  }
  showFeedback({
    tone: "success",
    title: forceFull
      ? t("notifications.result.bookRefreshed.rescanTitle")
      : t("notifications.result.bookRefreshed.title"),
    key: "book-refresh.outcome",
  });
}

/** A run that did not return: a condition whose Retry repeats it as it was. */
export function recordBookRefreshFailure(
  error: unknown,
  scope: NotificationScope,
  forceFull: boolean,
): void {
  useNotificationStore.getState().raiseCondition({
    code: "book-refresh.failed",
    // With the daemon ids: its Retry sends them as `expected_scope`.
    scope: { book: scope.book, expected: scope.expected },
    severity: "error",
    priority: "attention",
    params: { forceFull },
    action: {
      type: "operation",
      operation: { kind: "refresh-book", forceFull },
      review: "/connections",
    },
    details: [
      { message: safeMessage(error instanceof Error ? error.message : undefined) },
    ],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "error",
    title: forceFull
      ? t("notifications.condition.bookRefreshFailed.rescanTitle")
      : t("notifications.condition.bookRefreshFailed.title"),
    key: "book-refresh.outcome",
  });
}
