/**
 * What every linked transaction has left outside its links, from the pair
 * list the page already reads (`ui.transfers.list`). A local read of the
 * book; nothing leaves the machine.
 */
import * as React from "react";

import { useDaemon } from "@/daemon/client";

import type { TransferPairsEnvelope } from "./linkedMovesModel";
import { openParts } from "./linkModel";

export function useOpenParts() {
  const pairs = useDaemon<TransferPairsEnvelope>("ui.transfers.list", { include_unlinked: true }).data?.data?.pairs;
  return React.useMemo(() => (pairs ? openParts(pairs) : null), [pairs]);
}
