import { toDashboardTransaction } from "@/components/transactions/dashboard/model";
import type { Transaction } from "@/components/transactions";
import type { OverviewSnapshot } from "@/mocks/seed";

export function overviewDetailTransactions(
  snapshot: OverviewSnapshot,
  extraTransactions: OverviewSnapshot["txs"] = [],
): Transaction[] {
  const transactionsById = new Map<string, Transaction>();
  const addTransactions = (transactions: OverviewSnapshot["txs"]) => {
    for (const tx of transactions) {
      if (transactionsById.has(tx.id)) continue;
      transactionsById.set(
        tx.id,
        toDashboardTransaction(tx, transactionsById.size),
      );
    }
  };

  addTransactions(snapshot.txs);
  addTransactions(snapshot.activityTxs ?? []);
  addTransactions(extraTransactions);
  return Array.from(transactionsById.values());
}
