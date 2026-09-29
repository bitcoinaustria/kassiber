import { useDaemon, useDaemonMutation } from "@/daemon/client";

export interface OfflineModeStatus {
  offline: boolean;
  environment_blocked: boolean;
}

export interface OfflineModeState {
  /** Nothing may connect: the user's switch, or `KASSIBER_NO_EGRESS`. */
  offline: boolean;
  /** The operator's process override; the switch cannot lift it. */
  environmentBlocked: boolean;
  /** The daemon has answered, so `offline` is the real state. */
  known: boolean;
  pending: boolean;
  setOffline: (offline: boolean) => Promise<unknown>;
}

export function offlineModeFromStatus(
  status: OfflineModeStatus | null | undefined,
): Pick<OfflineModeState, "offline" | "environmentBlocked" | "known"> {
  const environmentBlocked = status?.environment_blocked === true;
  return {
    offline: status?.offline === true || environmentBlocked,
    environmentBlocked,
    known: status != null,
  };
}

/**
 * The machine-wide offline switch the daemon and every CLI process read
 * before connecting. Reading it touches only a local file, so any surface
 * may ask, including before the book is unlocked.
 */
export function useOfflineMode(enabled = true): OfflineModeState {
  const query = useDaemon<OfflineModeStatus>("ui.network.offline", undefined, {
    enabled,
    retry: false,
  });
  const mutation = useDaemonMutation<OfflineModeStatus>("ui.network.offline.set");
  const status =
    query.data?.kind === "ui.network.offline" ? query.data.data : null;
  return {
    ...offlineModeFromStatus(status),
    pending: mutation.isPending,
    setOffline: (offline) => mutation.mutateAsync({ enabled: offline }),
  };
}
