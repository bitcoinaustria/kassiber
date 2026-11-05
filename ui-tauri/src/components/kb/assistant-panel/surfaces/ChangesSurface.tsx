import * as React from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Bot,
  ChevronRight,
  GitCompareArrows,
  Loader2,
  Monitor,
  RotateCcw,
  TerminalSquare,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { useDaemonInfinite } from "@/daemon/client";
import { confirmRevert, useActivityHistoryRevert } from "@/hooks/useActivityHistoryRevert";
import {
  formatHistoryDate,
  formatHistoryRelative,
  historyTransactionLabel,
  type HistoryRevertTarget,
  type TransactionHistoryEvent,
  type TransactionHistoryField,
  type TransactionHistoryList,
} from "@/lib/transactionHistory";
import { cn } from "@/lib/utils";
import { useRightPanelStore } from "@/store/rightPanel";
import { useUiStore } from "@/store/ui";

import { SurfaceEmpty, SurfaceFrame } from "./SurfaceLayout";

type SourceFilter = "all" | "ai";

const SOURCE_ICON = { ai_tool: Bot, gui: Monitor } as Record<string, typeof Bot>;
const SOURCE_TONE: Record<string, string> = {
  ai_tool: "text-sky-600 dark:text-sky-400",
  gui: "text-emerald-600 dark:text-emerald-400",
};

/**
 * Recent edits to the book, newest first, after T3 Code's diff panel: one
 * line per edit, unfolding into each field's before and after, with undo.
 * It reads the Activity page's history (`ui.activity.history`); undo writes
 * a new history entry through the same revert the page uses.
 */
export function ChangesSurface() {
  const { t } = useTranslation(["assistant", "review", "transactions"]);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const openTransaction = useRightPanelStore((s) => s.openTransaction);
  const [source, setSource] = React.useState<SourceFilter>("all");
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set());
  const args = React.useMemo(
    () => ({ limit: 30, include_stale: false, ...(source === "ai" ? { ai_only: true } : {}) }),
    [source],
  );
  const history = useDaemonInfinite<TransactionHistoryList>(
    "ui.activity.history",
    args,
    (page) => page.data?.next_cursor ?? undefined,
  );
  const events = (history.data?.pages ?? []).flatMap((page) => page.data?.events ?? []);

  const { t: tTransactions } = useTranslation("transactions");
  const { revert, isReverting } = useActivityHistoryRevert();
  const onRevert = (target: HistoryRevertTarget) =>
    void confirmRevert(
      target,
      tTransactions as (key: string, opts?: Record<string, unknown>) => string, // loose translator
      (confirmed) => void revert(confirmed),
    );

  const toggle = (id: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  let body: React.ReactNode;
  if (history.isLoading) {
    body = (
      <div className="flex h-full min-h-48 items-center justify-center" role="status">
        <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" />
        <span className="sr-only">{t("assistant:panel.changes.loading")}</span>
      </div>
    );
  } else if (history.isError && events.length === 0) {
    body = (
      <SurfaceEmpty
        icon={AlertTriangle}
        tone="destructive"
        title={t("assistant:panel.changes.error")}
        action={
          <Button type="button" size="sm" variant="outline" onClick={() => void history.refetch()}>
            {t("assistant:panel.retry")}
          </Button>
        }
      />
    );
  } else if (events.length === 0) {
    body = (
      <SurfaceEmpty
        icon={GitCompareArrows}
        title={t("assistant:panel.changes.emptyTitle")}
        body={t(
          source === "ai" ? "assistant:panel.changes.emptyAssistant" : "assistant:panel.changes.emptyBody",
        )}
      />
    );
  } else {
    body = (
      <>
        <ul className="flex flex-col p-2">
          {events.map((event) => (
            <ChangeRow
              key={event.id}
              event={event}
              expanded={expanded.has(event.id)}
              onToggle={() => toggle(event.id)}
              hideSensitive={hideSensitive}
              onOpenTransaction={() => openTransaction(event.transaction_id)}
              onRevert={onRevert}
              reverting={isReverting}
            />
          ))}
        </ul>
        {history.hasNextPage ? (
          <div className="flex justify-center px-3 pb-3">
            <Button
              type="button"
              size="xs"
              variant="ghost"
              className="text-muted-foreground"
              disabled={history.isFetchingNextPage}
              onClick={() => void history.fetchNextPage()}
            >
              {history.isFetchingNextPage
                ? t("review:activity.loadingMore")
                : t("review:activity.loadMore")}
            </Button>
          </div>
        ) : null}
      </>
    );
  }

  return (
    <SurfaceFrame
      label={t("assistant:panel.surfaceChanges")}
      toolbar={
        <>
          <div
            role="radiogroup"
            aria-label={t("assistant:panel.changes.filter")}
            className="flex items-center gap-0.5 rounded-md bg-muted/60 p-0.5"
          >
            {(["all", "ai"] as const).map((value) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={source === value}
                className={cn(
                  "flex h-5 items-center gap-1 rounded-sm px-2 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  source === value
                    ? "bg-background text-foreground shadow-xs"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => setSource(value)}
              >
                {value === "ai" ? <Bot className="size-3" aria-hidden="true" /> : null}
                {t(value === "ai" ? "assistant:panel.changes.byAssistant" : "assistant:panel.changes.all")}
              </button>
            ))}
          </div>
          <span className="ml-auto text-muted-foreground tabular-nums">
            {history.isFetching && !history.isLoading ? (
              <Loader2 className="size-3 animate-spin" aria-hidden="true" />
            ) : events.length ? (
              t("assistant:panel.changes.count", { count: events.length })
            ) : null}
          </span>
        </>
      }
    >
      {body}
    </SurfaceFrame>
  );
}

function ChangeRow({
  event,
  expanded,
  onToggle,
  hideSensitive,
  onOpenTransaction,
  onRevert,
  reverting,
}: {
  event: TransactionHistoryEvent;
  expanded: boolean;
  onToggle: () => void;
  hideSensitive: boolean;
  onOpenTransaction: () => void;
  onRevert: (target: HistoryRevertTarget) => void;
  reverting: boolean;
}) {
  const { t } = useTranslation(["assistant", "transactions"]);
  const Icon = SOURCE_ICON[event.source] ?? TerminalSquare;
  const panelId = React.useId();
  const stale = Boolean(event.report_anchor?.stale_for_reports);
  return (
    <li className={cn("rounded-md", expanded && "bg-muted/30")}>
      <div className="flex min-w-0 items-start gap-1 rounded-md px-1 py-1 hover:bg-accent/40">
        <button
          type="button"
          aria-expanded={expanded}
          aria-controls={panelId}
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-start gap-2 rounded-sm px-1 py-0.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <ChevronRight
            className={cn(
              "mt-0.5 size-3.5 shrink-0 text-muted-foreground transition-transform",
              expanded && "rotate-90",
            )}
            aria-hidden="true"
          />
          <Icon
            className={cn("mt-0.5 size-3.5 shrink-0", SOURCE_TONE[event.source] ?? "text-muted-foreground")}
            aria-label={event.source_label}
          />
          <span className={cn("min-w-0 flex-1 truncate text-sm", hideSensitive && "sensitive")}>
            {event.summary}
          </span>
        </button>
        <time
          dateTime={event.changed_at}
          title={formatHistoryDate(event.changed_at)}
          className="shrink-0 px-1 pt-0.5 text-xs text-muted-foreground tabular-nums"
        >
          {formatHistoryRelative(event.changed_at)}
        </time>
      </div>
      <div className="flex min-w-0 items-center gap-1.5 pb-1 pr-2 pl-[2.625rem] text-xs text-muted-foreground">
        <button
          type="button"
          onClick={onOpenTransaction}
          className={cn(
            "min-w-0 truncate rounded-sm font-mono text-[0.6875rem] outline-none hover:text-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring",
            hideSensitive && "sensitive",
          )}
          title={t("assistant:panel.changes.openTransaction")}
        >
          {shortId(historyTransactionLabel(event))}
        </button>
        {event.wallet_label ? (
          <span className={cn("min-w-0 truncate", hideSensitive && "sensitive")}>· {event.wallet_label}</span>
        ) : null}
        <span className="ml-auto flex shrink-0 items-center gap-1">
          {stale ? (
            <span className="rounded-sm bg-amber-500/10 px-1 text-amber-700 dark:text-amber-300">
              {t("transactions:history.staleReports")}
            </span>
          ) : null}
          {event.families.map((family) => (
            <span key={family} className="rounded-sm bg-muted px-1">
              {family}
            </span>
          ))}
        </span>
      </div>
      {expanded ? (
        <div id={panelId} className="space-y-2 px-2 pb-2 pl-[2.625rem]">
          {event.reason ? (
            <p className={cn("text-xs text-muted-foreground", hideSensitive && "sensitive")}>
              {event.reason}
            </p>
          ) : null}
          {event.fields.map((field) => (
            <FieldDiff
              key={field.id}
              field={field}
              hideSensitive={hideSensitive}
              reverting={reverting}
              onRevert={() => onRevert({ event, field })}
            />
          ))}
          <Button
            type="button"
            size="xs"
            variant="outline"
            disabled={reverting}
            onClick={() => onRevert({ event })}
          >
            <RotateCcw aria-hidden="true" />
            {t("assistant:panel.changes.undo")}
          </Button>
        </div>
      ) : null}
    </li>
  );
}

/** One field as a two-line diff: what it was, what it became. */
function FieldDiff({
  field,
  hideSensitive,
  reverting,
  onRevert,
}: {
  field: TransactionHistoryField;
  hideSensitive: boolean;
  reverting: boolean;
  onRevert: () => void;
}) {
  const { t } = useTranslation("transactions");
  const added = field.diff?.added ?? [];
  const removed = field.diff?.removed ?? [];
  const tagged = added.length > 0 || removed.length > 0;
  return (
    <div className="group/field overflow-hidden rounded-md border border-border/70 bg-background">
      <div className="flex h-7 items-center gap-2 border-b border-border/70 bg-muted/40 pr-1 pl-2 text-xs">
        <span className="min-w-0 flex-1 truncate font-medium">{field.label}</span>
        <span className="text-muted-foreground">{field.family}</span>
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          className="size-6 opacity-0 group-hover/field:opacity-100 focus-visible:opacity-100"
          disabled={reverting}
          aria-label={t("history.revertFieldAria", { label: field.label })}
          title={t("history.revertFieldAria", { label: field.label })}
          onClick={onRevert}
        >
          <RotateCcw className="size-3" aria-hidden="true" />
        </Button>
      </div>
      <div className={cn("font-mono text-xs leading-5", hideSensitive && "sensitive")}>
        {tagged ? (
          <>
            {removed.map((tag) => (
              <DiffLine key={`-${tag}`} kind="removed" text={tag} />
            ))}
            {added.map((tag) => (
              <DiffLine key={`+${tag}`} kind="added" text={tag} />
            ))}
          </>
        ) : (
          <>
            <DiffLine kind="removed" text={field.before_label} label={t("history.before")} />
            <DiffLine kind="added" text={field.after_label} label={t("history.after")} />
          </>
        )}
      </div>
      {field.redacted ? (
        <p className="border-t border-border/70 px-2 py-1 text-xs text-muted-foreground">
          {t("history.redacted")}
        </p>
      ) : null}
    </div>
  );
}

function DiffLine({ kind, text, label }: { kind: "added" | "removed"; text: string; label?: string }) {
  return (
    <div
      className={cn(
        "flex min-w-0 gap-2 px-2",
        kind === "added"
          ? "bg-emerald-500/10 text-emerald-800 dark:text-emerald-300"
          : "bg-rose-500/10 text-rose-800 dark:text-rose-300",
      )}
    >
      <span aria-hidden="true" className="w-2 shrink-0 select-none opacity-70">
        {kind === "added" ? "+" : "−"}
      </span>
      {label ? <span className="sr-only">{label}: </span> : null}
      <span className="min-w-0 break-words whitespace-pre-wrap">{text || "—"}</span>
    </div>
  );
}

function shortId(id: string) {
  return id.length > 18 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;
}
