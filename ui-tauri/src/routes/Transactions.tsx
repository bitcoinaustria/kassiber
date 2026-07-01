import { useRouterState } from "@tanstack/react-router";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { TransactionsDashboard } from "@/components/transactions/dashboard/TransactionsDashboard";
import {
  candidateReferenceReviewType,
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
const TRANSACTIONS_WORKBENCH_PAGE_CAP = 4;
const TRANSACTIONS_HISTORY_BOUNDS_LIMIT = 1;

export function Transactions() {
  const { t } = useTranslation("transactions");
  const dataMode = useUiStore((state) => state.dataMode);
  const routeSearch = useRouterState({ select: (state) => state.location.search });
  const detailParams = React.useMemo(() => {
    void routeSearch;
    return readTransactionDetailParams();
  }, [routeSearch]);
  const scopeParams = React.useMemo(() => {
    void routeSearch;
    return readTransactionScopeParams();
  }, [routeSearch]);
  // When a wallet deep link is active (Wallet Detail "Show all" / related
  // links), scope the daemon queries to that wallet instead of filtering the
  // fetched page client-side. Otherwise a wallet whose transactions are older
  // than the global page/workbench limits would render an empty or truncated
  // table. The daemon `wallet` filter matches by wallet_id / label, so it
  // returns the wallet's complete history (including its transfers).
  //
  // This is plain React state (seeded from the deep-link param) so the
  // dropdown/clear mutate it directly and the queries refetch reliably.
  const [walletScope, setWalletScope] = React.useState<string | null>(
    scopeParams.wallet ?? null,
  );
  // Same-route navigations that change only the search string — the sidebar
  // Transactions link, browser back/forward, or a different wallet deep link —
  // don't remount this route, so re-sync the query scope from the URL whenever
  // its `wallet` param changes (the mount-time seed alone would go stale). A
  // dropdown pick changes `walletScope` without touching the URL, so this won't
  // clobber it (scopeParams.wallet is unchanged → effect doesn't fire).
  React.useEffect(() => {
    setWalletScope(scopeParams.wallet ?? null);
  }, [scopeParams.wallet]);
  const transactionArgs = React.useMemo(
    () => ({
      limit: TRANSACTIONS_PAGE_LIMIT,
      ...(walletScope ? { wallet: walletScope } : {}),
    }),
    [walletScope],
  );
  const transactionsQuery = useDaemonInfinite<TransactionsList>(
    "ui.transactions.list",
    transactionArgs,
    (lastPage) => lastPage.data?.nextCursor ?? undefined,
  );
  const hasNextTransactionsPage = transactionsQuery.hasNextPage;
  const isFetchingNextTransactionsPage = transactionsQuery.isFetchingNextPage;
  const fetchNextTransactionsPage = transactionsQuery.fetchNextPage;
  const workbenchQuery = useDaemonInfinite<TransactionsList>(
    "ui.transactions.list",
    {
      limit: TRANSACTIONS_WORKBENCH_LIMIT,
      ...(walletScope ? { wallet: walletScope } : {}),
    },
    (lastPage) => lastPage.data?.nextCursor ?? undefined,
  );
  const historyBoundsQuery = useDaemon<TransactionsList>("ui.transactions.list", {
    limit: TRANSACTIONS_HISTORY_BOUNDS_LIMIT,
    sort: "occurred-at",
    order: "asc",
    ...(walletScope ? { wallet: walletScope } : {}),
  });
  const workbenchPageCount = workbenchQuery.data?.pages.length ?? 0;
  React.useEffect(() => {
    if (
      workbenchQuery.hasNextPage &&
      !workbenchQuery.isFetchingNextPage &&
      workbenchPageCount < TRANSACTIONS_WORKBENCH_PAGE_CAP
    ) {
      void workbenchQuery.fetchNextPage();
    }
  }, [
    workbenchQuery.fetchNextPage,
    workbenchQuery.hasNextPage,
    workbenchQuery.isFetchingNextPage,
    workbenchPageCount,
  ]);
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
  const workbenchPages =
    workbenchQuery.data?.pages
      .map((page) => page.data)
      .filter((page): page is TransactionsList => Boolean(page)) ?? [];
  const workbenchData = React.useMemo<TransactionsList | null>(() => {
    if (!workbenchPages.length) return null;
    const latest = workbenchPages[workbenchPages.length - 1];
    return {
      ...latest,
      txs: workbenchPages.flatMap((page) => page.txs),
      hasMore: Boolean(latest.hasMore),
      nextCursor: latest.nextCursor ?? null,
    };
  }, [workbenchPages]);
  const historyBoundsData = historyBoundsQuery.data?.data ?? null;
  const hasLiveTransactions =
    Boolean(
      workbenchQuery.data?.pages.some(
        (page) => page.kind === "ui.transactions.list",
      ),
    ) &&
    Boolean(workbenchData);
  const hasLiveTableTransactions =
    firstPage?.kind === "ui.transactions.list" && Boolean(firstPage.data);
  const hasLiveHistoryBounds =
    historyBoundsQuery.data?.kind === "ui.transactions.list" &&
    Boolean(historyBoundsData);
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
  const loadMoreTransactions = React.useCallback(() => {
    if (!hasNextTransactionsPage || isFetchingNextTransactionsPage) return;
    void fetchNextTransactionsPage();
  }, [
    fetchNextTransactionsPage,
    hasNextTransactionsPage,
    isFetchingNextTransactionsPage,
  ]);
  const transactions: TransactionsList =
    hasLiveTransactions && workbenchData
      ? workbenchData
      : dataMode === "real"
        ? { ...MOCK_TRANSACTIONS, txs: [], nextCursor: null, hasMore: false }
        : MOCK_TRANSACTIONS;
  const tableTransactions: TransactionsList =
    liveTransactions ??
    (hasLiveTableTransactions ? firstPage?.data : null) ??
    (dataMode === "real"
      ? { ...MOCK_TRANSACTIONS, txs: [], nextCursor: null, hasMore: false }
      : transactions);
  const hasMoreTransactions = Boolean(hasNextTransactionsPage);
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
        title={t("route.unavailable.title")}
        body={
          workbenchQuery.error instanceof Error
            ? workbenchQuery.error.message
            : t("route.unavailable.body")
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
      ? swapQuery.data.data.candidates.filter(
          (candidate) => candidateReferenceReviewType(candidate) === "swap",
        ).length
      : hasLiveTransactions
        ? null
        : undefined;

  return (
    <TransactionsDashboard
      // Re-seed the dashboard's filter state (breakdownSelection / quickFilter)
      // when the URL scope changes on a same-route navigation. A dropdown pick
      // mutates client state without touching the URL, so the key is stable and
      // the dashboard is not remounted in that case.
      key={`${scopeParams.wallet ?? ""}::${scopeParams.quick ?? ""}`}
      transactions={transactions}
      tableTransactions={tableTransactions}
      historyBoundsTransactions={
        hasLiveHistoryBounds && historyBoundsData ? historyBoundsData : undefined
      }
      nowRate={nowRate}
      swapCandidates={swapCandidates}
      swapCandidateTotal={swapCandidateTotal}
      isDataRefreshing={
        hasLiveTransactions &&
        workbenchQuery.isFetching &&
        !isFetchingNextTransactionsPage
      }
      hasMoreTransactions={hasMoreTransactions}
      isLoadingMoreTransactions={isFetchingNextTransactionsPage}
      onLoadMoreTransactions={
        hasMoreTransactions
          ? loadMoreTransactions
          : undefined
      }
      focusedTransaction={focusedTransaction.data?.data?.transaction ?? null}
      deepLinkedTransactionId={detailParams.transactionId}
      deepLinkedTransactionTab={detailParams.tab}
      deepLinkedWallet={scopeParams.wallet}
      deepLinkedQuickFilter={scopeParams.quick}
      onWalletScopeChange={setWalletScope}
    />
  );
}
