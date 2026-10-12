// @vitest-environment happy-dom
//
// The Transactions page hands the assistant its visible counts only while the
// dashboard snapshot belongs to the current query. Placeholder data kept from
// the previous query (for example the previous book after a switch) must not
// go out with a new book's turn.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => async () => {},
}));

import { useAssistantScreenMetricsStore } from "@/store/assistantScreenMetrics";
import type { TransactionDashboardSnapshot } from "./model";
import { TransactionWorkbench } from "./TransactionWorkbench";

const stats = (count: number) => ({
  count,
  btc: 0,
  eur: 0,
  missingPrice: 0,
  review: 0,
  failed: 0,
});

function snapshot(
  profileId: string,
  pending: number,
): TransactionDashboardSnapshot {
  return {
    period: "1year",
    window: { since: null, until: null },
    metricsScope: {
      workspaceId: "ws-1",
      profileId,
      inputVersion: 3,
      wallet: null,
    },
    history: { count: 0, earliest: null, latest: null, walletOptions: [] },
    counts: { all: 0, external: 0 },
    summary: {
      incoming: stats(2),
      outgoing: stats(1),
      transfers: stats(0),
      swaps: stats(0),
      review: 0,
      pending,
      failed: 0,
    },
    buckets: [],
    candidates: [],
    swapCandidateTotal: 0,
  };
}

function workbench(
  dashboardSnapshot: TransactionDashboardSnapshot,
  dashboardSnapshotIsCurrent: boolean,
) {
  return (
    <TransactionWorkbench
      period="1year"
      records={[]}
      hideSensitive={false}
      currency="btc"
      chartSelection={null}
      onFlowSelectionChange={() => {}}
      onQuickFilterChange={() => {}}
      onBreakdownSelectionChange={() => {}}
      onTableFiltersReset={() => {}}
      dashboardSnapshot={dashboardSnapshot}
      dashboardSnapshotIsCurrent={dashboardSnapshotIsCurrent}
    />
  );
}

describe("Transactions page counts for the assistant", () => {
  beforeEach(() => {
    useAssistantScreenMetricsStore.setState({ snapshot: null });
  });
  afterEach(cleanup);

  it("publishes the current snapshot with the book state it was counted on", () => {
    render(workbench(snapshot("book-a", 11), true));
    expect(useAssistantScreenMetricsStore.getState().snapshot).toMatchObject({
      route: "/transactions",
      period: "1year",
      scope: { profileId: "book-a", inputVersion: 3, wallet: null },
      metrics: { review_queue_pending: 11, incoming_count: 2 },
    });
  });

  it("withdraws the counts while the previous book's snapshot is a placeholder", () => {
    const view = render(workbench(snapshot("book-a", 11), true));
    expect(useAssistantScreenMetricsStore.getState().snapshot).not.toBeNull();

    // Book switch: the query key changes and the old snapshot stays on screen
    // as placeholder data until the new book's dashboard resolves.
    act(() => view.rerender(workbench(snapshot("book-a", 11), false)));
    expect(useAssistantScreenMetricsStore.getState().snapshot).toBeNull();

    act(() => view.rerender(workbench(snapshot("book-b", 2), true)));
    expect(useAssistantScreenMetricsStore.getState().snapshot).toMatchObject({
      scope: { profileId: "book-b" },
      metrics: { review_queue_pending: 2 },
    });
  });

  it("never publishes placeholder data on first render", () => {
    render(workbench(snapshot("book-a", 11), false));
    expect(useAssistantScreenMetricsStore.getState().snapshot).toBeNull();
  });
});
