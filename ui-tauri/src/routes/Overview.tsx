import { OverviewDashboard } from "@/components/overview-dashboard/OverviewDashboard";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

export function Overview() {
  const dataMode = useUiStore((state) => state.dataMode);
  const { data, isLoading, isFetching, isError, error } =
    useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hasLiveOverview =
    data?.kind === "ui.overview.snapshot" && Boolean(data.data);
  const shouldUseMockOverview = dataMode !== "real" && !hasLiveOverview;
  const shouldShowLiveSkeleton =
    dataMode === "real" && (isLoading || isFetching) && !hasLiveOverview;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-32" />;
  }

  if (dataMode === "real" && !hasLiveOverview) {
    return (
      <ScreenNotice
        title="Overview unavailable"
        body={
          error instanceof Error
            ? error.message
            : data?.error?.message ??
              (isError
                ? "Kassiber could not read real overview data."
                : "No real overview data is available for the current book.")
        }
      />
    );
  }

  const snapshot =
    hasLiveOverview && data.data
      ? data.data
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
