import { useMemo } from "react";

import { draftForTransaction, metadataUpdateArgs, type Transaction, type TransactionEditDraft } from "@/components/transactions";
import { useDaemon } from "@/daemon/client";
import type { Tx } from "@/mocks/seed";
import { toDashboardTransaction } from "./model";

/** Details read the selected wallet leg, while the table keeps its net projection. */
export function useTransactionDetailRecord(
  selected: Transaction | null,
  t?: Parameters<typeof toDashboardTransaction>[2],
) {
  const query = useDaemon<{ transaction?: Tx | null }>(
    "ui.transactions.resolve",
    { query: selected?.id ?? "" },
    { enabled: Boolean(selected) },
  );
  const resolved = query.data?.data?.transaction;
  const record = useMemo(() => {
    if (!selected) return null;
    if (resolved?.id === selected.id) return toDashboardTransaction(resolved, 0, t);
    return null;
  }, [selected, resolved, t]);
  return { record, isLoading: query.isLoading, retry: query.refetch };
}

export function transactionDetailSaveArgs(
  transactionId: string,
  draft: TransactionEditDraft,
  records: Transaction[],
  detail: Transaction | null,
  drafts: Record<string, TransactionEditDraft>,
) {
  const source = detail?.id === transactionId
    ? detail
    : records.find((record) => record.id === transactionId);
  return metadataUpdateArgs({
    transactionId,
    draft,
    baseline: drafts[transactionId] ?? (source ? draftForTransaction(source) : null),
    sourceTags: source?.tags ?? [],
  });
}
