import { useRouterState } from "@tanstack/react-router";
import * as React from "react";

import { TransactionsDashboard } from "@/components/transactions/dashboard/TransactionsDashboard";
import {
  readTransactionDetailParams,
  readTransactionScopeParams,
  type SwapCandidateReference,
} from "@/components/transactions/dashboard/model";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon, useDaemonInfinite } from "@/daemon/client";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";
import { MOCK_OVERVIEW } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

interface SuggestEnvelope {
  candidates: SwapCandidateReference[];
  counts?: {
    total?: number;
  };
}

interface OverviewSnapshot {
  priceEur?: number | null;
}

const TRANSACTIONS_PAGE_LIMIT = 100;
const TRANSACTIONS_WORKBENCH_LIMIT = 500;

function isCrossAssetCandidate(candidate: SwapCandidateReference) {
  if (!candidate.in_asset || !candidate.out_asset) return true;
  return candidate.in_asset.toUpperCase() !== candidate.out_asset.toUpperCase();
}

export function Transactions() {
  const dataMode = useUiStore((state) => state.dataMode);
  const routeSearch = useRouterState({ select: (state) => state.location.search });
  const detailParams = React.useMemo(
    () => readTransactionDetailParams(),
    [routeSearch],
  );
  const scopeParams = React.useMemo(
    () => readTransactionScopeParams(),
    [routeSearch],
  );
  const transactionArgs = React.useMemo(
    () => ({
      limit: TRANSACTIONS_PAGE_LIMIT,
    }),
    [],
  );
  const transactionsQuery = useDaemonInfinite<TransactionsList>(
    "ui.transactions.list",
    transactionArgs,
    (lastPage) => lastPage.data?.nextCursor ?? undefined,
  );
  const workbenchQuery = useDaemon<TransactionsList>("ui.transactions.list", {
    limit: TRANSACTIONS_WORKBENCH_LIMIT,
  });
  const focusedTransaction = useDaemon<{
    transaction?: TransactionsList["txs"][number] | null;
  }>(
    "ui.transactions.resolve",
    { query: detailParams.transactionId ?? "" },
    { enabled: Boolean(detailParams.transactionId) },
  );
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const swapQuery = useDaemon<SuggestEnvelope>("ui.transfers.suggest");
  const firstPage = transactionsQuery.data?.pages[0];
  const workbenchData = workbenchQuery.data?.data ?? null;
  const hasLiveTransactions =
    workbenchQuery.data?.kind === "ui.transactions.list" && Boolean(workbenchData);
  const hasLiveTableTransactions =
    firstPage?.kind === "ui.transactions.list" && Boolean(firstPage.data);
  const liveTransactions = React.useMemo<TransactionsList | null>(() => {
    const pages =
      transactionsQuery.data?.pages
        .map((page) => page.data)
        .filter((page): page is TransactionsList => Boolean(page)) ?? [];
    if (pages.length === 0) return null;
    const latest = pages[pages.length - 1];
    return {
      ...latest,
      txs: pages.flatMap((page) => page.txs),
    };
  }, [transactionsQuery.data]);
  const transactions: TransactionsList =
    hasLiveTransactions && workbenchData
      ? workbenchData
      : dataMode === "real"
        ? { ...MOCK_TRANSACTIONS, txs: [], nextCursor: null, hasMore: false }
        : MOCK_TRANSACTIONS;
  const tableTransactions: TransactionsList =
    liveTransactions ??
    (hasLiveTableTransactions ? firstPage?.data : null) ??
    transactions;
  const hasMoreTransactions = Boolean(transactionsQuery.hasNextPage);
  const shouldShowLiveSkeleton =
    dataMode === "real" &&
    workbenchQuery.isLoading &&
    !hasLiveTransactions;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-44" />;
  }

  if (dataMode === "real" && !hasLiveTransactions) {
    return (
      <ScreenNotice
        title="Transactions unavailable"
        body={
          workbenchQuery.error instanceof Error
            ? workbenchQuery.error.message
            : workbenchQuery.data?.error?.message ??
              "Kassiber could not read real transactions for the current book."
        }
      />
    );
  }

  const hasLiveOverview =
    overview.data?.kind === "ui.overview.snapshot" && Boolean(overview.data.data);
  const nowRate =
    hasLiveTransactions && hasLiveOverview
      ? (overview.data?.data?.priceEur ?? null)
      : hasLiveTransactions
        ? null
        : dataMode === "real"
          ? null
          : MOCK_OVERVIEW.priceEur;
  const hasLiveSwapSuggestions =
    swapQuery.data?.kind === "ui.transfers.suggest" &&
    Boolean(swapQuery.data.data);
  const swapCandidates =
    hasLiveSwapSuggestions && swapQuery.data?.data
      ? swapQuery.data.data.candidates
      : hasLiveTransactions
        ? []
      : undefined;
  const swapCandidateTotal =
    hasLiveSwapSuggestions && swapQuery.data?.data
      ? swapQuery.data.data.candidates.filter(isCrossAssetCandidate).length
      : hasLiveTransactions
        ? null
        : undefined;

  return (
    <TransactionsDashboard
      transactions={transactions}
      tableTransactions={tableTransactions}
      nowRate={nowRate}
      swapCandidates={swapCandidates}
      swapCandidateTotal={swapCandidateTotal}
      isDataRefreshing={
        hasLiveTransactions &&
        workbenchQuery.isFetching &&
        !transactionsQuery.isFetchingNextPage
      }
      hasMoreTransactions={hasMoreTransactions}
      isLoadingMoreTransactions={transactionsQuery.isFetchingNextPage}
      onLoadMoreTransactions={
        hasMoreTransactions
          ? () => void transactionsQuery.fetchNextPage()
          : undefined
      }
      focusedTransaction={focusedTransaction.data?.data?.transaction ?? null}
      deepLinkedTransactionId={detailParams.transactionId}
      deepLinkedTransactionTab={detailParams.tab}
      deepLinkedWallet={scopeParams.wallet}
      deepLinkedQuickFilter={scopeParams.quick}
    />
  );
}
