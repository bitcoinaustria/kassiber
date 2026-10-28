import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  ArrowLeftRight,
  ArrowUpRight,
  CheckCircle2,
  ChevronRight,
  Loader2,
  RefreshCw,
} from "lucide-react";

import {
  causeCopy,
  categoryLabel,
  sheetTabForCause,
} from "@/components/kb/quarantine/explain";
import type { QuarantineGroup, QuarantineSnapshot } from "@/components/kb/quarantine/types";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";
import { cn } from "@/lib/utils";
import { useRightPanelStore } from "@/store/rightPanel";
import { useUiStore } from "@/store/ui";

import {
  SurfaceEmpty,
  SurfaceFrame,
  SurfaceSection,
  ToneDot,
  surfaceRowClassName,
} from "./SurfaceLayout";

/** The same cheap counts the side nav's hints read (`ui.review.badges`). */
type ReviewBadges = {
  quarantine: number;
  quarantine_attention?: number;
  journals_needs_processing: boolean;
  swaps: number | null;
};

/**
 * What needs the user in this book, beside whatever page is open: the causes
 * the journals hold transactions for, transfers still to match, and journals
 * waiting to be processed. A cause opens its transaction in a tab of its own,
 * at the tab that holds the evidence, so the page underneath stays put.
 */
export function ReviewSurface() {
  const { t } = useTranslation(["assistant", "journals"]);
  const navigate = useNavigate();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const openTransaction = useRightPanelStore((s) => s.openTransaction);
  const badgesQuery = useDaemon<ReviewBadges>("ui.review.badges");
  // The page's first attention read: the summary names every cause.
  const quarantineQuery = useDaemon<QuarantineSnapshot>("ui.journals.quarantine", {
    limit: 25,
    offset: 0,
    scope: "attention",
  });
  const { runJournalProcessing, isProcessingJournals } = useJournalProcessingAction({
    notifyStart: true,
    notifyAlreadyRunning: true,
  });

  const snapshot = React.useMemo(
    () =>
      quarantineQuery.data?.data ? normalizeQuarantineSnapshot(quarantineQuery.data.data) : null,
    [quarantineQuery.data?.data],
  );
  const badges = badgesQuery.data?.data;
  const groups = snapshot?.summary.groups ?? [];
  const summary = snapshot?.summary;
  const waiting = summary?.waiting_count ?? 0;
  const swaps = typeof badges?.swaps === "number" ? badges.swaps : 0;
  const needsProcessing = Boolean(
    badges?.journals_needs_processing || summary?.freshness?.needs_processing,
  );
  const reportsBlocked = Boolean(summary?.reports_blocked);
  const nothingOpen = groups.length === 0 && swaps === 0 && !needsProcessing;

  const openCause = (group: QuarantineGroup) => {
    const root = group.root_transaction_id ?? group.root_transaction_ids[0];
    if (!root) {
      void navigate({ to: "/quarantine" });
      return;
    }
    openTransaction(root, sheetTabForCause(group.reason, group.category, group.evidence));
  };

  let body: React.ReactNode;
  if (quarantineQuery.isLoading) {
    body = (
      <div className="flex h-full min-h-48 items-center justify-center" role="status">
        <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />
        <span className="sr-only">{t("assistant:panel.review.loading")}</span>
      </div>
    );
  } else if (quarantineQuery.isError || !snapshot) {
    body = (
      <SurfaceEmpty
        icon={AlertTriangle}
        tone="destructive"
        title={t("assistant:panel.review.error")}
        action={
          <Button type="button" size="sm" variant="outline" onClick={() => void quarantineQuery.refetch()}>
            {t("assistant:panel.retry")}
          </Button>
        }
      />
    );
  } else if (nothingOpen) {
    body = (
      <SurfaceEmpty
        icon={CheckCircle2}
        tone="success"
        title={t("assistant:panel.review.emptyTitle")}
        body={t("assistant:panel.review.emptyBody")}
      />
    );
  } else {
    body = (
      <>
        {reportsBlocked ? (
          <div className="mx-3 mt-3 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-2.5 py-2 text-xs text-destructive">
            <AlertTriangle className="mt-px size-3.5 shrink-0" aria-hidden="true" />
            <span>{t("assistant:panel.review.reportsBlocked")}</span>
          </div>
        ) : null}
        {groups.length ? (
          <SurfaceSection title={t("assistant:panel.review.causes")} count={groups.length}>
            <ul className="flex flex-col">
              {groups.map((group) => (
                <li key={group.key}>
                  <CauseRow group={group} hideSensitive={hideSensitive} onOpen={openCause} />
                </li>
              ))}
            </ul>
          </SurfaceSection>
        ) : null}
        {swaps > 0 || needsProcessing ? (
          <SurfaceSection title={t("assistant:panel.review.alsoOpen")}>
            <ul className="flex flex-col">
              {swaps > 0 ? (
                <li>
                  <button
                    type="button"
                    className={surfaceRowClassName}
                    onClick={() => void navigate({ to: "/swaps" })}
                  >
                    <ArrowLeftRight className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden="true" />
                    <span className="min-w-0 flex-1 truncate text-sm">
                      {t("assistant:panel.review.transfers", { count: swaps })}
                    </span>
                    <ArrowUpRight className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                  </button>
                </li>
              ) : null}
              {needsProcessing ? (
                <li className="flex items-center gap-2.5 px-2 py-1.5">
                  <RefreshCw
                    className={cn(
                      "size-3.5 shrink-0 text-amber-600 dark:text-amber-400",
                      isProcessingJournals && "animate-spin",
                    )}
                    aria-hidden="true"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm">
                    {t("assistant:panel.review.journals")}
                  </span>
                  <Button
                    type="button"
                    size="xs"
                    variant="outline"
                    disabled={isProcessingJournals}
                    onClick={runJournalProcessing}
                  >
                    {t("assistant:panel.review.process")}
                  </Button>
                </li>
              ) : null}
            </ul>
          </SurfaceSection>
        ) : null}
      </>
    );
  }

  return (
    <SurfaceFrame
      label={t("assistant:panel.surfaceReview")}
      toolbar={
        snapshot && !nothingOpen ? (
          <>
            <span className="min-w-0 flex-1 truncate text-muted-foreground">
              {t("assistant:panel.review.summary", { count: groups.length })}
              {waiting > 0 ? ` · ${t("assistant:panel.review.waiting", { count: waiting })}` : ""}
            </span>
            {quarantineQuery.isFetching ? (
              <Loader2 className="size-3 animate-spin text-muted-foreground" aria-hidden="true" />
            ) : null}
          </>
        ) : null
      }
      footer={
        <Button
          type="button"
          variant="ghost"
          size="xs"
          className="ml-auto gap-1 text-muted-foreground"
          onClick={() => void navigate({ to: "/quarantine" })}
        >
          {t("assistant:panel.review.openPage")}
          <ArrowUpRight className="size-3" aria-hidden="true" />
        </Button>
      }
    >
      {body}
    </SurfaceFrame>
  );
}

function CauseRow({
  group,
  hideSensitive,
  onOpen,
}: {
  group: QuarantineGroup;
  hideSensitive: boolean;
  onOpen: (group: QuarantineGroup) => void;
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
  const meta = [categoryLabel(group.category, t), group.wallets.join(", ")]
    .filter(Boolean)
    .join(" · ");
  return (
    <button type="button" className={cn(surfaceRowClassName, "items-start")} onClick={() => onOpen(group)}>
      <span className="flex h-5 items-center">
        <ToneDot tone={group.blocks_reports ? "blocker" : "review"} />
      </span>
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block truncate text-sm text-foreground",
            hideSensitive && copy.whyQuotesAmounts && "sensitive",
          )}
        >
          {copy.title}
        </span>
        <span className={cn("block truncate text-xs text-muted-foreground", hideSensitive && "sensitive")}>
          {meta}
        </span>
      </span>
      <span className="flex h-5 shrink-0 items-center gap-1 text-xs tabular-nums text-muted-foreground">
        {group.count}
        <ChevronRight className="size-3.5" aria-hidden="true" />
      </span>
    </button>
  );
}
