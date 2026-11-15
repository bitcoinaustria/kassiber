import * as React from "react";

import type { TransferSuggestions } from "@/components/review/transferModel";

/**
 * The suggestions as read before the journals started recalculating, kept
 * until the run is done and they are read again; the live list otherwise.
 */
export function useSettledSuggestions(
  live: TransferSuggestions["candidates"] | undefined,
  updating: boolean,
  refetch: () => Promise<unknown>,
): TransferSuggestions["candidates"] {
  const [frozen, setFrozen] = React.useState<TransferSuggestions["candidates"] | null>(null);
  const wasUpdating = React.useRef(updating);
  // Counts runs, so a read that finishes after the next run started is not taken for it.
  const run = React.useRef(0);
  React.useEffect(() => {
    if (updating && !wasUpdating.current) {
      run.current += 1;
      setFrozen((current) => current ?? live ?? []);
    }
    if (!updating && wasUpdating.current) {
      const finished = run.current;
      void refetch().then(() => {
        if (run.current === finished) setFrozen(null);
      });
    }
    wasUpdating.current = updating;
  }, [live, refetch, updating]);
  return frozen ?? live ?? [];
}
