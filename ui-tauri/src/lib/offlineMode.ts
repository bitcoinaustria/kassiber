import * as React from "react";

import { useDaemon, useDaemonMutation } from "@/daemon/client";

export interface OfflineModeStatus {
  offline: boolean;
  environment_blocked: boolean;
}

export interface OfflineModeState {
  /** The user's switch: nothing connects off this device. */
  offline: boolean;
  /**
   * `KASSIBER_NO_EGRESS` blocks backend connections for this process. It
   * covers fewer paths than the switch, so the switch stays usable.
   */
  environmentBlocked: boolean;
  /** Either one: backend connections can neither run nor be checked. */
  blocked: boolean;
  /** The daemon has answered, so `offline` is the real state. */
  known: boolean;
  pending: boolean;
  /** The last change was refused. */
  failed: boolean;
  setOffline: (offline: boolean) => void;
}

export function offlineModeFromStatus(
  status: OfflineModeStatus | null | undefined,
): Pick<OfflineModeState, "offline" | "environmentBlocked" | "blocked" | "known"> {
  const offline = status?.offline === true;
  const environmentBlocked = status?.environment_blocked === true;
  return {
    offline,
    environmentBlocked,
    blocked: offline || environmentBlocked,
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
  const [failed, setFailed] = React.useState(false);
  const { mutateAsync } = mutation;
  const setOffline = React.useCallback(
    (offline: boolean) => {
      setFailed(false);
      mutateAsync({ enabled: offline }).catch(() => setFailed(true));
    },
    [mutateAsync],
  );
  const status =
    query.data?.kind === "ui.network.offline" ? query.data.data : null;
  return {
    ...offlineModeFromStatus(status),
    pending: mutation.isPending,
    failed,
    setOffline,
  };
}
