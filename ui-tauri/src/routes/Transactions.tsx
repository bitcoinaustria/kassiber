import { useRouterState } from "@tanstack/react-router";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { TransactionsDashboard } from "@/components/transactions/dashboard/TransactionsDashboard";
import {
  readTransactionDetailParams,
  readTransactionScopeParams,
} from "@/components/transactions/dashboard/model";
import { ScreenNotice, ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon, useDaemonInfinite } from "@/daemon/client";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";
import { MOCK_OVERVIEW } from "@/mocks/seed";
import { isDaemonDataMode, useUiStore } from "@/store/ui";

interface OverviewSnapshot {
  priceEur?: number | null;
}

const TRANSACTIONS_PAGE_LIMIT = 100;

function sameDaemonArgs(
  left: Record<string, unknown>,
  right: Record<string, unknown>,
) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function Transactions() {
  const { t } = useTranslation("transactions");
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonBacked = isDaemonDataMode(dataMode);
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
  // than the current table page would render an empty or truncated
  // table. The daemon `wallet` filter matches by wallet_id / label, so it
  // returns the wallet's complete history (including its transfers).
  //
  // This route state mirrors the canonical dashboard controller so query-key
  // changes refetch immediately while the router URL catches up.
  const [walletScope, setWalletScope] = React.useState<string | null>(
    scopeParams.wallet ?? null,
  );
  const [tableFilterArgs, setTableFilterArgs] = React.useState<
    Record<string, unknown>
  >({});
  // Same-route navigations that change only the search string — the sidebar
  // Transactions link, browser back/forward, or a different wallet deep link —
  // don't remount this route, so re-sync the query scope from the URL whenever
  // its `wallet` param changes (the mount-time seed alone would go stale). A
  // local controller changes also update it directly, so this effect is only
  // responsible for external/browser navigation.
  React.useEffect(() => {
    setWalletScope(scopeParams.wallet ?? null);
  }, [scopeParams.wallet]);
  const transactionArgs = React.useMemo(
    () => ({
      limit: TRANSACTIONS_PAGE_LIMIT,
      ...(walletScope ? { wallet: walletScope } : {}),
      ...tableFilterArgs,
    }),
    [tableFilterArgs, walletScope],
  );
  const updateTableFilterArgs = React.useCallback(
    (nextArgs: Record<string, unknown>) => {
      setTableFilterArgs((current) =>
        sameDaemonArgs(current, nextArgs) ? current : nextArgs,
      );
    },
    [],
  );
  const transactionsQuery = useDaemonInfinite<TransactionsList>(
    "ui.transactions.list",
    transactionArgs,
    (lastPage) => lastPage.data?.nextCursor ?? undefined,
  );
  const hasNextTransactionsPage = transactionsQuery.hasNextPage;
  const isFetchingNextTransactionsPage = transactionsQuery.isFetchingNextPage;
  const fetchNextTransactionsPage = transactionsQuery.fetchNextPage;
  const focusedTransaction = useDaemon<{
    transaction?: TransactionsList["txs"][number] | null;
  }>(
    "ui.transactions.resolve",
    { query: detailParams.transactionId ?? "" },
    { enabled: Boolean(detailParams.transactionId) },
  );
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const firstPage = transactionsQuery.data?.pages[0];
  const hasLiveTransactions =
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
  const loadMoreTransactions = React.useCallback(() => {
    if (!hasNextTransactionsPage || isFetchingNextTransactionsPage) return;
    void fetchNextTransactionsPage();
  }, [
    fetchNextTransactionsPage,
    hasNextTransactionsPage,
    isFetchingNextTransactionsPage,
  ]);
  const transactions: TransactionsList =
    liveTransactions
      ? liveTransactions
      : hasLiveTransactions && firstPage?.data
        ? firstPage.data
        : daemonBacked
          ? { ...MOCK_TRANSACTIONS, txs: [], nextCursor: null, hasMore: false }
          : MOCK_TRANSACTIONS;
  const tableTransactions: TransactionsList =
    liveTransactions ??
    (hasLiveTransactions ? firstPage?.data : null) ??
    (daemonBacked
      ? { ...MOCK_TRANSACTIONS, txs: [], nextCursor: null, hasMore: false }
      : transactions);
  const hasMoreTransactions = Boolean(hasNextTransactionsPage);
  const shouldShowLiveSkeleton =
    daemonBacked &&
    transactionsQuery.isLoading &&
    !hasLiveTransactions;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-44" />;
  }

  if (daemonBacked && !hasLiveTransactions) {
    return (
      <ScreenNotice
        title={t("route.unavailable.title")}
        body={
          transactionsQuery.error instanceof Error
            ? transactionsQuery.error.message
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
        : daemonBacked
          ? null
          : MOCK_OVERVIEW.priceEur;
  return (
    <TransactionsDashboard
      transactions={transactions}
      tableTransactions={tableTransactions}
      nowRate={nowRate}
      isDataRefreshing={
        hasLiveTransactions &&
        transactionsQuery.isFetching &&
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
      scopeParams={scopeParams}
      onWalletScopeChange={setWalletScope}
      onTableFilterArgsChange={updateTableFilterArgs}
    />
  );
}
