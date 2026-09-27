import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, Link2, Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { Button } from "@/components/ui/button";
import { useDaemonStreamMutation } from "@/daemon/client";
import { formatSats } from "@/lib/localeFormat";
import {
  describeWalletSyncResult,
  journalStepNeedsAttention,
  type JournalStepSummary,
  type SyncResult,
} from "@/lib/syncResults";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import {
  actionLabel,
  categoryLabel,
  causeCopy,
  sheetTabForCause,
  type QuarantineSheetTab,
} from "./explain";
import type {
  QuarantineAction,
  QuarantineAssumption,
  QuarantineGroup,
  QuarantineSnapshot,
} from "./types";

interface QuarantineCausePanelProps {
  snapshot: QuarantineSnapshot;
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
  onOpenTransaction: (transactionId: string, tab: QuarantineSheetTab) => void;
}

type ConnectionDialogState =
  | { mode: "connect" }
  | { mode: "import"; walletId: string | null }
  | null;

function formatMsat(value: number) {
  return formatSats(Math.round(Math.abs(value) / 1000));
}

function dateOnly(value: string | null | undefined) {
  return value ? value.slice(0, 10) : "";
}

/**
 * The explanation layer above the quarantine table: whether the list is
 * current, why rows are held, which causes block reports, and one action per
 * cause. Every action opens an existing, separately confirmed flow.
 */
export function QuarantineCausePanel({
  snapshot,
  isProcessingJournals,
  onProcessJournals,
  onOpenTransaction,
}: QuarantineCausePanelProps) {
  const { t } = useTranslation("journals");
  const navigate = useNavigate();
  const addNotification = useUiStore((s) => s.addNotification);
  const [dialog, setDialog] = React.useState<ConnectionDialogState>(null);
  const [lockedGapNotice, setLockedGapNotice] = React.useState<string | null>(null);
  const syncWallet = useDaemonStreamMutation<
    { results?: SyncResult[]; journals?: JournalStepSummary | null },
    unknown
  >("ui.wallets.sync");
  const { summary } = snapshot;
  const groups = summary.groups ?? [];
  const freshness = summary.freshness ?? null;
  const downstreamCount = (summary.by_category ?? []).find(
    (entry) => entry.category === "downstream",
  )?.count ?? 0;
  const assumptions = summary.assumptions ?? null;

  const runSync = (action: QuarantineAction) => {
    if (!action.wallet_id || syncWallet.isPending) return;
    const wallet = action.wallet_label || action.wallet_id;
    addNotification({
      title: t("quarantine.panel.syncStartedTitle", { wallet }),
      body: t("quarantine.panel.syncStartedBody"),
      tone: "warning",
      dedupeKey: `quarantine-sync-${action.wallet_id}`,
    });
    syncWallet.mutate(
      { wallet: action.wallet_id },
      {
        onSuccess: (envelope) => {
          const result = envelope.data?.results?.[0];
          const journals = envelope.data?.journals;
          const failed = result?.status === "error";
          addNotification({
            title: failed
              ? t("quarantine.panel.syncFailedTitle", { wallet })
              : t("quarantine.panel.syncFinishedTitle", { wallet }),
            body: describeWalletSyncResult(result, wallet, journals),
            tone: failed ? "error" : journalStepNeedsAttention(journals) ? "warning" : "success",
            dedupeKey: `quarantine-sync-${action.wallet_id}`,
          });
        },
        onError: (error) => {
          addNotification({
            title: t("quarantine.panel.syncFailedTitle", { wallet }),
            body: error instanceof Error ? error.message : "",
            tone: "error",
            dedupeKey: `quarantine-sync-${action.wallet_id}`,
          });
        },
      },
    );
  };

  const runAction = (group: QuarantineGroup, action: QuarantineAction) => {
    const rootId = group.root_transaction_id;
    switch (action.kind) {
      case "sync_wallet":
        runSync(action);
        return;
      case "connect_wallet":
        setDialog({ mode: "connect" });
        return;
      case "import_history":
        setDialog({ mode: "import", walletId: action.wallet_id ?? null });
        return;
      case "review_custody_gap":
        // Decided at click time; the /swaps route guard enforces the same gate.
        if (useUiStore.getState().developerToolsEnabled && action.gap_id) {
          void navigate({ to: "/swaps", search: { tab: "gaps", gap: action.gap_id } });
        } else {
          setLockedGapNotice(group.key);
        }
        return;
      case "process_journals":
        onProcessJournals();
        return;
      case "wait_for_confirmation":
        return;
      case "resolve_root":
        if (action.transaction_id) onOpenTransaction(action.transaction_id, "details");
        return;
      default: {
        const target = action.transaction_id ?? rootId;
        if (target) {
          onOpenTransaction(target, sheetTabForCause(group.reason, group.category, group.evidence));
        }
      }
    }
  };

  const hasFreshnessWarning = Boolean(freshness?.needs_processing || freshness?.last_error);
  if (!summary.count && !hasFreshnessWarning && !hasAssumptions(assumptions)) {
    return null;
  }

  return (
    <section className="mb-4 space-y-4" aria-label={t("quarantine.panel.title")}>
      {hasFreshnessWarning ? (
        <div
          role="status"
          className="flex flex-col gap-3 rounded-lg border border-amber-300 bg-amber-50 p-4 text-amber-900 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100 sm:flex-row sm:items-center sm:justify-between"
        >
          <div className="flex min-w-0 gap-3">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div className="min-w-0 space-y-1 text-sm">
              <p className="font-medium">
                {freshness?.last_error
                  ? t("quarantine.freshness.failedTitle")
                  : t("quarantine.freshness.staleTitle")}
              </p>
              <p>
                {freshness?.last_error
                  ? t("quarantine.freshness.failedBody", {
                      message: freshness.last_error.message,
                    })
                  : freshness?.last_processed_at
                    ? t("quarantine.freshness.staleBody", {
                        when: dateOnly(freshness.last_processed_at),
                      })
                    : t("quarantine.freshness.neverBody")}
              </p>
            </div>
          </div>
          <Button
            type="button"
            size="sm"
            onClick={onProcessJournals}
            disabled={isProcessingJournals}
            className="shrink-0"
          >
            {isProcessingJournals ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            {t("quarantine.cta.processJournals")}
          </Button>
        </div>
      ) : null}

      {summary.count ? (
        <div className="rounded-lg border bg-card p-4">
          <h2 className="text-base font-semibold">{t("quarantine.panel.title")}</h2>
          <p className="mt-1 max-w-3xl text-sm text-muted-foreground">
            {t("quarantine.panel.intro")}
          </p>
          <ul className="mt-3 flex flex-wrap gap-2 text-xs font-medium">
            <li className="rounded-full bg-muted px-2.5 py-1">
              {t("quarantine.panel.counts", { count: summary.count })}
            </li>
            {summary.blocking_count ? (
              <li className="rounded-full bg-red-100 px-2.5 py-1 text-red-800 dark:bg-red-950/50 dark:text-red-200">
                {t("quarantine.panel.blocking", { count: summary.blocking_count })}
              </li>
            ) : null}
            {downstreamCount ? (
              <li className="rounded-full bg-muted px-2.5 py-1 text-muted-foreground">
                {t("quarantine.panel.downstream", { count: downstreamCount })}
              </li>
            ) : null}
          </ul>
          {summary.reports_blocked ? (
            <p className="mt-3 text-sm text-red-700 dark:text-red-300">
              {t("quarantine.panel.reportsBlocked")}
            </p>
          ) : null}
        </div>
      ) : null}

      {groups.length ? (
        <div className="space-y-2">
          <div>
            <h3 className="text-sm font-semibold">{t("quarantine.panel.causesTitle")}</h3>
            <p className="text-xs text-muted-foreground">{t("quarantine.panel.causesHint")}</p>
          </div>
          <ol className="space-y-2">
            {groups.map((group) => (
              <QuarantineCauseCard
                key={group.key}
                group={group}
                lockedGapNotice={lockedGapNotice === group.key}
                syncPending={syncWallet.isPending}
                onAction={(action) => runAction(group, action)}
                onOpenRoot={() => {
                  if (group.root_transaction_id) {
                    onOpenTransaction(
                      group.root_transaction_id,
                      sheetTabForCause(group.reason, group.category, group.evidence),
                    );
                  }
                }}
              />
            ))}
          </ol>
          {(summary.group_count ?? groups.length) > groups.length ? (
            <p className="text-xs text-muted-foreground">
              {t("quarantine.panel.moreCauses", {
                count: (summary.group_count ?? groups.length) - groups.length,
              })}
            </p>
          ) : null}
        </div>
      ) : null}

      {assumptions && hasAssumptions(assumptions) ? (
        <QuarantineAssumptions
          outbound={assumptions.presumed_external_outbound}
          inbound={assumptions.unclassified_inbound}
          onOpenTransaction={onOpenTransaction}
          onConnectWallet={() => setDialog({ mode: "connect" })}
        />
      ) : null}

      {dialog ? (
        <AddConnectionDialog
          open
          initialSourceId={dialog.mode === "connect" ? "descriptor" : null}
          initialTargetWalletId={dialog.mode === "import" ? dialog.walletId : undefined}
          onOpenChange={(open) => {
            if (!open) setDialog(null);
          }}
        />
      ) : null}
    </section>
  );
}

function hasAssumptions(assumptions: QuarantineSnapshot["summary"]["assumptions"]) {
  return Boolean(
    assumptions &&
      (assumptions.presumed_external_outbound.count ||
        assumptions.unclassified_inbound.count),
  );
}

function QuarantineCauseCard({
  group,
  lockedGapNotice,
  syncPending,
  onAction,
  onOpenRoot,
}: {
  group: QuarantineGroup;
  lockedGapNotice: boolean;
  syncPending: boolean;
  onAction: (action: QuarantineAction) => void;
  onOpenRoot: () => void;
}) {
  const { t } = useTranslation("journals");
  const copy = causeCopy(
    {
      reason: group.reason,
      category: group.category,
      evidence: group.evidence,
      wallet: group.root_wallet,
      asset: group.root_asset,
    },
    t,
  );
  const rootCount = group.count - group.downstream_count;
  const deprecated = (group.evidence.missing_source_wallets ?? []).filter(
    (wallet) => wallet.deprecated,
  );
  return (
    <li
      className={cn(
        "rounded-lg border bg-card p-4",
        group.blocks_reports && "border-red-300 dark:border-red-900/60",
      )}
    >
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="rounded-full bg-muted px-2 py-0.5 font-medium">
          {categoryLabel(group.category, t)}
        </span>
        {group.blocks_reports ? (
          <span className="rounded-full bg-red-100 px-2 py-0.5 font-medium text-red-800 dark:bg-red-950/50 dark:text-red-200">
            {t("quarantine.panel.blocksReports")}
          </span>
        ) : null}
        <span className="text-muted-foreground">
          {t("quarantine.panel.affected", { count: Math.max(rootCount, 1) })}
        </span>
        {group.earliest_occurred_at ? (
          <span className="text-muted-foreground">
            {t("quarantine.panel.since", { date: dateOnly(group.earliest_occurred_at) })}
          </span>
        ) : null}
        {group.wallets.length ? (
          <span className="text-muted-foreground">
            {t("quarantine.panel.wallets", { wallets: group.wallets.join(", ") })}
          </span>
        ) : null}
      </div>
      <p className="mt-2 text-sm font-semibold">{copy.title}</p>
      <p className="mt-1 text-sm text-muted-foreground">{copy.why}</p>
      {group.root_amount_msat &&
      ["BTC", "LBTC"].includes(String(group.root_asset ?? "").toUpperCase()) ? (
        <p className="mt-1 text-xs text-muted-foreground tabular-nums">
          {dateOnly(group.root_occurred_at)} · {group.root_wallet} ·{" "}
          {formatMsat(group.root_amount_msat)}
        </p>
      ) : null}
      <p className="mt-2 text-sm">
        <span className="font-medium">{t("quarantine.panel.whatToProvide")}: </span>
        {copy.provide}
      </p>
      {deprecated.map((wallet) => (
        <p key={wallet.id} className="mt-1 text-xs text-amber-700 dark:text-amber-300">
          {t("quarantine.panel.deprecatedWallet", { wallet: wallet.label })}
        </p>
      ))}
      {group.downstream_count ? (
        <p className="mt-2 text-xs text-muted-foreground">
          {t("quarantine.panel.dependents", { count: group.downstream_count })}
        </p>
      ) : null}
      {lockedGapNotice ? (
        <p className="mt-2 text-xs text-muted-foreground">{t("quarantine.panel.gapReviewLocked")}</p>
      ) : null}
      <div className="mt-3 flex flex-wrap gap-2">
        {group.actions.map((action, index) => (
          <Button
            key={`${action.kind}:${action.wallet_id ?? action.transaction_id ?? index}`}
            type="button"
            size="sm"
            variant={index === 0 ? "default" : "outline"}
            disabled={
              action.kind === "wait_for_confirmation" ||
              (action.kind === "sync_wallet" && syncPending)
            }
            onClick={() => onAction(action)}
          >
            {action.kind === "sync_wallet" && syncPending ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : null}
            {actionLabel(action, t)}
          </Button>
        ))}
        {group.root_transaction_id ? (
          <Button type="button" size="sm" variant="ghost" onClick={onOpenRoot}>
            <Link2 className="size-4" aria-hidden="true" />
            {t("quarantine.cta.openTransaction")}
          </Button>
        ) : null}
      </div>
    </li>
  );
}

function QuarantineAssumptions({
  outbound,
  inbound,
  onOpenTransaction,
  onConnectWallet,
}: {
  outbound: QuarantineAssumption;
  inbound: QuarantineAssumption;
  onOpenTransaction: (transactionId: string, tab: QuarantineSheetTab) => void;
  onConnectWallet: () => void;
}) {
  const { t } = useTranslation("journals");
  const blocks = [
    {
      key: "outbound",
      data: outbound,
      title: t("quarantine.assumptions.outboundTitle", { count: outbound.count }),
      why: t("quarantine.assumptions.outboundWhy"),
      fix: t("quarantine.assumptions.outboundFix"),
    },
    {
      key: "inbound",
      data: inbound,
      title: t("quarantine.assumptions.inboundTitle", { count: inbound.count }),
      why: t("quarantine.assumptions.inboundWhy"),
      fix: t("quarantine.assumptions.inboundFix"),
    },
  ].filter((block) => block.data.count > 0);
  return (
    <div className="rounded-lg border bg-card p-4">
      <h3 className="text-sm font-semibold">{t("quarantine.assumptions.title")}</h3>
      <p className="mt-1 text-xs text-muted-foreground">{t("quarantine.assumptions.intro")}</p>
      <div className="mt-3 grid gap-3 lg:grid-cols-2">
        {blocks.map((block) => (
          <details key={block.key} className="rounded-md border p-3">
            <summary className="cursor-pointer text-sm font-medium">
              {block.title}
              <span className="ml-2 text-xs font-normal text-muted-foreground tabular-nums">
                {t("quarantine.assumptions.total", { amount: formatMsat(block.data.amount_msat) })}
              </span>
            </summary>
            <p className="mt-2 text-xs text-muted-foreground">{block.why}</p>
            <p className="mt-1 text-xs">{block.fix}</p>
            <ul className="mt-2 divide-y text-xs">
              {block.data.items.map((item) => (
                <li key={item.transaction_id}>
                  <button
                    type="button"
                    className="flex w-full items-center justify-between gap-3 py-1.5 text-left hover:underline"
                    onClick={() => onOpenTransaction(item.transaction_id, "tax")}
                  >
                    <span className="truncate">
                      {dateOnly(item.occurred_at)} · {item.wallet}
                    </span>
                    <span className="shrink-0 tabular-nums">{formatMsat(item.amount_msat)}</span>
                  </button>
                </li>
              ))}
            </ul>
            {block.data.count > block.data.items.length ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {t("quarantine.assumptions.showMore", {
                  count: block.data.count - block.data.items.length,
                })}
              </p>
            ) : null}
            <Button type="button" size="sm" variant="outline" className="mt-2" onClick={onConnectWallet}>
              {t("quarantine.cta.connectWallet")}
            </Button>
          </details>
        ))}
      </div>
    </div>
  );
}
