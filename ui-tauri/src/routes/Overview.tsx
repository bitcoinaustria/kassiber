import { useTranslation } from "react-i18next";

import { OverviewDashboard } from "@/components/overview-dashboard/OverviewDashboard";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { normalizeOverviewSnapshot } from "@/lib/normalizeUiSnapshots";
import type { OverviewSnapshot } from "@/mocks/seed";

export function Overview() {
  const { t } = useTranslation("overview");
  const { data, isLoading, isFetching, isError, error } =
    useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hasLiveOverview =
    data?.kind === "ui.overview.snapshot" && Boolean(data.data);
  const shouldShowLiveSkeleton =
    (isLoading || isFetching) && !hasLiveOverview;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-32" />;
  }

  if (!hasLiveOverview) {
    return (
      <ScreenNotice
        title={t("screen.unavailableTitle")}
        body={
          error instanceof Error
            ? error.message
            : data?.error?.message ??
              (isError
                ? t("screen.unavailableBody")
                : t("screen.noRealData"))
        }
      />
    );
  }

  const snapshot = normalizeOverviewSnapshot(data.data!);

  return (
    <OverviewDashboard
      snapshot={snapshot}
      isSnapshotRefreshing={isFetching}
    />
  );
}
