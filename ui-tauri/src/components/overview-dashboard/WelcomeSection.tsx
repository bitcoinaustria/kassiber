import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import { buildOverviewReadiness, readinessToneStyles } from "./model";

export const WelcomeSection = ({
  onAddConnection,
  onProcessJournals,
  isProcessingJournals,
  snapshot,
}: {
  onAddConnection: () => void;
  onProcessJournals: () => void;
  isProcessingJournals: boolean;
  snapshot: OverviewSnapshot;
}) => {
  const { t } = useTranslation("overview");
  const readiness = buildOverviewReadiness(snapshot);
  const ReadinessIcon = readiness.icon;
  // dynamic key
  const readinessTitle = t(readiness.title.key as never, readiness.title.params);
  const needsJournals = Boolean(snapshot.status?.needsJournals);
  const hideReadinessPill = readiness.title.key === "readiness.connectSource.title";
  const readinessClassName = cn(
    "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
    readinessToneStyles[readiness.tone],
  );

  return (
    <div className={pageHeaderClassName}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {hideReadinessPill ? null : needsJournals ? (
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
            {isProcessingJournals
              ? t("welcome.reprocessingJournals")
              : readinessTitle}
          </button>
        ) : (
          <span className={readinessClassName}>
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readinessTitle}
          </span>
        )}
        <span className="min-w-0 truncate text-xs text-muted-foreground sm:text-sm">
          {/* dynamic key */}
          {t(readiness.detail.key as never, readiness.detail.params)}
        </span>
      </div>

      <div className={pageHeaderActionsClassName}>
        <Button
          size="sm"
          className={pageHeaderActionClassName}
          aria-label={t("welcome.addConnectionAria")}
          onClick={onAddConnection}
        >
          <Plus className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">{t("welcome.addConnection")}</span>
        </Button>
      </div>
    </div>
  );
};
