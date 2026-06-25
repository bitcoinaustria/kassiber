/**
 * Sync progress card.
 *
 * Shown while a book's sync runs — both the initial first sync and later
 * incremental refreshes (`isFirstSync` only switches the headline/body copy). It
 * floats centered over the content area using the same frosted dock language as
 * the shell assistant, so a sync has a clear home instead of a thin top bar that
 * shoves content around.
 *
 * It is a non-blocking overlay: the container is `pointer-events-none` so the
 * rest of the shell stays clickable, and "Continue in background" demotes it to
 * the top progress line (see `RouteTopProgressLine` in AppShell), from where the
 * book-refresh notification can re-open it.
 */
import { Check, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { RouteProgressState } from "@/components/kb/progressIndicator";
import { Button } from "@/components/ui/button";
import {
  FIRST_SYNC_MILESTONES,
  firstSyncActiveMilestoneIndex,
} from "@/lib/syncProgress";
import { cn } from "@/lib/utils";

interface FirstSyncCardProps {
  progress: RouteProgressState | null;
  title?: string;
  /** First sync vs a later incremental refresh — only changes the copy. */
  isFirstSync?: boolean;
  onDismiss: () => void;
}

function clampPercent(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

export function FirstSyncCard({
  progress,
  title,
  isFirstSync = true,
  onDismiss,
}: FirstSyncCardProps) {
  const { t } = useTranslation("chrome");
  const rawValue = progress?.value;
  const hasValue = typeof rawValue === "number" && Number.isFinite(rawValue);
  const isDeterminate = hasValue && !progress?.indeterminate;
  const value =
    typeof rawValue === "number" && Number.isFinite(rawValue)
      ? clampPercent(rawValue)
      : 0;
  const fraction = value / 100;

  // The milestone the bar currently sits in; everything before reads as done,
  // everything after as pending. (See firstSyncActiveMilestoneIndex.)
  const activeIndex = firstSyncActiveMilestoneIndex(fraction, isDeterminate);

  return (
    <div className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center px-4 pb-28">
      {/* Subtle scrim over the content area to focus the card. It captures
          pointer events (pointer-events-auto) so background hover/animations go
          inert while the card is open — only the modal is in focus. Scoped to
          this content region, so the sidebar/top bar stay usable; "Continue in
          background" collapses the card to resume working in this area. */}
      <div
        aria-hidden="true"
        className="pointer-events-auto absolute inset-0 bg-background/30 backdrop-blur-sm"
      />
      <div
        className="relative pointer-events-auto w-full max-w-md rounded-[28px] border border-white/70 bg-muted/85 p-5 shadow-[0_24px_90px_rgba(15,23,42,0.26),0_3px_18px_rgba(15,23,42,0.12),inset_0_1px_0_rgba(255,255,255,0.80)] ring-1 ring-zinc-950/10 backdrop-blur-2xl backdrop-saturate-150 dark:border-border dark:bg-card dark:shadow-[0_18px_48px_rgba(0,0,0,0.28)] dark:ring-border/70 dark:backdrop-blur-none dark:backdrop-saturate-100"
      >
        <div className="flex items-start gap-3">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-primary/10 text-primary">
            <Loader2
              className="h-5 w-5 animate-spin motion-reduce:animate-none"
              aria-hidden="true"
            />
          </span>
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold text-foreground">
              {title ??
                (isFirstSync
                  ? t("firstSync.defaultTitle")
                  : t("firstSync.defaultTitleRefresh"))}
            </h2>
            <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
              {isFirstSync ? t("firstSync.body") : t("firstSync.bodyRefresh")}
            </p>
          </div>
        </div>

        <div className="mt-4">
          {/* Only the concise status line + percentage is a live region, so
              assistive tech announces "<label> — <pct>%" on each tick rather
              than re-reading the whole card (header + milestone list). */}
          <div
            role="status"
            aria-live="polite"
            className="mb-1.5 flex items-center justify-between gap-3 text-[11px] font-medium leading-none"
          >
            <span className="min-w-0 truncate text-primary">
              {progress?.label ?? t("firstSync.preparing")}
            </span>
            <span className="shrink-0 tabular-nums text-muted-foreground">
              {isDeterminate ? `${Math.round(value)}%` : t("firstSync.working")}
            </span>
          </div>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-primary/15">
            <div
              className={cn(
                "h-full rounded-full bg-primary/80",
                isDeterminate
                  ? "transition-[width] duration-300 ease-out"
                  : "w-1/3 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
              )}
              style={isDeterminate ? { width: `${value}%` } : undefined}
            />
          </div>
        </div>

        <ul className="mt-4 space-y-2">
          {FIRST_SYNC_MILESTONES.map((milestone, index) => {
            const done = isDeterminate && index < activeIndex;
            const active = index === activeIndex;
            return (
              <li
                key={milestone.phase}
                className="flex items-center gap-2.5 text-xs"
              >
                <span
                  className={cn(
                    "flex h-4 w-4 shrink-0 items-center justify-center rounded-full",
                    done
                      ? "bg-primary/80 text-background"
                      : active
                        ? "text-primary"
                        : "text-muted-foreground/40",
                  )}
                  aria-hidden="true"
                >
                  {done ? (
                    <Check className="h-3 w-3" />
                  ) : active ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                  ) : (
                    <span className="h-1.5 w-1.5 rounded-full bg-current" />
                  )}
                </span>
                <span
                  className={cn(
                    "min-w-0 truncate",
                    done
                      ? "text-muted-foreground line-through decoration-muted-foreground/40"
                      : active
                        ? "font-medium text-foreground"
                        : "text-muted-foreground/70",
                  )}
                >
                  {milestone.label}
                </span>
              </li>
            );
          })}
        </ul>

        <div className="mt-5 flex flex-col items-center gap-2.5 border-t border-border/60 pt-4">
          <p className="text-center text-[11px] leading-snug text-muted-foreground">
            {t("firstSync.keepUsing")}
          </p>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="rounded-full text-xs"
            onClick={onDismiss}
          >
            {t("firstSync.continueInBackground")}
          </Button>
        </div>
      </div>
    </div>
  );
}
