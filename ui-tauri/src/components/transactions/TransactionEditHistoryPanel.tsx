import {
  AlertTriangle,
  Bot,
  ChevronRight,
  Clock,
  History,
  Monitor,
  RefreshCw,
  RotateCcw,
  TerminalSquare,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  formatHistoryDate,
  formatHistoryRelative,
  historyTransactionLabel,
  transactionHistoryFamilyClass,
  transactionHistorySourceClass,
  type HistoryRevertTarget,
  type TransactionHistoryEvent,
  type TransactionHistoryField,
  type TransactionHistoryStaleSummary,
} from "@/lib/transactionHistory";

function sourceIcon(source: string) {
  if (source === "ai_tool") return Bot;
  if (source === "gui") return Monitor;
  return TerminalSquare;
}

function hiddenClass(hideSensitive: boolean) {
  return hideSensitive ? "blur-sm select-none" : "";
}

function confirmRevert(target: HistoryRevertTarget, onRevert?: (target: HistoryRevertTarget) => void) {
  if (!onRevert) return;
  const fieldLabel = target.field ? target.field.label : "all fields in this edit";
  const ok = window.confirm(`Revert ${fieldLabel}? This creates a new edit history entry.`);
  if (!ok) return;
  onRevert(target);
}

function FieldDiffRow({
  event,
  field,
  hideSensitive,
  onRevert,
  isReverting,
}: {
  event: TransactionHistoryEvent;
  field: TransactionHistoryField;
  hideSensitive: boolean;
  onRevert?: (target: HistoryRevertTarget) => void;
  isReverting?: boolean;
}) {
  const added = field.diff?.added ?? [];
  const removed = field.diff?.removed ?? [];
  return (
    <div className="grid gap-2 border-t px-3 py-2 text-xs first:border-t-0 sm:grid-cols-[8rem_minmax(0,1fr)_auto]">
      <div className="flex min-w-0 items-center gap-1.5">
        <Badge
          variant="outline"
          className={cn("rounded-md text-[11px]", transactionHistoryFamilyClass(field.family))}
        >
          {field.family}
        </Badge>
        <span className="truncate font-medium text-foreground">{field.label}</span>
      </div>
      <div className="grid min-w-0 gap-1 sm:grid-cols-2">
        <div className="min-w-0 rounded border bg-muted/40 px-2 py-1">
          <div className="text-[10px] uppercase text-muted-foreground">Before</div>
          <div className={cn("truncate", hiddenClass(hideSensitive))}>
            {field.before_label}
          </div>
        </div>
        <div className="min-w-0 rounded border bg-background px-2 py-1">
          <div className="text-[10px] uppercase text-muted-foreground">After</div>
          <div className={cn("truncate", hiddenClass(hideSensitive))}>
            {field.after_label}
          </div>
        </div>
        {added.length || removed.length ? (
          <div className="sm:col-span-2 flex flex-wrap gap-1 text-[11px] text-muted-foreground">
            {added.map((tag) => (
              <span key={`add-${tag}`} className={cn("rounded bg-emerald-100 px-1.5 py-0.5 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300", hiddenClass(hideSensitive))}>
                +{tag}
              </span>
            ))}
            {removed.map((tag) => (
              <span key={`remove-${tag}`} className={cn("rounded bg-rose-100 px-1.5 py-0.5 text-rose-700 dark:bg-rose-950 dark:text-rose-300", hiddenClass(hideSensitive))}>
                -{tag}
              </span>
            ))}
          </div>
        ) : null}
        {field.redacted ? (
          <div className="sm:col-span-2 text-[11px] text-muted-foreground">
            Secret-shaped material was redacted for display.
          </div>
        ) : null}
      </div>
      {onRevert ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-8 justify-self-end"
          disabled={isReverting}
          aria-label={`Revert ${field.label}`}
          onClick={() => confirmRevert({ event, field }, onRevert)}
        >
          <RotateCcw className="size-3.5" aria-hidden="true" />
        </Button>
      ) : null}
    </div>
  );
}

export function TransactionHistoryTimeline({
  events,
  hideSensitive,
  isLoading,
  emptyLabel = "No edit history yet",
  showTransaction = false,
  onRevert,
  isReverting,
}: {
  events: TransactionHistoryEvent[];
  hideSensitive: boolean;
  isLoading?: boolean;
  emptyLabel?: string;
  showTransaction?: boolean;
  onRevert?: (target: HistoryRevertTarget) => void;
  isReverting?: boolean;
}) {
  if (isLoading && events.length === 0) {
    return (
      <div className="rounded-md border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
        Loading edit history...
      </div>
    );
  }
  if (!events.length) {
    return (
      <div className="rounded-md border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
        {emptyLabel}
      </div>
    );
  }
  return (
    <div className="space-y-2">
      {events.map((event) => {
        const Icon = sourceIcon(event.source);
        return (
          <details
            key={event.id}
            className="group rounded-md border bg-card"
          >
            <summary className="grid cursor-pointer list-none gap-2 p-3 text-sm sm:grid-cols-[minmax(0,1fr)_auto]">
              <div className="min-w-0 space-y-1">
                <div className="flex min-w-0 flex-wrap items-center gap-2">
                  <ChevronRight className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" aria-hidden="true" />
                  <Badge
                    variant="outline"
                    className={cn("rounded-md gap-1", transactionHistorySourceClass(event.source))}
                  >
                    <Icon className="size-3" aria-hidden="true" />
                    {event.source_label}
                  </Badge>
                  {event.families.map((family) => (
                    <Badge
                      key={family}
                      variant="outline"
                      className={cn("rounded-md", transactionHistoryFamilyClass(family))}
                    >
                      {family}
                    </Badge>
                  ))}
                  {event.report_anchor?.stale_for_reports ? (
                    <Badge variant="outline" className="rounded-md border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-800 dark:bg-amber-950/40 dark:text-amber-300">
                      stale reports
                    </Badge>
                  ) : null}
                </div>
                <div className={cn("truncate font-medium text-foreground", hiddenClass(hideSensitive))}>
                  {event.summary}
                </div>
                {showTransaction ? (
                  <a
                    className="inline-flex min-w-0 max-w-full truncate text-xs text-primary underline-offset-2 hover:underline"
                    href={`/transactions?tx=${encodeURIComponent(event.transaction_id)}`}
                  >
                    {historyTransactionLabel(event)}
                    {event.wallet_label ? ` · ${event.wallet_label}` : ""}
                  </a>
                ) : null}
                {event.reason ? (
                  <div className={cn("truncate text-xs text-muted-foreground", hiddenClass(hideSensitive))}>
                    {event.reason}
                  </div>
                ) : null}
              </div>
              <div className="flex items-start justify-between gap-2 sm:justify-end">
                <div className="text-right text-xs text-muted-foreground">
                  <div>{formatHistoryRelative(event.changed_at)}</div>
                  <div>{formatHistoryDate(event.changed_at)}</div>
                </div>
                {onRevert ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-8"
                    disabled={isReverting}
                    aria-label="Revert edit"
                    onClick={(clickEvent) => {
                      clickEvent.preventDefault();
                      confirmRevert({ event }, onRevert);
                    }}
                  >
                    <RotateCcw className="size-3.5" aria-hidden="true" />
                  </Button>
                ) : null}
              </div>
            </summary>
            <div className="border-t">
              {event.fields.map((field) => (
                <FieldDiffRow
                  key={field.id}
                  event={event}
                  field={field}
                  hideSensitive={hideSensitive}
                  onRevert={onRevert}
                  isReverting={isReverting}
                />
              ))}
            </div>
          </details>
        );
      })}
    </div>
  );
}

export function TransactionEditHistoryPanel({
  events,
  stale,
  hideSensitive,
  isLoading,
  onRevert,
  isReverting,
  onProcessJournals,
  isProcessingJournals,
}: {
  events?: TransactionHistoryEvent[];
  stale?: TransactionHistoryStaleSummary;
  hideSensitive: boolean;
  isLoading?: boolean;
  onRevert?: (target: HistoryRevertTarget) => void;
  isReverting?: boolean;
  onProcessJournals?: () => void;
  isProcessingJournals?: boolean;
}) {
  const staleCount = stale?.edit_count ?? 0;
  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <History className="size-4 text-muted-foreground" aria-hidden="true" />
          Edit history
        </div>
        {events?.length ? (
          <Badge variant="secondary" className="rounded-md">
            {events.length}
          </Badge>
        ) : null}
      </div>
      {staleCount > 0 ? (
        <div className="mb-3 flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 p-2 text-xs text-amber-900 dark:border-amber-900 dark:bg-amber-950/40 dark:text-amber-200">
          <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <div className="font-medium">{staleCount} edit{staleCount === 1 ? "" : "s"} after the last journal run</div>
            <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-amber-800 dark:text-amber-300">
              <Clock className="size-3" aria-hidden="true" />
              {stale?.latest_changed_at
                ? formatHistoryDate(stale.latest_changed_at)
                : "Reports need a fresh journal state"}
            </div>
          </div>
          {onProcessJournals ? (
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-7 gap-1.5 border-amber-300 bg-amber-100 text-amber-900 hover:bg-amber-200 dark:border-amber-800 dark:bg-amber-950 dark:text-amber-100"
              disabled={isProcessingJournals}
              onClick={onProcessJournals}
            >
              <RefreshCw className={cn("size-3", isProcessingJournals && "animate-spin")} aria-hidden="true" />
              Process
            </Button>
          ) : null}
        </div>
      ) : null}
      <TransactionHistoryTimeline
        events={events ?? []}
        hideSensitive={hideSensitive}
        isLoading={isLoading}
        onRevert={onRevert}
        isReverting={isReverting}
      />
    </div>
  );
}
