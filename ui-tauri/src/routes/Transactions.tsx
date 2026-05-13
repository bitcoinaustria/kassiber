import {
  Dashboard2,
  type SwapCandidateReference,
} from "@/components/dashboard2";
import { useDaemon } from "@/daemon/client";
import {
  MOCK_TRANSACTIONS,
  type TransactionsList,
} from "@/mocks/transactions";

interface SuggestEnvelope {
  candidates: SwapCandidateReference[];
}

export function Transactions() {
  const { data } = useDaemon<TransactionsList>("ui.transactions.list", {
    limit: 500,
  });
  const { data: swapData } = useDaemon<SuggestEnvelope>("ui.transfers.suggest");
  const hasLiveTransactions =
    data?.kind === "ui.transactions.list" && Boolean(data.data);
  const transactions =
    hasLiveTransactions && data.data
      ? data.data
      : MOCK_TRANSACTIONS;
  const swapCandidates =
    swapData?.kind === "ui.transfers.suggest" && swapData.data
      ? swapData.data.candidates
      : hasLiveTransactions
        ? []
      : undefined;

  return <Dashboard2 transactions={transactions} swapCandidates={swapCandidates} />;
}
