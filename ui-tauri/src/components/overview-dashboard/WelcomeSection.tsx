import { Plus, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import { buildOverviewReadiness, readinessToneStyles } from "./shared";

export const WelcomeSection = ({
  onAddConnection,
  onRefresh,
  onProcessJournals,
  isRefreshing,
  isProcessingJournals,
  snapshot,
}: {
  onAddConnection: () => void;
  onRefresh: () => void;
  onProcessJournals: () => void;
  isRefreshing: boolean;
  isProcessingJournals: boolean;
  snapshot: OverviewSnapshot;
}) => {
  const readiness = buildOverviewReadiness(snapshot);
  const ReadinessIcon = readiness.icon;
  const needsJournals = Boolean(snapshot.status?.needsJournals);
  const readinessClassName = cn(
    "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
    readinessToneStyles[readiness.tone],
  );

  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:items-center sm:justify-between sm:gap-3">
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {needsJournals ? (
          <button
            type="button"
            className={cn(
              readinessClassName,
              "transition-colors hover:bg-amber-500/15 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-60",
            )}
            onClick={onProcessJournals}
            disabled={isProcessingJournals}
          >
            <ReadinessIcon
              className={cn("size-4", isProcessingJournals && "animate-spin")}
              aria-hidden="true"
            />
            {isProcessingJournals ? "Reprocessing journals" : readiness.title}
          </button>
        ) : (
          <span className={readinessClassName}>
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readiness.title}
          </span>
        )}
        <span className="min-w-0 truncate text-xs text-muted-foreground sm:text-sm">
          {readiness.detail}
        </span>
      </div>

      <div className="flex flex-wrap items-center gap-2 sm:gap-3">
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-2"
          aria-label="Refresh wallets and journals"
          onClick={onRefresh}
          disabled={isRefreshing}
        >
          <RefreshCw
            className={cn("size-4", isRefreshing && "animate-spin")}
            aria-hidden="true"
          />
          <span className="hidden sm:inline">
            {isRefreshing ? "Refreshing" : "Refresh"}
          </span>
        </Button>
        <Button
          size="sm"
          className="h-8 gap-2"
          aria-label="Add connection"
          onClick={onAddConnection}
        >
          <Plus className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">Add connection</span>
        </Button>
      </div>
    </div>
  );
};
