/**
 * The list "Link manually" picks one side from: the book's sends or
 * receipts in one aligned table, ranked against the side already chosen.
 *
 * After rotki's "Find match": candidates come from a window around the
 * chosen side, close amounts and times first, the matcher's own suggestion
 * marked. Unlike rotki there is no amount tolerance to tune: every candidate
 * in the window stays listed with what it differs by, and a difference that
 * is no fee is coloured rather than hidden. A linked or ignored transaction
 * stays listed too, last, and says why it can't be picked.
 *
 * One search field and one time choice are the only filters. When the time
 * leaves nothing that fits, the list says what it looked through and offers
 * the next wider time in one step (rotki's "Widen search"). The rows are
 * read from the book (`ui.transactions.list`); nothing leaves the machine.
 */
import * as React from "react";
import { keepPreviousData } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight, Check, Loader2, Search, Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import type { TransferSuggestions } from "@/components/review/transferModel";
import { blurClass } from "@/components/transactions/model";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useDaemon } from "@/daemon/client";
import { transactionTypeLabel } from "@/lib/transactionTypeLabel";
import type { TransactionsList } from "@/mocks/transactions";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { LegAmount, WalletName } from "./figures";
import {
  bpsText,
  byTypedAmount,
  datePart,
  dateTime,
  hasOpenPart,
  legFromTx,
  linkableMsat,
  listCandidates,
  PICK_LIMIT,
  rankCandidates,
  RECEIPT_WINDOWS,
  searchIntent,
  timePart,
  widerWindow,
  windowBounds,
  windowBoundsBefore,
  withOpenPart,
  yearBounds,
  type Candidate,
  type Difference,
  type LegSide,
  type LinkLeg,
  type OpenPart,
  type ReceiptWindow,
} from "./linkModel";
import { formatMsatAsBtc, formatMsatAsSats } from "./satsFormat";

const ANY = "any";
/** The first year a bitcoin transaction can carry. */
const FIRST_YEAR = 2009;

type Translate = ReturnType<typeof useTranslation<"review">>["t"];

function relativeTime(t: Translate, minutes: number): string {
  const magnitude = Math.abs(minutes);
  const time =
    magnitude < 90
      ? t("transfersPage.link.minutes", { count: magnitude })
      : magnitude < 48 * 60
        ? t("transfersPage.link.hours", { count: Math.round(magnitude / 60) })
        : t("transfersPage.link.days", { count: Math.round(magnitude / 1440) });
  return t(minutes >= 0 ? "transfersPage.link.after" : "transfersPage.link.before", { time });
}

/** A difference in a few words, for a table cell. */
function differenceShort(t: Translate, difference: Difference): string {
  switch (difference.kind) {
    case "otherAsset":
      return t("transfersPage.link.diff.otherAsset");
    case "none":
      return t("transfersPage.link.diff.same");
    case "negative":
      return t("transfersPage.link.diff.more", { amount: formatMsatAsSats(difference.amountMsat.toString()) });
    case "mismatch":
      return t("transfersPage.link.diff.lessShare", { share: bpsText(difference.shareBps ?? 0n) });
    default:
      return t("transfersPage.link.diff.less", { amount: formatMsatAsSats(difference.amountMsat.toString()) });
  }
}

const oddDifference = (difference: Difference | null) => difference?.kind === "negative" || difference?.kind === "mismatch";

const windowKey = (hours: ReceiptWindow) => (hours === null ? "any" : (`h${hours}` as const));
/** A time that can be widened: every choice but any time. */
type Span = Exclude<ReturnType<typeof windowKey>, "any">;
/** A time widening leads to: every choice but the shortest. */
type Wider = Exclude<ReturnType<typeof windowKey>, "h6">;

export function LinkCandidates({
  side,
  anchor,
  parts = null,
  selectedId,
  onPick,
  onOpen,
}: {
  /** The side being chosen. */
  side: LegSide;
  /** The side already chosen, which the candidates are ranked against. */
  anchor: LinkLeg | null;
  /** What each linked transaction has left open, so one linked in part can be picked. */
  parts?: Record<LegSide, Map<string, OpenPart>> | null;
  selectedId: string | null;
  onPick: (leg: LinkLeg) => void;
  /** Called before a row's transaction opens on Transactions. */
  onOpen?: () => void;
}) {
  const { t } = useTranslation("review");
  const [text, setText] = React.useState("");
  const [hours, setHours] = React.useState<ReceiptWindow>(72);
  const [year, setYear] = React.useState<number | null>(null);
  const intent = searchIntent(React.useDeferredValue(text));

  const bounds = anchor
    ? side === "receipt"
      ? windowBounds(anchor.occurredAt, hours)
      : windowBoundsBefore(anchor.occurredAt, hours)
    : yearBounds(year);
  const list = useDaemon<TransactionsList>(
    "ui.transactions.list",
    {
      direction: side === "send" ? "outbound" : "inbound",
      limit: PICK_LIMIT,
      ...(intent.query ? { query: intent.query } : {}),
      ...bounds,
    },
    { placeholderData: keepPreviousData },
  );
  const suggest = useDaemon<TransferSuggestions>("ui.transfers.suggest", {});
  const legs = React.useMemo(
    () =>
      (list.data?.data?.txs ?? [])
        .map((tx) => legFromTx(tx, side))
        .filter((leg): leg is LinkLeg => leg !== null)
        .map((leg) => withOpenPart(leg, parts)),
    [list.data, parts, side],
  );
  const suggestedIds = React.useMemo(() => {
    const ids = new Set<string>();
    if (!anchor) return ids;
    for (const candidate of suggest.data?.data?.candidates ?? []) {
      if (side === "receipt" && candidate.out_id === anchor.id) ids.add(candidate.in_id);
      if (side === "send" && candidate.in_id === anchor.id) ids.add(candidate.out_id);
    }
    return ids;
  }, [anchor, side, suggest.data]);
  const rows = React.useMemo(
    () => byTypedAmount(anchor ? rankCandidates(anchor, legs, suggestedIds) : listCandidates(legs), intent.amountMsat),
    [anchor, intent.amountMsat, legs, suggestedIds],
  );

  // One row is in the tab order; the arrow keys move between them.
  const rowRefs = React.useRef<Array<HTMLTableRowElement | null>>([]);
  const [focused, setFocused] = React.useState<number | null>(null);
  const selectedIndex = rows.findIndex((row) => row.leg.id === selectedId);
  const tabStop = Math.min(focused ?? (selectedIndex >= 0 ? selectedIndex : 0), Math.max(rows.length - 1, 0));
  const focusRow = (index: number) => {
    const next = Math.max(0, Math.min(rows.length - 1, index));
    setFocused(next);
    rowRefs.current[next]?.focus();
  };
  const onRowKey = (event: React.KeyboardEvent<HTMLTableRowElement>, index: number, row: Candidate) => {
    if (event.target !== event.currentTarget) return;
    const move: Record<string, number> = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: rows.length - 1 };
    if (event.key in move) {
      event.preventDefault();
      focusRow(move[event.key]);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!row.blocked) onPick(row.leg);
    }
  };

  const windowLabel = t(side === "receipt" ? "transfersPage.link.windowAfter" : "transfersPage.link.windowBefore");
  const limited = list.data?.data?.hasMore ?? legs.length >= PICK_LIMIT;
  const currentYear = new Date().getUTCFullYear();
  const narrowed = Boolean(text.trim()) || (anchor ? hours !== null : year !== null);
  // Only the time stands between the owner and more candidates: say what was
  // looked through and offer the next wider time, one step at a time.
  const wider = anchor && !text.trim() ? widerWindow(hours) : undefined;
  const searched =
    wider === undefined || hours === null
      ? null
      : t(side === "receipt" ? "transfersPage.link.searchedReceipts" : "transfersPage.link.searchedSends", {
          span: t(`transfersPage.link.span.${windowKey(hours) as Span}`),
        });
  const widen = wider === undefined ? null : (
    <Button type="button" size="sm" variant="outline" className="h-7" data-testid="link-widen" onClick={() => setHours(wider)}>
      {t(`transfersPage.link.widen.${windowKey(wider) as Wider}`)}
    </Button>
  );
  // Nothing in the list looks like the other half: the right one may lie outside the time.
  const thin = rows.length > 0 && !rows.some((row) => !row.blocked && (row.suggested || row.sameTxid || row.close));

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2" data-testid="link-pick" data-side={side}>
      <div className="flex items-center gap-2" role="group" aria-label={t("transfersPage.link.filters")}>
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" aria-hidden="true" />
          <Input
            value={text}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== "ArrowDown" || rows.length === 0) return;
              event.preventDefault();
              focusRow(0);
            }}
            placeholder={t("transfersPage.link.searchPlaceholder")}
            aria-label={t("transfersPage.link.search")}
            className="h-8 pl-8 text-sm"
          />
        </div>
        {anchor ? (
          <Select value={hours === null ? ANY : String(hours)} onValueChange={(value) => setHours(value === ANY ? null : (Number(value) as ReceiptWindow))}>
            <SelectTrigger className="h-8 w-[9.5rem] shrink-0 text-sm" aria-label={windowLabel}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RECEIPT_WINDOWS.map((option) => (
                <SelectItem key={option ?? ANY} value={option === null ? ANY : String(option)}>
                  {t(`transfersPage.link.window.${option === null ? "any" : (`h${option}` as const)}`)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : (
          <Select value={year === null ? ANY : String(year)} onValueChange={(value) => setYear(value === ANY ? null : Number(value))}>
            <SelectTrigger className="h-8 w-[9.5rem] shrink-0 text-sm" aria-label={t("transfersPage.link.year")}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ANY}>{t("transfersPage.link.window.any")}</SelectItem>
              {Array.from({ length: currentYear - FIRST_YEAR + 1 }, (_, index) => currentYear - index).map((option) => (
                <SelectItem key={option} value={String(option)}>
                  {option}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>

      <div className="@container/pick min-h-[11rem] flex-1 overflow-y-auto rounded-lg border" aria-busy={list.isFetching}>
        {list.isLoading ? (
          <p className="flex h-24 items-center justify-center gap-2 text-sm text-muted-foreground" role="status">
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            {t("transfersPage.link.loading")}
          </p>
        ) : list.isError || list.data?.error ? (
          <div className="space-y-2 p-4 text-sm" role="alert">
            <p className="text-destructive">{t("transfersPage.link.unavailable")}</p>
            <Button type="button" size="sm" variant="outline" onClick={() => void list.refetch()}>
              {t("transfersPage.effects.retry")}
            </Button>
          </div>
        ) : rows.length === 0 ? (
          <div className="space-y-1 p-4 text-sm" data-testid="link-pick-empty">
            <p className="font-medium">{searched ?? t(side === "receipt" ? "transfersPage.link.emptyReceipts" : "transfersPage.link.emptySends")}</p>
            {searched ? (
              <div className="pt-1">{widen}</div>
            ) : (
              <p className="text-muted-foreground">{t(narrowed ? "transfersPage.link.emptyNarrowed" : "transfersPage.link.emptyBook")}</p>
            )}
            {narrowed && !searched ? (
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="mt-2"
                onClick={() => {
                  setText("");
                  setHours(null);
                  setYear(null);
                }}
              >
                {t("transfersPage.link.showAll")}
              </Button>
            ) : null}
          </div>
        ) : (
          <table className="w-full table-fixed border-collapse text-sm" aria-label={t(side === "receipt" ? "transfersPage.link.receiptsLabel" : "transfersPage.link.sendsLabel")}>
            <colgroup>
              <col className="w-[7rem] @md/pick:w-[7.75rem]" />
              <col />
              <col className="w-[8rem]" />
              {anchor ? <col className="hidden w-[8.5rem] @lg/pick:table-column" /> : null}
              <col className="hidden w-[7.5rem] @2xl/pick:table-column" />
              <col className="w-[2.25rem]" />
            </colgroup>
            <thead className="sticky top-0 z-10 bg-background text-left text-xs text-muted-foreground shadow-[0_1px_0_var(--border)]">
              <tr>
                <th scope="col" className="px-3 py-1.5 font-medium">{t("transfersPage.link.col.when")}</th>
                <th scope="col" className="px-2 py-1.5 font-medium">{t("transfersPage.link.col.wallet")}</th>
                <th scope="col" className="px-2 py-1.5 text-right font-medium">{t("transfersPage.link.col.amount")}</th>
                {anchor ? (
                  <th scope="col" className="hidden px-2 py-1.5 text-right font-medium @lg/pick:table-cell">{t("transfersPage.link.col.difference")}</th>
                ) : null}
                <th scope="col" className="hidden px-2 py-1.5 font-medium @2xl/pick:table-cell">{t("transfersPage.link.col.bookedAs")}</th>
                <th scope="col">
                  <span className="sr-only">{t("transfersPage.showInTransactions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row, index) => (
                <CandidateRow
                  key={row.leg.id}
                  ref={(element) => {
                    rowRefs.current[index] = element;
                  }}
                  row={row}
                  anchored={Boolean(anchor)}
                  selected={row.leg.id === selectedId}
                  tabStop={index === tabStop}
                  onFocus={() => setFocused(index)}
                  onKeyDown={(event) => onRowKey(event, index, row)}
                  onPick={() => onPick(row.leg)}
                  onOpen={onOpen}
                />
              ))}
            </tbody>
          </table>
        )}
      </div>
      {thin && searched && !list.isLoading ? (
        <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground" data-testid="link-pick-thin">
          <span>{t(side === "receipt" ? "transfersPage.link.onlyReceipts" : "transfersPage.link.onlySends", { span: t(`transfersPage.link.span.${windowKey(hours) as Span}`) })}</span>
          {widen}
        </p>
      ) : null}
      {/* The read stops at PICK_LIMIT rows; say so rather than look complete. */}
      {limited && rows.length > 0 ? (
        <p className="text-xs text-muted-foreground" data-testid="link-pick-limited">
          {t(anchor ? "transfersPage.link.limitedWindow" : "transfersPage.link.limited", { count: legs.length })}
        </p>
      ) : null}
    </div>
  );
}

const CandidateRow = React.forwardRef<
  HTMLTableRowElement,
  {
    row: Candidate;
    anchored: boolean;
    selected: boolean;
    tabStop: boolean;
    onFocus: () => void;
    onKeyDown: (event: React.KeyboardEvent<HTMLTableRowElement>) => void;
    onPick: () => void;
    onOpen?: () => void;
  }
>(function CandidateRow({ row, anchored, selected, tabStop, onFocus, onKeyDown, onPick, onOpen }, ref) {
  const { t } = useTranslation("review");
  const { t: tTransactions } = useTranslation("transactions");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const { leg } = row;
  const time = timePart(leg.occurredAt);
  const difference = anchored && !row.blocked && row.difference ? differenceShort(t, row.difference) : null;
  const differenceClass = cn(
    "tabular-nums",
    oddDifference(row.difference) ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground",
    blurClass(hideSensitive),
  );
  const early = row.notes.includes("before");
  const bookedAs = leg.excluded
    ? t("transfersPage.link.booked.ignored")
    : leg.held
      ? t("transfersPage.link.booked.held")
      : transactionTypeLabel(tTransactions, leg.bookedAs) || "—";
  return (
    <tr
      ref={ref}
      tabIndex={tabStop ? 0 : -1}
      aria-selected={selected}
      aria-disabled={row.blocked ? true : undefined}
      data-testid="link-candidate"
      data-blocked={row.blocked ?? undefined}
      onFocus={onFocus}
      onKeyDown={onKeyDown}
      onClick={row.blocked ? undefined : onPick}
      className={cn(
        "border-b align-top transition-colors last:border-b-0 focus-visible:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
        row.blocked ? "cursor-not-allowed text-muted-foreground" : "cursor-pointer hover:bg-muted/50",
        selected && "bg-primary/5",
      )}
    >
      <td className="py-2 pr-2 pl-3 tabular-nums" title={dateTime(leg.occurredAt)}>
        <span className="block">{datePart(leg.occurredAt)}</span>
        {/* Next to a chosen side, how far apart the two are says more than the clock. */}
        {anchored && row.minutes !== null ? (
          <span className={cn("block text-xs", early ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground")}>{relativeTime(t, row.minutes)}</span>
        ) : time ? (
          <span className="block text-xs text-muted-foreground">{time}</span>
        ) : null}
      </td>
      <td className="overflow-hidden px-2 py-2">
        <span className="flex min-w-0 items-center gap-1.5">
          {/* Narrow, the name needs the room more than the network's logo does. */}
          {selected ? (
            <Check className="size-3.5 shrink-0 text-primary" aria-hidden="true" />
          ) : (
            <span className="hidden shrink-0 @md/pick:inline-flex">
              <RailIcon rail={leg.rail} size="compact" />
            </span>
          )}
          <WalletName name={leg.wallet} className={cn("font-medium", row.blocked && "font-normal")} />
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground empty:hidden">
          {row.blocked === "linked" ? (
            <span className={blurClass(hideSensitive)}>
              {leg.link?.wallet ? t("transfersPage.link.linkedTo", { wallet: leg.link.wallet }) : t("transfersPage.link.alreadyLinked")}
            </span>
          ) : row.blocked === "ignored" ? (
            <span>{t("transfersPage.link.ignored")}</span>
          ) : row.blocked === "rest" ? (
            <span>{t("transfersPage.link.restOnly")}</span>
          ) : (
            <>
              {row.suggested ? (
                <Badge variant="outline" className="gap-1 rounded-md border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0 text-2xs text-emerald-800 dark:text-emerald-200">
                  <Sparkles className="size-3" aria-hidden="true" />
                  {t("transfersPage.link.suggested")}
                </Badge>
              ) : row.sameTxid ? (
                <Badge variant="outline" className="rounded-md border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0 text-2xs text-emerald-800 dark:text-emerald-200">
                  {t("transfersPage.link.sameTxid")}
                </Badge>
              ) : row.close ? (
                <Badge variant="outline" className="rounded-md px-1.5 py-0 text-2xs">
                  {t("transfersPage.link.closeMatch")}
                </Badge>
              ) : null}
              {hasOpenPart(leg) ? (
                <span className={cn("tabular-nums", blurClass(hideSensitive))} data-testid="link-candidate-part">
                  {t("transfersPage.link.partLeft", {
                    left: formatMsatAsBtc(linkableMsat(leg).toString()),
                    total: formatMsatAsBtc(((leg.open?.linkedMsat ?? 0n) + linkableMsat(leg)).toString()),
                  })}
                </span>
              ) : null}
              {row.notes.includes("sameWallet") ? <span className="text-amber-700 dark:text-amber-300">{t("transfersPage.link.sameWallet")}</span> : null}
              {/* The "Booked as" column says this when there is room for it. */}
              {leg.held ? <span className="text-amber-700 @2xl/pick:hidden dark:text-amber-300">{t("transfersPage.link.booked.held")}</span> : null}
            </>
          )}
        </span>
      </td>
      <td className="px-2 py-2 text-right">
        <LegAmount leg={leg} toggle={false} className={row.blocked ? undefined : "font-medium"} />
        {/* Narrow, the difference sits under the amount instead of in its own column. */}
        {difference ? <span className={cn("block text-xs @lg/pick:hidden", differenceClass)}>{difference}</span> : null}
      </td>
      {anchored ? (
        <td className={cn("hidden px-2 py-2 text-right @lg/pick:table-cell", differenceClass)}>{difference ?? "—"}</td>
      ) : null}
      <td
        className={cn(
          "hidden truncate px-2 py-2 @2xl/pick:table-cell",
          leg.held && !leg.excluded ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground",
        )}
      >
        {bookedAs}
      </td>
      <td className="py-1.5 pr-2 text-right">
        {/* The row picks; this opens the transaction instead. */}
        <Link
          to="/transactions"
          search={{ tx: leg.id }}
          onClick={(event) => {
            event.stopPropagation();
            onOpen?.();
          }}
          onKeyDown={(event) => event.stopPropagation()}
          // With its row, one stop in the tab order.
          tabIndex={tabStop ? 0 : -1}
          aria-label={t("transfersPage.showInTransactions")}
          title={t("transfersPage.showInTransactions")}
          className="inline-grid size-6 place-items-center rounded-sm text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
        >
          <ArrowUpRight className="size-3.5" aria-hidden="true" />
        </Link>
      </td>
    </tr>
  );
});
