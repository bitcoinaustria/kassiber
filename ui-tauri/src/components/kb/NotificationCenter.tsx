import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import type { TFunction } from "i18next";
import {
  AlertTriangle,
  Bell,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Info,
  Loader2,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { formatDateTime, formatRelativeTime } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";
import {
  conditionsInScope,
  groupConditions,
  MAX_RESULTS,
  useNotificationStore,
  type ConditionCode,
  type ConditionGroup,
  type JobEntry,
  type NotificationActionSpec,
  type NotificationOperation,
  type NotificationParams,
  type NotificationScope,
  type ResultEntry,
} from "@/store/notifications";
import type { ActiveBook, DaemonBookScope } from "@/lib/activeBook";
import {
  useUiStore,
  type NotificationTone,
} from "@/store/ui";

import { notificationProgressValue } from "./notificationDisplay";
import { requestSettingsAnchor } from "./settings/settingsSearch";
import {
  projectConditionGroup,
  projectJob,
  projectResult,
  type ProjectedEntry,
} from "./notificationProjection";
import {
  resolveEntryAction,
  type ResolvedEntryAction,
} from "./notificationRouting";

type ChromeT = TFunction<"chrome">;
type OperationKind = NotificationOperation["kind"];

export interface NotificationCenterProps {
  /** Tint for the bell when something blocks or awaits review. */
  triggerClassName?: string;
  /** The book open now; null while unknown or while a switch is in flight. */
  currentBook: ActiveBook | null;
  /**
   * The open book at the moment of a click, read afresh: the list may have
   * been drawn before a switch started. Defaults to `currentBook`.
   */
  resolveCurrentBook?: () => ActiveBook | null;
  /**
   * Repeat an operation with its original options, sending `expectedScope`
   * so the daemon refuses it if another book is open by then. Called only
   * after the entry's book matched the open book at click time.
   */
  runOperation: (
    operation: NotificationOperation,
    expectedScope: DaemonBookScope,
  ) => void;
  /** Operations already running; their buttons are disabled. */
  pendingOperations?: Partial<Record<OperationKind, boolean>>;
  /**
   * Opening a running book refresh re-opens the full-screen sync card
   * instead of navigating. Absent when no book is open.
   */
  onOpenRunningBookRefresh?: () => void;
}

type Severity = NotificationTone;

const TONE_ICON: Record<
  Severity,
  React.ComponentType<React.SVGProps<SVGSVGElement>>
> = {
  error: CircleAlert,
  warning: AlertTriangle,
  success: CheckCircle2,
  info: Info,
};

const TONE_ICON_CLASS: Record<Severity, string> = {
  error: "text-red-700 dark:text-red-300",
  warning: "text-amber-700 dark:text-amber-300",
  success: "text-emerald-700 dark:text-emerald-300",
  info: "text-muted-foreground",
};

/** Re-renders on an interval while mounted, so "2 min. ago" keeps up. */
function useNow(intervalMs: number): number {
  const [now, setNow] = React.useState(() => Date.now());
  React.useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);
  return now;
}

function conditionActionLabel(
  code: ConditionCode,
  params: NotificationParams,
  t: ChromeT,
): string {
  switch (code) {
    case "journals.stale":
      return t("notifications.condition.journalsStale.action");
    case "review.pending":
      return t("notifications.condition.reviewPending.action");
    case "source.refresh-failed":
      return t("notifications.condition.sourceFailed.action");
    case "freshness.worker-error":
      return t("notifications.condition.workerError.action");
    case "book-refresh.failed":
      return params.forceFull
        ? t("notifications.condition.bookRefreshFailed.rescanAction")
        : t("notifications.condition.bookRefreshFailed.action");
    case "book-refresh.backend-required":
      return t("notifications.condition.backendRequired.action");
    case "journals.process-failed":
      return t("notifications.condition.journalsFailed.action");
    case "connection.sync-failed":
    case "btcpay.provenance-incomplete":
      return t("notifications.action.openWallet");
    case "touch-id.off":
      return t("notifications.condition.touchIdOff.action");
    case "unlock.credential-remains":
    case "backup.restore-warning":
      return t("notifications.action.openSecurity");
    case "backup.restore-failed":
      return t("notifications.condition.backupRestoreFailed.action");
    case "source-funds.rescan-needed":
      return t("notifications.action.openTransaction");
    case "pricing.rebuild-failed":
      return t("notifications.action.openMarketData");
    case "workspace.refresh-failed":
    case "workspace.books-need-attention":
      return t("notifications.action.openAllBooks");
    case "watch.evidence-changed":
      return t("notifications.condition.watchChanged.action");
  }
}

/** A result's button is named after where it leads. */
function resultActionLabel(resolved: ResolvedEntryAction, t: ChromeT): string {
  switch (resolved.kind) {
    case "open-connection":
      return t("notifications.action.openWallet");
    case "open-transaction":
      return t("notifications.action.openTransaction");
    case "open-workspace":
      return t("notifications.action.openAllBooks");
    case "operation":
      return t("notifications.condition.bookRefreshFailed.action");
    case "navigate":
      if (resolved.stale) return t("notifications.staleAction");
      if (resolved.to === "/journals") {
        return t("notifications.result.journalsProcessed.action");
      }
      if (resolved.to === "/connections") {
        return t("notifications.result.bookRefreshPending.action");
      }
      if (resolved.to === "/reports") {
        return t("notifications.result.reportExported.action");
      }
      if (resolved.to === "/settings/market") {
        return t("notifications.action.openMarketData");
      }
      return t("notifications.staleAction");
  }
}

/** A row's one button, resolved against the open book. */
interface RowAction {
  label: string;
  resolved: ResolvedEntryAction;
  pending: boolean;
}

function severityTone(severity: string): Severity {
  return severity === "error" || severity === "warning" ? severity : "info";
}

/**
 * The title-bar bell and its notification list. A popover, not a menu: each
 * entry carries several controls (action, details, dismiss), which a
 * `role="menu"` item cannot hold without fighting its keyboard model.
 *
 * Needs attention (blockers first), then Running, then Recent. The badge
 * counts unresolved attention groups for the open book; Clear recent never
 * touches conditions.
 */
export function NotificationCenter({
  triggerClassName,
  currentBook: currentActiveBook,
  resolveCurrentBook,
  runOperation,
  pendingOperations = {},
  onOpenRunningBookRefresh,
}: NotificationCenterProps) {
  const { t } = useTranslation("chrome");
  const navigate = useNavigate();
  const [open, setOpen] = React.useState(false);
  const conditions = useNotificationStore((s) => s.conditions);
  const jobs = useNotificationStore((s) => s.jobs);
  const results = useNotificationStore((s) => s.results);
  const dismissResult = useNotificationStore((s) => s.dismissResult);
  const clearResults = useNotificationStore((s) => s.clearResults);
  const markSeen = useNotificationStore((s) => s.markSeen);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const privacyMode = useUiStore((s) => s.privacyMode);
  const currentBook = currentActiveBook?.key ?? null;

  const groups = groupConditions(conditionsInScope(conditions, currentBook));
  const runningJobs = Object.values(jobs).sort((a, b) =>
    a.startedAt.localeCompare(b.startedAt),
  );
  // Newest first; the store already bounds Recent.
  const recent: ResultEntry[] = [...results]
    .sort((a, b) => b.lastOccurredAt.localeCompare(a.lastOccurredAt))
    .slice(0, MAX_RESULTS);

  const count = groups.length;
  const label =
    count > 0
      ? t("notifications.labelActive", { count })
      : t("notifications.label");

  const onOpenChange = (next: boolean) => {
    setOpen(next);
    if (next) markSeen();
  };

  // Resolve again at click time, against the book open *now*: the list may
  // have been rendered before a book switch.
  const runEntryAction = (
    action: NotificationActionSpec | undefined,
    scope: NotificationScope,
  ) => {
    const book = resolveCurrentBook ? resolveCurrentBook() : currentActiveBook;
    const resolved = resolveEntryAction(
      action,
      scope,
      book,
      useUiStore.getState().developerToolsEnabled,
    );
    if (!resolved) return;
    setOpen(false);
    switch (resolved.kind) {
      case "operation":
        runOperation(resolved.operation, resolved.expectedScope);
        return;
      case "open-connection":
        void navigate({
          to: "/connections/$connectionId",
          params: { connectionId: resolved.connectionId },
        });
        return;
      case "open-transaction":
        void navigate({
          to: "/transactions",
          search: { tx: resolved.transactionId },
        });
        return;
      case "open-workspace":
        void navigate({
          to: "/books/$workspaceId/birds-eye",
          params: { workspaceId: resolved.workspaceId },
        });
        return;
      case "navigate":
        if (resolved.anchor) requestSettingsAnchor(resolved.anchor);
        void navigate({ to: resolved.to });
        return;
    }
  };

  const pendingFor = (resolved: ResolvedEntryAction | null) =>
    resolved?.kind === "operation" &&
    Boolean(pendingOperations[resolved.operation.kind]);

  const empty = groups.length === 0 && runningJobs.length === 0 && recent.length === 0;

  return (
    <Popover open={open} onOpenChange={onOpenChange}>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn("relative", triggerClassName)}
          aria-label={label}
          title={label}
        >
          <Bell className="size-4" aria-hidden="true" />
          {count > 0 && (
            <span className="absolute -top-1 -right-1 flex size-4 items-center justify-center rounded-full bg-destructive text-3xs font-medium text-destructive-foreground">
              {count}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        className="flex max-h-[min(36rem,calc(100vh-4rem))] w-96 flex-col p-0"
        aria-label={t("notifications.label")}
      >
        <div className="flex items-center justify-between gap-2 px-3 pt-2 pb-1.5">
          <h2 className="text-sm font-medium">{t("notifications.label")}</h2>
          <Button
            type="button"
            variant="ghost"
            size="xs"
            disabled={recent.length === 0}
            onClick={clearResults}
          >
            {t("notifications.clearRecent")}
          </Button>
        </div>
        <Separator />
        {empty ? (
          <p className="px-3 py-4 text-center text-xs text-muted-foreground">
            {t("notifications.empty")}
          </p>
        ) : (
          <div className="min-h-0 flex-1 overflow-y-auto p-1">
            {groups.length > 0 ? (
              <Section title={t("notifications.sections.attention")} id="attention">
                {groups.map((group) => {
                  const first = group.entries[0];
                  const resolved = resolveEntryAction(
                    first.action,
                    first.scope,
                    currentActiveBook,
                    developerToolsEnabled,
                  );
                  return (
                    <EntryRow
                      key={group.key}
                      tone={severityTone(group.severity)}
                      projected={projectConditionGroup(group, privacyMode, t)}
                      occurredAt={latestOccurrence(group)}
                      repeatCount={Math.max(
                        ...group.entries.map((entry) => entry.occurrenceCount),
                      )}
                      action={
                        resolved
                          ? {
                              // An operation that cannot run here only
                              // opens its review page, and says so.
                              label:
                                resolved.kind === "navigate" && resolved.stale
                                  ? t("notifications.staleAction")
                                  : conditionActionLabel(group.code, first.params, t),
                              resolved,
                              pending: pendingFor(resolved),
                            }
                          : undefined
                      }
                      onAction={() => runEntryAction(first.action, first.scope)}
                      testId="notification-attention"
                    />
                  );
                })}
              </Section>
            ) : null}
            {runningJobs.length > 0 ? (
              <Section title={t("notifications.sections.running")} id="running">
                {runningJobs.map((job) => (
                  <JobRow
                    key={job.id}
                    job={job}
                    onOpen={
                      job.code === "book-refresh" && onOpenRunningBookRefresh
                        ? () => {
                            setOpen(false);
                            onOpenRunningBookRefresh();
                          }
                        : undefined
                    }
                  />
                ))}
              </Section>
            ) : null}
            {recent.length > 0 ? (
              <Section title={t("notifications.sections.recent")} id="recent">
                {recent.map((entry) => {
                  const resolved = resolveEntryAction(
                    entry.action,
                    entry.scope,
                    currentActiveBook,
                    developerToolsEnabled,
                  );
                  return (
                    <EntryRow
                      key={entry.id}
                      tone={
                        entry.severity === "info"
                          ? "success"
                          : severityTone(entry.severity)
                      }
                      projected={projectResult(entry, privacyMode, t)}
                      occurredAt={entry.lastOccurredAt}
                      // Workspace-wide results belong to no book.
                      otherBook={
                        entry.scope.book !== null &&
                        entry.scope.book !== currentBook
                      }
                      action={
                        resolved
                          ? {
                              label: resultActionLabel(resolved, t),
                              resolved,
                              pending: pendingFor(resolved),
                            }
                          : undefined
                      }
                      onAction={() => runEntryAction(entry.action, entry.scope)}
                      onDismiss={() => dismissResult(entry.id)}
                      testId="notification-entry"
                    />
                  );
                })}
              </Section>
            ) : null}
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

function latestOccurrence(group: ConditionGroup): string {
  return group.entries.reduce(
    (latest, entry) =>
      entry.lastOccurredAt > latest ? entry.lastOccurredAt : latest,
    group.entries[0].lastOccurredAt,
  );
}

function Section({
  title,
  id,
  children,
}: {
  title: string;
  id: string;
  children: React.ReactNode;
}) {
  const headingId = React.useId();
  return (
    <section aria-labelledby={headingId} data-section={id} className="pb-1">
      <h3
        id={headingId}
        className="px-2 pt-1.5 pb-0.5 text-2xs font-medium tracking-wide text-muted-foreground uppercase"
      >
        {title}
      </h3>
      <ul>{children}</ul>
    </section>
  );
}

function OccurredAt({ at }: { at: string }) {
  const { i18n } = useTranslation("chrome");
  const now = useNow(30_000);
  const relative = formatRelativeTime(at, now, i18n.language);
  const absolute = formatDateTime(at, i18n.language);
  if (!relative) return null;
  return (
    <time
      dateTime={at}
      title={absolute ?? undefined}
      className="block text-2xs text-muted-foreground"
    >
      {relative}
    </time>
  );
}

function RepeatBadge({ count }: { count: number }) {
  const { t } = useTranslation("chrome");
  if (count <= 1) return null;
  return (
    <Badge
      variant="secondary"
      className="shrink-0 px-1.5 py-0 text-3xs"
      title={t("notifications.repeated", { count })}
    >
      <span aria-hidden="true">{t("notifications.repeatBadge", { count })}</span>
      <span className="sr-only">{t("notifications.repeated", { count })}</span>
    </Badge>
  );
}

function DetailsToggle({
  expanded,
  controls,
  onToggle,
}: {
  expanded: boolean;
  controls: string;
  onToggle: () => void;
}) {
  const { t } = useTranslation("chrome");
  return (
    <Button
      type="button"
      variant="ghost"
      size="xs"
      className="text-muted-foreground"
      aria-expanded={expanded}
      aria-controls={controls}
      onClick={onToggle}
    >
      {expanded ? t("notifications.hideDetails") : t("notifications.showDetails")}
      <ChevronDown
        className={cn("transition-transform", expanded && "rotate-180")}
        aria-hidden="true"
      />
    </Button>
  );
}

function DetailList({ id, details }: { id: string; details: readonly string[] }) {
  const { t } = useTranslation("chrome");
  return (
    <ul
      id={id}
      className="mt-1 list-disc space-y-0.5 pl-4 text-xs break-words text-muted-foreground"
      aria-label={t("notifications.details")}
    >
      {details.map((line, index) => (
        <li key={index}>{line}</li>
      ))}
    </ul>
  );
}

function ActionButton({
  action,
  onClick,
}: {
  action: RowAction;
  onClick: () => void;
}) {
  const { t } = useTranslation("chrome");
  const operation =
    action.resolved.kind === "operation"
      ? action.resolved.operation.kind
      : null;
  const pendingLabel =
    operation === "process-journals"
      ? t("notifications.processing")
      : operation === "refresh-book"
        ? t("notifications.refreshing")
        : null;
  return (
    <Button
      type="button"
      variant="outline"
      size="xs"
      disabled={action.pending}
      onClick={onClick}
    >
      {action.pending && pendingLabel ? pendingLabel : action.label}
    </Button>
  );
}

function EntryRow({
  tone,
  projected,
  occurredAt,
  repeatCount = 1,
  otherBook = false,
  action,
  onAction,
  onDismiss,
  testId,
}: {
  tone: Severity;
  projected: ProjectedEntry;
  occurredAt: string;
  repeatCount?: number;
  otherBook?: boolean;
  action?: RowAction;
  onAction: () => void;
  onDismiss?: () => void;
  testId: string;
}) {
  const { t } = useTranslation("chrome");
  const [expanded, setExpanded] = React.useState(false);
  const detailsId = React.useId();
  const ToneIcon = TONE_ICON[tone] ?? Info;
  const hasControls = Boolean(action) || projected.details.length > 0;

  return (
    <li
      className="flex items-start gap-2 rounded-md px-2 py-2 hover:bg-accent/40"
      data-testid={testId}
    >
      <ToneIcon
        className={cn("mt-0.5 size-4 shrink-0", TONE_ICON_CLASS[tone])}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 text-sm font-medium break-words">
            {projected.title}
          </span>
          <RepeatBadge count={repeatCount} />
        </div>
        <OccurredAt at={occurredAt} />
        {otherBook ? (
          <p className="text-2xs text-muted-foreground">
            {t("notifications.otherBook")}
          </p>
        ) : null}
        {projected.body ? (
          <p className="mt-0.5 text-xs break-words whitespace-pre-line text-muted-foreground">
            {projected.body}
          </p>
        ) : null}
        {expanded && projected.details.length > 0 ? (
          <DetailList id={detailsId} details={projected.details} />
        ) : null}
        {hasControls ? (
          <div className="mt-1 flex flex-wrap items-center gap-1">
            {action ? <ActionButton action={action} onClick={onAction} /> : null}
            {projected.details.length > 0 ? (
              <DetailsToggle
                expanded={expanded}
                controls={detailsId}
                onToggle={() => setExpanded((value) => !value)}
              />
            ) : null}
          </div>
        ) : null}
      </div>
      {onDismiss ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-xs"
          className="shrink-0 text-muted-foreground"
          aria-label={t("notifications.dismiss", { title: projected.title })}
          title={t("notifications.dismissShort")}
          onClick={onDismiss}
        >
          <X aria-hidden="true" />
        </Button>
      ) : null}
    </li>
  );
}

function ProgressBar({
  value,
  indeterminate,
}: {
  value?: number;
  indeterminate?: boolean;
}) {
  return (
    <div className="h-1.5 overflow-hidden rounded-full bg-muted">
      <div
        className={cn(
          "h-full rounded-full bg-primary transition-[width] duration-300",
          indeterminate &&
            "w-1/2 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
        )}
        style={
          indeterminate
            ? undefined
            : { width: `${notificationProgressValue(value)}%` }
        }
      />
    </div>
  );
}

function JobRow({ job, onOpen }: { job: JobEntry; onOpen?: () => void }) {
  const { t } = useTranslation("chrome");
  const privacyMode = useUiStore((s) => s.privacyMode);
  const projected = projectJob(job, privacyMode, t);
  const phase = job.progress.phaseKey
    ? t(job.progress.phaseKey as never) /* dynamic key */
    : projected.label;
  return (
    <li
      className="flex items-start gap-2 rounded-md px-2 py-2 hover:bg-accent/40"
      data-testid="notification-running"
    >
      <Loader2
        className="mt-0.5 size-4 shrink-0 text-muted-foreground motion-safe:animate-spin"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        {onOpen ? (
          <button
            type="button"
            className="flex min-w-0 items-center gap-1 rounded-sm text-left text-sm font-medium hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            onClick={onOpen}
          >
            <span className="min-w-0 break-words">{projected.title}</span>
            <ChevronRight
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
          </button>
        ) : (
          <span className="min-w-0 text-sm font-medium break-words">
            {projected.title}
          </span>
        )}
        <div className="mt-1.5">
          <ProgressBar
            value={job.progress.value}
            indeterminate={job.progress.indeterminate}
          />
          {phase ? (
            <div className="mt-1 text-xs text-muted-foreground">{phase}</div>
          ) : null}
        </div>
      </div>
    </li>
  );
}
