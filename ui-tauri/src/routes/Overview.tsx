import { Dashboard5 } from "@/components/dashboard5";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

export function Overview() {
  const dataMode = useUiStore((state) => state.dataMode);
  const { data, isLoading, isFetching } =
    useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hasLiveOverview =
    data?.kind === "ui.overview.snapshot" && Boolean(data.data);
  const shouldShowLiveSkeleton =
    dataMode === "real" && isLoading && !hasLiveOverview;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-32" />;
  }

  const snapshot =
    hasLiveOverview && data.data
      ? data.data
      : MOCK_OVERVIEW;

  return (
    <Dashboard5
      snapshot={snapshot}
      isSnapshotRefreshing={hasLiveOverview && isFetching}
    />
  );
}
