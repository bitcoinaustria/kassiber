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
import type { OverviewSnapshot } from "@/mocks/seed";

import { ReadinessPill } from "@/components/kb/ReadinessPill";
import { useBlockingSourceCount } from "@/lib/reportBlockers";

import { buildOverviewReadiness } from "./model";

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
  const blockingSources = useBlockingSourceCount();
  const readiness = buildOverviewReadiness(
    snapshot,
    badges ? reviewTotal(badges) : undefined,
    blockingSources,
  );
  // dynamic key
  const readinessTitle = t(readiness.title.key as never, readiness.title.params);
  // Stale journals are fixed in place: the pill reprocesses them. Branch on
  // what the pill says, so a source error ahead of them still opens Wallets.
  const reprocesses = readiness.title.key === "readiness.reprocessJournals.title";
  const hideReadinessPill = readiness.title.key === "readiness.connectSource.title";

  return (
    <div className={pageHeaderClassName}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
        {hideReadinessPill ? null : (
          <ReadinessPill
            tone={readiness.tone}
            icon={readiness.icon}
            title={readinessTitle}
            busy={reprocesses && isProcessingJournals}
            busyTitle={t("welcome.reprocessingJournals")}
            onClick={reprocesses ? onProcessJournals : undefined}
            // The one place the overview names what holds the book back, so
            // it opens where that is dealt with.
            href={reprocesses ? undefined : readiness.href}
          />
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
