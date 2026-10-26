import * as React from "react";

import { useDaemon } from "@/daemon/client";

/** The book a chat turn belongs to, as the daemon's `expected_scope` takes it. */
export interface AssistantBookScope {
  workspace_id: string;
  profile_id: string;
}

/**
 * The open book's ids, read locally from `ui.profiles.snapshot` for the
 * current daemon session; null until known. Every assistant turn carries the
 * book it was asked in, and the daemon refuses one asked in another book
 * before any model or tool sees it.
 */
export function useActiveBookScope(): AssistantBookScope | null {
  const query = useDaemon<{
    activeWorkspaceId?: string | null;
    activeProfileId?: string | null;
  }>("ui.profiles.snapshot");
  const workspaceId = query.isPlaceholderData
    ? null
    : query.data?.data?.activeWorkspaceId;
  const profileId = query.isPlaceholderData
    ? null
    : query.data?.data?.activeProfileId;
  return React.useMemo(
    () =>
      workspaceId && profileId
        ? { workspace_id: workspaceId, profile_id: profileId }
        : null,
    [workspaceId, profileId],
  );
}
