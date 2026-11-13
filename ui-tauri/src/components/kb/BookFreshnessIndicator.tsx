import { useIsMutating } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { useBookRefreshStore } from "@/daemon/bookRefresh";
import { bookFreshnessState, type BookFreshnessState } from "@/lib/bookFreshness";
import { daemonMutationKey } from "@/daemon/client";
import { formatBtc } from "@/lib/currency";
import { maskSensitive, useSensitive } from "@/lib/sensitive";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

const dotClassName: Record<BookFreshnessState["kind"], string> = {
  updating: "bg-sky-500 motion-safe:animate-pulse",
  failed: "bg-red-500",
  "needs-journals": "bg-amber-500",
  current: "bg-emerald-500",
};

/**
 * The title bar's book status: whether what the screens show is current,
 * being updated, or held back, and what the last refresh changed. Screens
 * keep their data on screen while the book updates and only dim the figures
 * that will change, so this is the one place that says so.
 */
export function BookFreshnessIndicator({
  daemonEnabled,
  needsJournals,
  refreshFailed,
}: {
  daemonEnabled: boolean;
  needsJournals: boolean;
  refreshFailed: boolean;
}) {
  const dataMode = useUiStore((state) => state.dataMode);
  const syncs =
    useIsMutating({ mutationKey: daemonMutationKey(dataMode, "ui.freshness.run") }) +
    useIsMutating({ mutationKey: daemonMutationKey(dataMode, "ui.wallets.sync") });
  const journals = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.journals.process"),
  });
  const cycling = useBookRefreshStore((state) => state.cycling);
  const lastChange = useBookRefreshStore((state) => state.lastChange);
  const hideSensitive = useSensitive("amount");
  if (!daemonEnabled) return null;
  return (
    <BookFreshnessView
      state={bookFreshnessState({
        syncing: syncs > 0,
        processingJournals: journals > 0,
        loadingChanges: cycling,
        refreshFailed,
        needsJournals,
        lastChange,
      })}
      hideSensitive={hideSensitive}
    />
  );
}

export function BookFreshnessView({
  state,
  hideSensitive,
}: {
  state: BookFreshnessState;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("chrome");
  const updating = {
    syncing: t("shell.freshness.syncing"),
    processing: t("shell.freshness.processing"),
    loading: t("shell.freshness.loading"),
  };
  const label =
    state.kind === "updating"
      ? updating[state.reason]
      : state.kind === "failed"
        ? t("shell.freshness.failed")
        : state.kind === "needs-journals"
          ? t("shell.freshness.needsJournals")
          : t("shell.freshness.current");
  const change = state.kind === "current" ? state.change : null;
  const transactions =
    change && change.transactionDelta !== 0
      ? change.transactionDelta > 0
        ? t("shell.freshness.newTransactions", { count: change.transactionDelta })
        : t("shell.freshness.removedTransactions", {
            count: -change.transactionDelta,
          })
      : null;
  const balance =
    change?.balanceDeltaSat != null
      ? formatBtc(change.balanceDeltaSat / 1e8, { sign: true })
      : null;
  return (
    // Kept in the tree on narrow windows (visually hidden) so the update
    // is still announced; it is the screens' one live region for refreshes.
    <div
      className="flex min-w-0 items-center gap-1.5 px-2 text-xs text-muted-foreground max-sm:sr-only"
      role="status"
      aria-live="polite"
      title={t("shell.freshness.label")}
      data-state={state.kind}
    >
      <span
        className={cn("size-1.5 shrink-0 rounded-full", dotClassName[state.kind])}
        aria-hidden="true"
      />
      <span className="truncate">{label}</span>
      {transactions ? (
        <span className="hidden truncate lg:inline">· {transactions}</span>
      ) : null}
      {balance ? (
        <span className="hidden truncate tabular-nums lg:inline">
          ·{" "}
          {t("shell.freshness.balance")}{" "}
          {/* Replaced, not only blurred: the live region reads it aloud. */}
          <span className={hideSensitive ? "sensitive" : undefined}>
            {maskSensitive(balance, hideSensitive)}
          </span>
        </span>
      ) : null}
    </div>
  );
}
