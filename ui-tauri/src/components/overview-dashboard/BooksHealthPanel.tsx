import { Link } from "@tanstack/react-router";
import { CheckCircle2, FileText } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  buildOverviewHealthItems,
  buildPrimaryOverviewAction,
  type OverviewHealthTone,
} from "./model";

const healthToneStyles: Record<OverviewHealthTone, string> = {
  good: "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/25 dark:text-emerald-300 dark:ring-emerald-400/20",
  warning:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/25 dark:text-amber-300 dark:ring-amber-400/20",
  alert:
    "bg-red-50 text-red-700 ring-red-600/15 dark:bg-red-900/25 dark:text-red-300 dark:ring-red-400/20",
  neutral:
    "bg-zinc-50 text-zinc-700 ring-zinc-500/20 dark:bg-zinc-800/70 dark:text-zinc-300 dark:ring-zinc-400/20",
};

export const BooksHealthPanel = ({
  className,
  snapshot,
  onProcessJournals,
  isProcessingJournals,
}: {
  className?: string;
  snapshot: OverviewSnapshot;
  onProcessJournals: () => void;
  isProcessingJournals: boolean;
}) => {
  const healthItems = buildOverviewHealthItems(snapshot);
  const primaryAction = buildPrimaryOverviewAction(snapshot);
  const PrimaryIcon = primaryAction?.icon;
  const needsJournals = Boolean(snapshot.status?.needsJournals);

  return (
    <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-3 pt-3 sm:px-4">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0 sm:size-8"
            aria-label="Books health"
          >
            <CheckCircle2 className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium">
              Books Health
            </span>
            <p className="text-[10px] text-muted-foreground sm:text-xs">
              What needs attention before reports
            </p>
          </div>
        </div>
      </div>

      <div className="space-y-2.5 px-3 pt-2.5 pb-3 sm:px-4">
        {primaryAction && PrimaryIcon ? (
          <Link
            to={primaryAction.href}
            className={cn(
              "group flex items-start gap-3 rounded-lg p-2.5 ring-1 ring-inset transition-colors hover:bg-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              healthToneStyles[primaryAction.tone],
            )}
          >
            <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-background/70">
              <PrimaryIcon className="size-4" aria-hidden="true" />
            </span>
            <span className="min-w-0">
              <span className="block text-sm font-semibold">
                {primaryAction.title}
              </span>
              <span className="mt-0.5 block text-xs leading-5 opacity-80">
                {primaryAction.detail}
              </span>
            </span>
          </Link>
        ) : null}

        <div className="divide-y rounded-lg border bg-background/50">
          {healthItems.map((item) => {
            const ItemIcon = item.icon;
            const isJournalRefresh = item.key === "journals" && needsJournals;
            const content = (
              <>
                <span
                  className={cn(
                    "flex size-8 shrink-0 items-center justify-center rounded-md ring-1 ring-inset",
                    healthToneStyles[item.tone],
                  )}
                >
                  <ItemIcon
                    className={cn(
                      "size-4",
                      isJournalRefresh && isProcessingJournals && "animate-spin",
                    )}
                    aria-hidden="true"
                  />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-xs font-medium text-muted-foreground">
                    {item.title}
                  </span>
                  <span className="mt-0.5 block truncate text-sm font-semibold text-foreground">
                    {isJournalRefresh && isProcessingJournals
                      ? "Reprocessing"
                      : item.value}
                  </span>
                </span>
                <span className="hidden max-w-[140px] text-right text-[10px] leading-4 text-muted-foreground sm:block">
                  {item.detail}
                </span>
              </>
            );
            const className =
              "group flex w-full items-center gap-3 px-3 py-2.5 text-left transition-colors first:rounded-t-lg last:rounded-b-lg hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

            return isJournalRefresh ? (
              <button
                key={item.key}
                type="button"
                className={className}
                onClick={onProcessJournals}
                disabled={isProcessingJournals}
              >
                {content}
              </button>
            ) : (
              <Link
                key={item.key}
                to={item.href}
                className={className}
              >
                {content}
              </Link>
            );
          })}
        </div>

        <Button asChild variant="ghost" size="sm" className="h-8 w-full">
          <Link to="/reports">
            <FileText className="size-4" aria-hidden="true" />
            Reports
          </Link>
        </Button>
      </div>
    </div>
  );
};
