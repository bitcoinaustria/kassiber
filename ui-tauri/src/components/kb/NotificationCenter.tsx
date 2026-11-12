import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  Bell,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Info,
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
  useUiStore,
  type AppNotification,
  type NotificationAction,
  type NotificationCommand,
  type NotificationTone,
} from "@/store/ui";

import {
  filterNotifications,
  notificationIsExpandable,
  notificationProgressValue,
  shouldHideNotificationProgressLabel,
  type NotificationFilter,
} from "./notificationDisplay";
import {
  notificationTarget,
  resolveNotificationAction,
  type NotificationTarget,
  type ResolvedNotificationAction,
} from "./notificationRouting";

/**
 * An entry derived from live book state rather than stored, such as the
 * review count. It cannot be dismissed (it would come straight back) and has
 * no timestamp.
 */
export interface SystemNotificationItem {
  id: string;
  title: string;
  body: string;
  tone: NotificationTone;
  to?: NotificationTarget;
  action?: NotificationAction;
}

type NotificationCenterItem = Omit<AppNotification, "createdAt" | "action"> & {
  createdAt?: string;
  to?: NotificationTarget;
  action?: ResolvedNotificationAction;
  system?: boolean;
};

export interface NotificationCenterProps {
  systemItems?: readonly SystemNotificationItem[];
  /** Tint for the bell when something blocks or awaits review. */
  triggerClassName?: string;
  runCommand: (command: NotificationCommand) => void;
  /** Commands already running; their buttons are disabled. */
  pendingCommands?: Partial<Record<NotificationCommand, boolean>>;
  /**
   * Opening a running book refresh re-opens the full-screen sync card
   * instead of navigating. Absent when no book is open.
   */
  onOpenRunningBookRefresh?: () => void;
}

const TONE_ICON: Record<
  NotificationTone,
  React.ComponentType<React.SVGProps<SVGSVGElement>>
> = {
  error: CircleAlert,
  warning: AlertTriangle,
  success: CheckCircle2,
  info: Info,
};

const TONE_ICON_CLASS: Record<NotificationTone, string> = {
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

/**
 * The title-bar bell and its notification list. A popover, not a menu: each
 * entry carries several controls (open, expand, action, dismiss), which a
 * `role="menu"` item cannot hold without fighting its keyboard model.
 */
export function NotificationCenter({
  systemItems = [],
  triggerClassName,
  runCommand,
  pendingCommands = {},
  onOpenRunningBookRefresh,
}: NotificationCenterProps) {
  const { t } = useTranslation("chrome");
  const navigate = useNavigate();
  const [open, setOpen] = React.useState(false);
  const [filter, setFilter] = React.useState<NotificationFilter>("all");
  const appNotifications = useUiStore((s) => s.notifications);
  const clearNotification = useUiStore((s) => s.clearNotification);
  const clearNotifications = useUiStore((s) => s.clearNotifications);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);

  const items: NotificationCenterItem[] = [
    ...appNotifications.map((item) => ({
      ...item,
      to: notificationTarget(
        item.title,
        item.tone,
        developerToolsEnabled,
        item.target,
      ),
      action:
        resolveNotificationAction(item.action, developerToolsEnabled) ??
        undefined,
    })),
    ...systemItems.map((item) => ({
      ...item,
      system: true,
      action:
        resolveNotificationAction(item.action, developerToolsEnabled) ??
        undefined,
    })),
  ];
  const needsActionItems = filterNotifications(items, "needsAction");
  const visibleItems = filter === "needsAction" ? needsActionItems : items;
  const count = items.filter(
    (item) => item.tone !== "info" || item.title.toLowerCase().includes("sync"),
  ).length;
  const label =
    count > 0
      ? t("notifications.labelActive", { count })
      : t("notifications.label");

  const openItem = (item: NotificationCenterItem) => {
    // A running book refresh minimized via "Continue in background" re-opens
    // the full-screen sync card rather than navigating.
    if (
      item.dedupeKey === "book-refresh" &&
      item.progress &&
      onOpenRunningBookRefresh
    ) {
      setOpen(false);
      onOpenRunningBookRefresh();
      return;
    }
    if (!item.to) return;
    setOpen(false);
    void navigate({ to: item.to });
  };

  const runAction = (action: ResolvedNotificationAction) => {
    setOpen(false);
    if (action.kind === "command") {
      runCommand(action.command);
      return;
    }
    void navigate({ to: action.to });
  };

  return (
    <Popover open={open} onOpenChange={setOpen}>
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
            disabled={appNotifications.length === 0}
            onClick={() => clearNotifications()}
          >
            {t("notifications.clearAll")}
          </Button>
        </div>
        <div
          role="group"
          aria-label={t("notifications.filter.label")}
          className="flex gap-1 px-3 pb-2"
        >
          {(["all", "needsAction"] as const).map((value) => (
            <Button
              key={value}
              type="button"
              variant={filter === value ? "secondary" : "ghost"}
              size="xs"
              aria-pressed={filter === value}
              onClick={() => setFilter(value)}
            >
              {value === "all"
                ? t("notifications.filter.all")
                : t("notifications.filter.needsAction", {
                    count: needsActionItems.length,
                  })}
            </Button>
          ))}
        </div>
        <Separator />
        {visibleItems.length === 0 ? (
          <p className="px-3 py-4 text-center text-xs text-muted-foreground">
            {filter === "needsAction"
              ? t("notifications.empty.needsAction")
              : t("notifications.empty.all")}
          </p>
        ) : (
          <ul className="min-h-0 flex-1 overflow-y-auto p-1">
            {visibleItems.map((item) => (
              <NotificationRow
                key={item.id}
                item={item}
                actionPending={
                  item.action?.kind === "command" &&
                  Boolean(pendingCommands[item.action.command])
                }
                onOpen={() => openItem(item)}
                onAction={runAction}
                onDismiss={
                  item.system ? undefined : () => clearNotification(item.id)
                }
              />
            ))}
          </ul>
        )}
      </PopoverContent>
    </Popover>
  );
}

function NotificationRow({
  item,
  actionPending,
  onOpen,
  onAction,
  onDismiss,
}: {
  item: NotificationCenterItem;
  actionPending: boolean;
  onOpen: () => void;
  onAction: (action: ResolvedNotificationAction) => void;
  onDismiss?: () => void;
}) {
  const { t, i18n } = useTranslation("chrome");
  const now = useNow(30_000);
  const [expanded, setExpanded] = React.useState(false);
  const bodyId = React.useId();
  const expandable = notificationIsExpandable(item);
  const details = (item.details ?? []).filter((line) => line.trim());
  const ToneIcon = TONE_ICON[item.tone] ?? Info;
  const relative = formatRelativeTime(item.createdAt, now, i18n.language);
  const absolute = formatDateTime(item.createdAt, i18n.language);
  const opensSomething =
    Boolean(item.to) || (item.dedupeKey === "book-refresh" && item.progress);
  const progressLabel =
    item.progress?.label &&
    !shouldHideNotificationProgressLabel(item.body, item.progress.label)
      ? item.progress.label
      : null;

  return (
    <li
      className="flex items-start gap-2 rounded-md px-2 py-2 hover:bg-accent/40"
      data-testid="notification-entry"
    >
      <ToneIcon
        className={cn("mt-0.5 size-4 shrink-0", TONE_ICON_CLASS[item.tone])}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-1.5">
          {opensSomething ? (
            <button
              type="button"
              className="flex min-w-0 items-center gap-1 rounded-sm text-left text-sm font-medium hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              onClick={onOpen}
            >
              <span className="min-w-0 break-words">{item.title}</span>
              <ChevronRight
                className="size-3.5 shrink-0 text-muted-foreground"
                aria-hidden="true"
              />
            </button>
          ) : (
            <span className="min-w-0 text-sm font-medium break-words">
              {item.title}
            </span>
          )}
          {item.repeatCount && item.repeatCount > 1 ? (
            <Badge
              variant="secondary"
              className="shrink-0 px-1.5 py-0 text-3xs"
              title={t("notifications.repeated", { count: item.repeatCount })}
            >
              <span aria-hidden="true">
                {t("notifications.repeatBadge", { count: item.repeatCount })}
              </span>
              <span className="sr-only">
                {t("notifications.repeated", { count: item.repeatCount })}
              </span>
            </Badge>
          ) : null}
        </div>
        {relative ? (
          <time
            dateTime={item.createdAt}
            title={absolute ?? undefined}
            className="block text-2xs text-muted-foreground"
          >
            {relative}
          </time>
        ) : null}
        <p
          id={bodyId}
          className={cn(
            "mt-0.5 text-xs break-words whitespace-pre-line text-muted-foreground",
            expandable && !expanded && "line-clamp-2",
          )}
        >
          {item.body}
        </p>
        {expanded && details.length > 0 ? (
          <ul
            className="mt-1 list-disc space-y-0.5 pl-4 text-xs break-words text-muted-foreground"
            aria-label={t("notifications.details")}
          >
            {details.map((line, index) => (
              <li key={index}>{line}</li>
            ))}
          </ul>
        ) : null}
        {item.progress ? (
          <div className="mt-1.5">
            <div className="h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className={cn(
                  "h-full rounded-full bg-primary transition-[width] duration-300",
                  item.progress.indeterminate &&
                    "w-1/2 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
                )}
                style={
                  item.progress.indeterminate
                    ? undefined
                    : {
                        width: `${notificationProgressValue(item.progress.value)}%`,
                      }
                }
              />
            </div>
            {progressLabel ? (
              <div className="mt-1 text-xs text-muted-foreground">
                {progressLabel}
              </div>
            ) : null}
          </div>
        ) : null}
        {expandable || item.action ? (
          <div className="mt-1 flex flex-wrap items-center gap-1">
            {item.action ? (
              <Button
                type="button"
                variant="outline"
                size="xs"
                disabled={actionPending}
                onClick={() => item.action && onAction(item.action)}
              >
                {actionPending &&
                item.action.kind === "command" &&
                item.action.command === "process-journals"
                  ? t("notifications.processing")
                  : item.action.label}
              </Button>
            ) : null}
            {expandable ? (
              <Button
                type="button"
                variant="ghost"
                size="xs"
                className="text-muted-foreground"
                aria-expanded={expanded}
                aria-controls={bodyId}
                onClick={() => setExpanded((value) => !value)}
              >
                {expanded
                  ? t("notifications.showLess")
                  : t("notifications.showMore")}
                <ChevronDown
                  className={cn("transition-transform", expanded && "rotate-180")}
                  aria-hidden="true"
                />
              </Button>
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
          aria-label={t("notifications.dismiss", { title: item.title })}
          title={t("notifications.dismissShort")}
          onClick={onDismiss}
        >
          <X aria-hidden="true" />
        </Button>
      ) : null}
    </li>
  );
}
