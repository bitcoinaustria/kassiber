import { useTranslation } from "react-i18next";

import { OverviewDashboard } from "@/components/overview-dashboard/OverviewDashboard";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { normalizeOverviewSnapshot } from "@/lib/normalizeUiSnapshots";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { isDaemonDataMode, useUiStore } from "@/store/ui";

export function Overview() {
  const { t } = useTranslation("overview");
  const dataMode = useUiStore((state) => state.dataMode);
  const { data, isLoading, isFetching, isError, error } =
    useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hasLiveOverview =
    data?.kind === "ui.overview.snapshot" && Boolean(data.data);
  const daemonBacked = isDaemonDataMode(dataMode);
  const shouldUseMockOverview = !daemonBacked && !hasLiveOverview;
  const shouldShowLiveSkeleton =
    daemonBacked && (isLoading || isFetching) && !hasLiveOverview;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-32" />;
  }

  if (daemonBacked && !hasLiveOverview) {
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

  const snapshot =
    hasLiveOverview && data.data
      ? normalizeOverviewSnapshot(data.data)
      : shouldUseMockOverview
        ? MOCK_OVERVIEW
        : data?.data;

  return (
    <OverviewDashboard
      snapshot={snapshot ?? MOCK_OVERVIEW}
      isSnapshotRefreshing={hasLiveOverview && isFetching}
    />
  );
}
