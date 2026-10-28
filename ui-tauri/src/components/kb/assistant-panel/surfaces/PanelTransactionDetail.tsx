import * as React from "react";
import { useTranslation } from "react-i18next";

import { TransactionDetailController } from "@/components/transactions/dashboard/TransactionDetailController";
import { toDashboardTransaction } from "@/components/transactions/dashboard/model";
import { RightPanelDetailOwnerContext } from "@/components/transactions/TransactionDetailTrail";
import { useDaemon } from "@/daemon/client";
import { useCurrency } from "@/lib/currency";
import type { Tx } from "@/mocks/seed";
import { useRightPanelStore, type PanelTransaction } from "@/store/rightPanel";
import { useUiStore } from "@/store/ui";

/**
 * A transaction the panel keeps open in a tab of its own: opened from one of
 * its surfaces, or a page's preview pinned. It reads the transaction by id
 * and runs the same detail controller the Source-of-Funds picker does.
 */
export function PanelTransactionDetail({ entry }: { entry: PanelTransaction }) {
  const { t } = useTranslation("transactions");
  const currency = useCurrency();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const closeTransaction = useRightPanelStore((s) => s.closeTransaction);
  const retargetTransaction = useRightPanelStore((s) => s.retargetTransaction);
  const query = useDaemon<{ transaction?: Tx | null }>("ui.transactions.resolve", {
    query: entry.transactionId,
  });
  const resolved = query.data?.data?.transaction;
  const transaction = React.useMemo(
    () =>
      resolved?.id === entry.transactionId
        ? toDashboardTransaction(
            resolved,
            0,
            t as (key: string, opts?: Record<string, unknown>) => string,
          )
        : null,
    [entry.transactionId, resolved, t],
  );
  return (
    <RightPanelDetailOwnerContext.Provider value={entry.id}>
      <TransactionDetailController
        open
        transaction={transaction}
        isLoading={query.isLoading || query.isFetching}
        onRetry={() => void query.refetch()}
        initialTab={entry.initialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        onOpenChange={(open) => {
          if (!open) closeTransaction(entry.id);
        }}
        onNavigate={(next) => retargetTransaction(entry.id, next.id)}
      />
    </RightPanelDetailOwnerContext.Provider>
  );
}
