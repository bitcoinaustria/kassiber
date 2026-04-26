import { Dashboard2 } from "@/components/dashboard2";
import { useDaemon } from "@/daemon/client";
import {
  MOCK_TRANSACTIONS,
  type TransactionsLedger,
} from "@/mocks/transactions";

export function Transactions() {
  const { data } = useDaemon<TransactionsLedger>("ui.transactions.list", {
    limit: 500,
  });
  const ledger =
    data?.kind === "ui.transactions.list" && data.data
      ? data.data
      : MOCK_TRANSACTIONS;

  return <Dashboard2 ledger={ledger} />;
}
