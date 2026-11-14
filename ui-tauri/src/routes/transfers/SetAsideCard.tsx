/**
 * Suggestions the owner set aside ("Not my transfer"), each with Restore, so
 * a dismissal made by mistake is not gone once its 20-second undo passed.
 * Folded by default: it is a way back, not work to do.
 *
 * After rotki's unmatched list, where an ignored match keeps a restore action
 * (`frontend/app/src/modules/history/events/UnmatchedActions.vue`).
 */
import * as React from "react";
import { ArrowRight, ChevronDown, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { blurClass } from "@/components/transactions/model";
import { Button } from "@/components/ui/button";
import { railForLeg } from "@/lib/bitcoinRail";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import type { TransferDismissal } from "./linkedMovesModel";
import { RowMenu } from "./RowMenu";
import { dateTime } from "./linkModel";
import { formatMsatAsBtc } from "./satsFormat";

export function SetAsideCard({
  rows,
  pending,
  defaultOpen = false,
  onRestore,
}: {
  rows: TransferDismissal[];
  pending: boolean;
  /** Unfolded from the start, as on its own tab. */
  defaultOpen?: boolean;
  onRestore: (dismissal: TransferDismissal) => void;
}) {
  const { t } = useTranslation("review");
  const [open, setOpen] = React.useState(defaultOpen);
  if (!rows.length) return null;
  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="transfers-set-aside-title" data-testid="transfers-set-aside">
      <button
        type="button"
        className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-sm hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none sm:px-6"
        aria-expanded={open}
        aria-controls="transfers-set-aside-list"
        onClick={() => setOpen((value) => !value)}
      >
        <span id="transfers-set-aside-title" className="font-medium">
          {t("transfersPage.setAside.title", { count: rows.length })}
        </span>
        <span className="min-w-0 truncate text-xs text-muted-foreground">{t("transfersPage.setAside.hint")}</span>
        <ChevronDown className={cn("ml-auto size-4 shrink-0 text-muted-foreground transition-transform", open && "rotate-180")} aria-hidden="true" />
      </button>
      {open ? (
        <ul id="transfers-set-aside-list" className="divide-y border-t">
          {rows.map((row) => (
            <SetAsideRow key={row.id} row={row} pending={pending} onRestore={() => onRestore(row)} />
          ))}
        </ul>
      ) : null}
    </section>
  );
}

function SetAsideRow({ row, pending, onRestore }: { row: TransferDismissal; pending: boolean; onRestore: () => void }) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const route = t("inbox.transfers.route", { from: row.out.wallet, to: row.in.wallet });
  return (
    <li
      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-3 py-2 sm:px-6 @6xl/list:grid-cols-[7.5rem_minmax(0,1fr)_9rem_auto]"
      data-testid="transfers-set-aside-row"
    >
      <span className="text-xs text-muted-foreground tabular-nums">{dateTime(row.out.occurred_at ?? row.in.occurred_at)}</span>
      <span className="col-span-2 row-start-2 grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2 @6xl/list:col-span-1 @6xl/list:col-start-2 @6xl/list:row-start-1">
        <span className="flex min-w-0 items-center gap-1.5">
          <RailIcon rail={railForLeg(row.out.asset, null)} size="compact" />
          <span className={cn("truncate text-sm", blurClass(hideSensitive))}>{row.out.wallet}</span>
        </span>
        <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("inbox.transfers.to")} />
        <span className="flex min-w-0 items-center gap-1.5">
          <RailIcon rail={railForLeg(row.in.asset, null)} size="compact" />
          <span className={cn("truncate text-sm", blurClass(hideSensitive))}>{row.in.wallet}</span>
        </span>
      </span>
      <span className={cn("col-start-2 row-start-1 text-right text-sm tabular-nums @6xl/list:col-start-3", blurClass(hideSensitive))}>
        {formatMsatAsBtc(row.out.amount_msat)}
      </span>
      <span className="col-span-2 row-start-3 flex items-center justify-end gap-1 @6xl/list:col-span-1 @6xl/list:col-start-4 @6xl/list:row-start-1">
      <Button
        type="button"
        size="sm"
        variant="ghost"
        className="h-8 gap-1.5"
        disabled={pending}
        onClick={onRestore}
        aria-label={t("transfersPage.setAside.restoreLabel", { route })}
      >
        <RotateCcw className="size-3.5" aria-hidden="true" />
        {t("transfersPage.setAside.restore")}
      </Button>
      <RowMenu route={route} outId={row.out_transaction_id} inId={row.in_transaction_id} sayWhat />
      </span>
    </li>
  );
}
