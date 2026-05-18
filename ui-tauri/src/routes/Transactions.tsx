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

function isCrossAssetCandidate(candidate: SwapCandidateReference) {
  if (!candidate.in_asset || !candidate.out_asset) return true;
  return candidate.in_asset.toUpperCase() !== candidate.out_asset.toUpperCase();
}

export function Transactions() {
  const dataMode = useUiStore((state) => state.dataMode);
  const { data, isLoading, isFetching } = useDaemon<TransactionsList>(
    "ui.transactions.list",
    {
      limit: 500,
    },
  );
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
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
  const hasLiveOverview =
    overview.data?.kind === "ui.overview.snapshot" && Boolean(overview.data.data);
  const nowRate =
    hasLiveTransactions && hasLiveOverview
      ? (overview.data?.data?.priceEur ?? null)
      : hasLiveTransactions
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
    <Dashboard2
      transactions={transactions}
      nowRate={nowRate}
      swapCandidates={swapCandidates}
      swapCandidateTotal={swapCandidateTotal}
      isDataRefreshing={hasLiveTransactions && isFetching}
    />
  );
}
