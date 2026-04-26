import { Dashboard5 } from "@/components/dashboard5";
import { useDaemon } from "@/daemon/client";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";

export function Overview() {
  const { data } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const snapshot =
    data?.kind === "ui.overview.snapshot" && data.data
      ? data.data
      : MOCK_OVERVIEW;

  return <Dashboard5 snapshot={snapshot} />;
}
