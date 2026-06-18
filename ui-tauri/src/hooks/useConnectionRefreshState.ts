import { useIsMutating } from "@tanstack/react-query";

import { daemonMutationKey } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import type { Connection } from "@/mocks/seed";

/**
 * Single source of truth for "is this wallet/connection currently refreshing?".
 *
 * Previously the Wallet Detail header button only watched `ui.wallets.sync`, so
 * when a *global book refresh* (`ui.freshness.run`, fired from Overview, the
 * AppShell, or the Transactions screen) was running, the page data refreshed
 * and the global maintenance indicator spun while this page's Refresh button
 * sat idle — the "one button animates, the other doesn't" mismatch. This hook
 * folds in the book refresh so every refresh affordance on the page reflects
 * the same state.
 *
 * It does NOT include the component's own `mutation.isPending`; callers can OR
 * that in for immediate feedback, though the shared `ui.wallets.sync` mutation
 * key already makes `useIsMutating` observe it.
 */
export function useConnectionRefreshState(
  connection: Pick<Connection, "status">,
): boolean {
  const dataMode = useUiStore((state) => state.dataMode);
  const walletSyncsInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.wallets.sync"),
  });
  const bookRefreshesInFlight = useIsMutating({
    mutationKey: daemonMutationKey(dataMode, "ui.freshness.run"),
  });
  return (
    walletSyncsInFlight > 0 ||
    bookRefreshesInFlight > 0 ||
    connection.status === "syncing"
  );
}
