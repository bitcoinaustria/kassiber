import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemon, useDaemonStreamMutation } from "@/daemon/client";
import type { ProfilesSnapshot } from "@/mocks/profiles";
import { useUiStore } from "@/store/ui";

const SETTLE_MS = 800;

/** Queued work survives navigation and can only process its originating book. */
export function useJournalRefreshAfterDecisions() {
  const queryClient = useQueryClient();
  const dataMode = useUiStore((s) => s.dataMode);
  const session = useUiStore((s) => s.daemonSession);
  const profiles = useDaemon<ProfilesSnapshot>("ui.profiles.snapshot");
  const workspaceId = profiles.data?.data?.activeWorkspaceId;
  const profileId = profiles.data?.data?.activeProfileId;
  const { mutateAsync } = useDaemonStreamMutation("ui.journals.process");
  const isProcessingJournals = useIsMutating({ mutationKey: daemonMutationKey(dataMode, "ui.journals.process") }) > 0;
  const [pending, setPending] = React.useState(false);
  const mounted = React.useRef(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  React.useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  React.useEffect(() => { setPending(false); }, [session]);

  const requestRefresh = React.useCallback(() => {
    if (!workspaceId || !profileId || useUiStore.getState().daemonSession !== session) return;
    if (mounted.current) setPending(true);
    void queryClient.invalidateQueries({
      queryKey: ["daemon", dataMode],
      predicate: (query) => query.queryKey.includes("ui.review.badges"),
    });
    clearTimeout(timer.current);
    const run = async () => {
      // The daemon also checks expected_scope, covering a switch in flight.
      if (useUiStore.getState().daemonSession !== session) return;
      if (queryClient.isMutating({ mutationKey: daemonMutationKey(dataMode, "ui.journals.process") })) {
        timer.current = setTimeout(() => void run(), SETTLE_MS);
        return;
      }
      if (mounted.current) setPending(false);
      try {
        await mutateAsync({ auto_pair: false, expected_scope: { workspace_id: workspaceId, profile_id: profileId } });
      } catch {
        // A failed refresh leaves the decision stored and the existing
        // freshness barrier visible until this book is processed again.
        void queryClient.invalidateQueries({ queryKey: ["daemon", dataMode] });
      }
    };
    // Keep this timer alive on unmount, including a response arriving after
    // unmount. Neither case needs another React render to start work.
    timer.current = setTimeout(() => void run(), SETTLE_MS);
  }, [dataMode, mutateAsync, profileId, queryClient, session, workspaceId]);

  return { requestRefresh, isProcessingJournals, refreshPending: pending };
}
