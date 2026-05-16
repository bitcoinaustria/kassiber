import {
  Dashboard2,
  type SwapCandidateReference,
} from "@/components/dashboard2";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";
import { useUiStore } from "@/store/ui";

interface SuggestEnvelope {
  candidates: SwapCandidateReference[];
  counts?: {
    total?: number;
  };
}

export function Transactions() {
  const dataMode = useUiStore((state) => state.dataMode);
  const { data, isLoading } = useDaemon<TransactionsList>(
    "ui.transactions.list",
    {
      limit: 500,
    },
  );
  const swapQuery = useDaemon<SuggestEnvelope>("ui.transfers.suggest");
  const hasLiveTransactions =
    data?.kind === "ui.transactions.list" && Boolean(data.data);
  const shouldShowLiveSkeleton =
    dataMode === "real" && isLoading && !hasLiveTransactions;

  if (shouldShowLiveSkeleton) {
    return <ScreenSkeleton titleWidth="w-44" />;
  }

  const transactions =
    hasLiveTransactions && data.data
      ? data.data
      : MOCK_TRANSACTIONS;
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
      ? (swapQuery.data.data.counts?.total ?? swapQuery.data.data.candidates.length)
      : hasLiveTransactions
        ? null
        : undefined;

  return (
    <Dashboard2
      transactions={transactions}
      swapCandidates={swapCandidates}
      swapCandidateTotal={swapCandidateTotal}
    />
  );
}
