/**
 * Moves the owner unlinked whose legs nothing links now, each with the way
 * back: "Link again" restores the pair with the terms it had (kind, tax
 * treatment, swapped portion) after the same preview as any link, and an
 * undone automatic pairing is linked back by removing its undo. Listed
 * whether or not the matcher would suggest the pair, so a move Kassiber
 * never suggests (a peg-out with a large fee) can always be linked again.
 *
 * After rotki's "Ignored" tab, where an unlinked match stays to be restored
 * or matched by hand, but on the page itself rather than inside a dialog.
 */
import { ArrowRight, Link2, Unlink } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { SurfaceCardHeader } from "@/components/kb/SurfaceCardHeader";
import { blurClass } from "@/components/transactions/model";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import type { RecentlyUnlinked } from "./linkedMovesModel";
import { RowMenu } from "./RowMenu";
import { datePart, dateTime, kindKey } from "./linkModel";

import { formatMsatAsBtc } from "./satsFormat";

export function RecentlyUnlinkedCard({
  rows,
  pending,
  headingRef,
  onLinkAgain,
  onLinkDifferently,
}: {
  rows: RecentlyUnlinked[];
  pending: boolean;
  headingRef?: React.RefObject<HTMLHeadingElement | null>;
  onLinkAgain: (row: RecentlyUnlinked) => void;
  onLinkDifferently: (row: RecentlyUnlinked) => void;
}) {
  const { t } = useTranslation("review");
  if (!rows.length) return null;
  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="transfers-unlinked-title" data-testid="transfers-recently-unlinked">
      <SurfaceCardHeader
        title={t("transfersPage.unlinked.title")}
        titleId="transfers-unlinked-title"
        titleRef={headingRef}
        count={rows.length}
      >
        <span className="hidden text-xs text-muted-foreground sm:inline">{t("transfersPage.unlinked.hint")}</span>
      </SurfaceCardHeader>
      <ul className="divide-y border-t">
        {rows.map((row) => (
          <UnlinkedRow
            key={row.key}
            row={row}
            pending={pending}
            onLinkAgain={() => onLinkAgain(row)}
            onLinkDifferently={() => onLinkDifferently(row)}
          />
        ))}
      </ul>
    </section>
  );
}

function UnlinkedRow({
  row,
  pending,
  onLinkAgain,
  onLinkDifferently,
}: {
  row: RecentlyUnlinked;
  pending: boolean;
  onLinkAgain: () => void;
  onLinkDifferently: () => void;
}) {
  const { t } = useTranslation("review");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const was = row.pair
    ? t("transfersPage.unlinked.wasPair", {
        kind: t(`transfersPage.kind.${kindKey(row.pair.kind)}`),
        policy: t(`transfersPage.policyShort.${row.pair.policy === "taxable" ? "taxable" : "carrying"}`),
      })
    : t("transfersPage.unlinked.wasAutomatic");
  const route = t("inbox.transfers.route", { from: row.fromLabel, to: row.toLabel });
  return (
    <li
      className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 px-3 py-2.5 sm:px-6 @6xl/list:grid-cols-[7.5rem_minmax(0,1fr)_9rem_auto]"
      data-testid="transfers-unlinked-row"
    >
      <span className="text-xs text-muted-foreground tabular-nums sm:text-sm">{dateTime(row.occurredAt)}</span>
      <span className="col-span-2 row-start-2 min-w-0 @6xl/list:col-span-1 @6xl/list:col-start-2 @6xl/list:row-start-1">
        <span className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-2">
          <span className="flex min-w-0 items-center gap-1.5">
            <RailIcon rail={row.fromRail} size="compact" />
            <span className={cn("truncate text-sm font-medium", blurClass(hideSensitive))}>{row.fromLabel}</span>
          </span>
          <ArrowRight className="size-3.5 shrink-0 text-muted-foreground" aria-label={t("inbox.transfers.to")} />
          <span className="flex min-w-0 items-center gap-1.5">
            <RailIcon rail={row.toRail} size="compact" />
            <span className={cn("truncate text-sm font-medium", blurClass(hideSensitive))}>{row.toLabel}</span>
          </span>
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-1.5 text-2xs text-muted-foreground sm:text-xs">
          <Unlink className="size-3" aria-hidden="true" />
          <span>{t("transfersPage.unlinked.on", { date: datePart(row.unlinkedAt) })}</span>
          <span aria-hidden="true">·</span>
          <span>{was}</span>
        </span>
      </span>
      <span className={cn("col-start-2 row-start-1 text-right text-sm font-medium tabular-nums @6xl/list:col-start-3", blurClass(hideSensitive))}>
        {formatMsatAsBtc(row.amountMsat)}
      </span>
      <span className="col-span-2 row-start-3 flex flex-wrap justify-end gap-1.5 @6xl/list:col-span-1 @6xl/list:col-start-4 @6xl/list:row-start-1">
        <Button
          type="button"
          size="sm"
          className="h-8 gap-1.5"
          disabled={pending}
          onClick={onLinkAgain}
          aria-label={t("transfersPage.unlinked.linkAgainLabel", { route })}
        >
          <Link2 className="size-3.5" aria-hidden="true" />
          {row.pair ? t("transfersPage.unlinked.linkAgain") : t("transfersPage.unlinked.linkBack")}
        </Button>
        <Button type="button" size="sm" variant="ghost" className="h-8" disabled={pending} onClick={onLinkDifferently}>
          {t("transfersPage.unlinked.linkDifferently")}
        </Button>
        {/* Not a transfer after all: each side says what it was on its own page. */}
        <RowMenu route={route} outId={row.outTransactionId} inId={row.inTransactionId} sayWhat />
      </span>
    </li>
  );
}
