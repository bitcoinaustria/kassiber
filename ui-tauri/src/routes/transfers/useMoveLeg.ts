/**
 * One transaction read as a leg (`ui.transactions.resolve`): its wallet,
 * rail, time, exact amount, fee and txid. A local read of the book; nothing
 * leaves the machine.
 */
import { useDaemon } from "@/daemon/client";
import type { Tx } from "@/mocks/seed";

import { legFromTx, type LinkLeg } from "./linkModel";

interface ResolveEnvelope {
  transaction?: Tx | null;
}

export function useMoveLeg(id: string | null | undefined): { leg: LinkLeg | null; tx: Tx | null; loading: boolean; failed: boolean } {
  const query = useDaemon<ResolveEnvelope>("ui.transactions.resolve", { query: id ?? "" }, { enabled: Boolean(id) });
  const tx = query.data?.data?.transaction ?? null;
  return {
    leg: tx ? legFromTx(tx) : null,
    tx,
    loading: Boolean(id) && query.isLoading,
    failed: query.isError || Boolean(query.data?.error),
  };
}
