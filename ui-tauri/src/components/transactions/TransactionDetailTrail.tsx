import * as React from "react";

import { formatShortTxid, type Transaction } from "./model";

type TrailEntry = { id: string; label: string };

/**
 * The transactions reached by following coins inside the sheet, so "back"
 * returns along the path as in a block explorer. Opening a row from elsewhere
 * (the list, a deep link) starts a fresh trail.
 */
export function useTransactionTrail(
  transaction: Transaction | null,
  open: boolean,
  onOpenTransaction?: (transactionId: string) => void,
) {
  const [trail, setTrail] = React.useState<TrailEntry[]>([]);
  const pending = React.useRef<{ target: string; back: boolean } | null>(null);
  const shown = React.useRef<TrailEntry | null>(null);
  const currentId = transaction?.id ?? null;
  React.useEffect(() => {
    if (!open) {
      setTrail([]);
      pending.current = null;
      shown.current = null;
      return;
    }
    if (!currentId || !transaction) return;
    const previous = shown.current;
    const step = pending.current;
    if (previous && previous.id !== currentId) {
      if (step && step.target === currentId) {
        setTrail((current) => (step.back ? current.slice(0, -1) : [...current, previous]));
      } else {
        setTrail([]);
      }
    }
    pending.current = null;
    shown.current = {
      id: currentId,
      label: formatShortTxid(transaction.explorerId || transaction.txnId || currentId),
    };
    // Only the row identity drives the trail; field updates of the same row do not.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, currentId]);
  const follow = onOpenTransaction
    ? (transactionId: string) => {
        pending.current = { target: transactionId, back: false };
        onOpenTransaction(transactionId);
      }
    : undefined;
  const last = trail.at(-1);
  const back =
    onOpenTransaction && last
      ? () => {
          pending.current = { target: last.id, back: true };
          onOpenTransaction(last.id);
        }
      : undefined;
  return { follow, back, backLabel: last?.label ?? null };
}

export type TransactionTrail = ReturnType<typeof useTransactionTrail>;

/** Provided by the sheet frame; the keyed body reads it across transactions. */
export const TransactionTrailContext = React.createContext<TransactionTrail | null>(null);
