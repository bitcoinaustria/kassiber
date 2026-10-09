import { ArrowRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { formatSats } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import {
  causeCopy,
  isWaiting,
  QUARANTINE_SCOPES,
  quarantineRootLabel,
  quarantineRowTarget,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import type { QuarantineItem, QuarantineScope } from "./types";

const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

function dateOnly(value: string | null | undefined) {
  return value ? value.slice(0, 10) : "";
}

function shortId(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function signedAmount(item: QuarantineItem) {
  const sign = item.direction === "outbound" ? "−" : "+";
  const asset = item.asset.toUpperCase();
  if (asset === "BTC" || asset === "LBTC") {
    return `${sign}${formatSats(Math.round(Math.abs(item.amount_msat) / 1000))}`;
  }
  return `${sign}${Math.abs(item.amount)} ${item.asset}`;
}

/**
 * The quarantined transactions, one scope at a time: what needs the user,
 * what only waits on a cause, or everything. Counts cover the whole book;
 * pages come from the daemon in its order, causes before what follows them.
 */
export function QuarantineQueue({
  items,
  scope,
  counts,
  offset,
  pageSize,
  total,
  loading,
  error = null,
  onRetry,
  hideSensitive,
  onScopeChange,
  onOffsetChange,
  onOpenTransaction,
}: {
  items: QuarantineItem[];
  scope: QuarantineScope;
  counts: Record<QuarantineScope, number>;
  offset: number;
  pageSize: number;
  /** Rows in this scope across all pages. */
  total: number;
  loading: boolean;
  /** Why this scope's page could not be read; never shown as an empty list. */
  error?: string | null;
  onRetry?: () => void;
  hideSensitive: boolean;
  onScopeChange: (scope: QuarantineScope) => void;
  onOffsetChange: (offset: number) => void;
  onOpenTransaction: (
    transactionId: string,
    tab: QuarantineSheetTab,
    context: QuarantineDetailContext | null,
  ) => void;
}) {
  const { t } = useTranslation(["journals", "common"]);
  const pageEnd = Math.min(offset + items.length, total);
  const empty =
    scope === "attention"
      ? t("quarantine.queue.attentionEmpty")
      : scope === "waiting"
        ? t("quarantine.queue.waitingEmpty")
        : t("quarantine.empty");
  return (
    <section className="kb-surface" aria-label={t("quarantine.tableTitle")}>
      <div className="flex flex-col gap-3 border-b p-3 sm:flex-row sm:items-center sm:justify-between sm:px-4">
        <h2 className="text-sm font-medium sm:text-base">{t("quarantine.tableTitle")}</h2>
        <div
          role="tablist"
          aria-label={t("quarantine.queue.scopeAria")}
          className="flex flex-wrap gap-2"
        >
          {QUARANTINE_SCOPES.map((option) => (
            <Button
              key={option}
              type="button"
              role="tab"
              aria-selected={scope === option}
              size="sm"
              variant={scope === option ? "default" : "outline"}
              className="h-8 gap-1.5"
              onClick={() => onScopeChange(option)}
            >
              {t(`quarantine.queue.scope.${option}`)}
              <span className="tabular-nums opacity-70">{counts[option]}</span>
            </Button>
          ))}
        </div>
      </div>

      {error !== null ? (
        <div className="flex flex-col items-center gap-2 px-4 py-8 text-center text-sm" role="alert">
          <p className="font-medium">{t("quarantine.queue.loadError")}</p>
          {error ? <p className="text-muted-foreground">{error}</p> : null}
          {onRetry ? (
            <Button type="button" size="sm" variant="outline" onClick={onRetry}>
              {t("common:actions.retry")}
            </Button>
          ) : null}
        </div>
      ) : loading ? (
        <div className="flex items-center justify-center gap-2 px-4 py-8 text-sm text-muted-foreground" role="status">
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        </div>
      ) : items.length ? (
        <ul className="divide-y" data-testid="quarantine-queue">
          {items.map((item) => (
            <QuarantineQueueRow
              key={item.transaction_id}
              item={item}
              hideSensitive={hideSensitive}
              onOpenTransaction={onOpenTransaction}
            />
          ))}
        </ul>
      ) : (
        <p className="px-4 py-8 text-center text-sm text-muted-foreground">{empty}</p>
      )}

      {error === null && total > pageSize ? (
        <nav
          className="flex items-center justify-end gap-2 border-t px-3 py-2 text-xs text-muted-foreground sm:px-4"
          aria-label={t("quarantine.tableTitle")}
        >
          <span className="tabular-nums">
            {t("quarantine.paging.range", {
              from: items.length ? offset + 1 : 0,
              to: pageEnd,
              total,
            })}
          </span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={offset === 0}
            onClick={() => onOffsetChange(Math.max(0, offset - pageSize))}
          >
            {t("quarantine.paging.previous")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={pageEnd >= total}
            onClick={() => onOffsetChange(offset + pageSize)}
          >
            {t("quarantine.paging.next")}
          </Button>
        </nav>
      ) : null}
    </section>
  );
}

function QuarantineQueueRow({
  item,
  hideSensitive,
  onOpenTransaction,
}: {
  item: QuarantineItem;
  hideSensitive: boolean;
  onOpenTransaction: (
    transactionId: string,
    tab: QuarantineSheetTab,
    context: QuarantineDetailContext | null,
  ) => void;
}) {
  const { t } = useTranslation("journals");
  const waiting = isWaiting(item);
  const target = quarantineRowTarget(item);
  const meta = (
    <span className={cn("min-w-0 truncate", sensitiveClass(hideSensitive))}>
      {[dateOnly(item.occurred_at), item.wallet, item.external_id ? shortId(item.external_id) : ""]
        .filter(Boolean)
        .join(" · ")}
    </span>
  );
  const amount = (
    <span className={cn("shrink-0 text-sm tabular-nums", sensitiveClass(hideSensitive))}>
      {signedAmount(item)}
    </span>
  );
  if (waiting && item.root) {
    // One line: everything here waits on its cause and needs nothing itself.
    const root = item.root;
    return (
      <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-xs text-muted-foreground sm:px-4">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center justify-between gap-3 text-left hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onOpenTransaction(item.transaction_id, target.tab, target.context)}
        >
          {meta}
          {amount}
        </button>
        <button
          type="button"
          className="inline-flex shrink-0 items-center gap-1 rounded-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onOpenTransaction(root.transaction_id, "details", null)}
        >
          <span className={sensitiveClass(hideSensitive)}>
            {t("quarantine.queue.waitsOn", { root: quarantineRootLabel(root) })}
          </span>
          <ArrowRight className="size-3" aria-hidden="true" />
        </button>
      </li>
    );
  }
  const copy = causeCopy(
    {
      reason: item.reason,
      category: item.category,
      evidence: item.evidence,
      detail: item.detail,
      wallet: item.wallet,
      asset: item.asset,
      rootLabel: item.root ? quarantineRootLabel(item.root) : null,
    },
    t,
  );
  return (
    <li>
      <button
        type="button"
        className={cn(
          "flex w-full items-start gap-3 px-3 py-2.5 text-left hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:px-4",
          item.blocks_reports && "bg-red-500/[0.035] dark:bg-red-950/10",
        )}
        onClick={() => onOpenTransaction(item.transaction_id, target.tab, target.context)}
      >
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium">{copy.title}</span>
            {item.blocks_reports ? (
              <span className="rounded-full bg-red-100 px-2 py-0.5 text-2xs font-medium text-red-800 dark:bg-red-950/50 dark:text-red-200">
                {t("quarantine.panel.blocksReports")}
              </span>
            ) : null}
          </span>
          <span className="mt-0.5 flex min-w-0 text-xs text-muted-foreground">{meta}</span>
        </span>
        {amount}
        <ArrowRight className="mt-1 size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
      </button>
    </li>
  );
}
