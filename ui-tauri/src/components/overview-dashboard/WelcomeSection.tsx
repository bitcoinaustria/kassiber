import { Link } from "@tanstack/react-router";
import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { reviewTotal, type ReviewBadges } from "@/lib/reviewCount";
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
  // The same number as the sidebar's Review badge, from the same read.
  const badges = useDaemon<ReviewBadges>("ui.review.badges").data?.data;
  const readiness = buildOverviewReadiness(snapshot, badges ? reviewTotal(badges) : undefined);
  const ReadinessIcon = readiness.icon;
  // dynamic key
  const readinessTitle = t(readiness.title.key as never, readiness.title.params);
  // Stale journals are fixed in place: the pill reprocesses them. Branch on
  // what the pill says, so a source error ahead of them still opens Wallets.
  const reprocesses = readiness.title.key === "readiness.reprocessJournals.title";
  const hideReadinessPill = readiness.title.key === "readiness.connectSource.title";
  const readinessClassName = cn(
    "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium",
    readinessToneStyles[readiness.tone],
  );

  return (
    <div className={pageHeaderClassName}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {hideReadinessPill ? null : reprocesses ? (
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
        ) : readiness.href ? (
          // The one place the overview names what holds the book back, so it
          // opens where that is dealt with.
          <Link
            to={readiness.href}
            className={cn(
              readinessClassName,
              "transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            )}
          >
            <ReadinessIcon className="size-4" aria-hidden="true" />
            {readinessTitle}
          </Link>
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
