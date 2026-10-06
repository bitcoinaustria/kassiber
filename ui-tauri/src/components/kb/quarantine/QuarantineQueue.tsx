import { ArrowRight, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import {
  quarantineRootLabel,
  quarantineRowAmount,
  quarantineRowMeta,
  quarantineRowTarget,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import type { QuarantineItem } from "./types";

const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

/**
 * The transactions that only wait on a cause and clear with it. What needs
 * the user is listed on its cause's card; this is the rest, one line each,
 * paged in the daemon's order.
 */
export function QuarantineQueue({
  items,
  offset,
  pageSize,
  total,
  loading,
  hideSensitive,
  onOffsetChange,
  onOpenTransaction,
  onHide,
}: {
  items: QuarantineItem[];
  offset: number;
  pageSize: number;
  /** Waiting rows across all pages. */
  total: number;
  loading: boolean;
  hideSensitive: boolean;
  onOffsetChange: (offset: number) => void;
  /** Folds the list away again. */
  onHide: () => void;
  onOpenTransaction: (
    transactionId: string,
    tab: QuarantineSheetTab,
    context: QuarantineDetailContext | null,
  ) => void;
}) {
  const { t } = useTranslation("journals");
  const pageEnd = Math.min(offset + items.length, total);
  return (
    <section className="kb-surface" aria-label={t("quarantine.queue.title")}>
      <div className="flex items-center gap-2 border-b p-3 sm:px-4">
        <h2 className="text-sm font-medium sm:text-base">{t("quarantine.queue.title")}</h2>
        <span className="text-xs tabular-nums text-muted-foreground">{total}</span>
        <Button type="button" variant="ghost" size="sm" className="ml-auto h-7 text-xs" onClick={onHide}>
          {t("quarantine.queue.hide")}
        </Button>
      </div>

      {loading ? (
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
        <p className="px-4 py-8 text-center text-sm text-muted-foreground">
          {t("quarantine.queue.waitingEmpty")}
        </p>
      )}

      {total > pageSize ? (
        <nav
          className="flex items-center justify-end gap-2 border-t px-3 py-2 text-xs text-muted-foreground sm:px-4"
          aria-label={t("quarantine.queue.title")}
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
  const target = quarantineRowTarget(item);
  const root = item.root;
  // One line: the row needs nothing itself; its cause is one click away.
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2 text-xs text-muted-foreground sm:px-4">
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center justify-between gap-3 text-left hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        onClick={() => onOpenTransaction(item.transaction_id, target.tab, target.context)}
      >
        <span className={cn("min-w-0 truncate", sensitiveClass(hideSensitive))}>
          {quarantineRowMeta(item)}
        </span>
        <span className={cn("shrink-0 text-sm tabular-nums", sensitiveClass(hideSensitive))}>
          {quarantineRowAmount(item)}
        </span>
      </button>
      {root ? (
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
      ) : null}
    </li>
  );
}
