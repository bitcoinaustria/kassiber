import * as React from "react";
import { useIsMutating, type QueryClient } from "@tanstack/react-query";

import {
  DaemonRequestError,
  daemonMutationKey,
  daemonQueryKey,
  useDaemon,
} from "@/daemon/client";
import { getTransport } from "@/daemon/transport";
import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { bookIdentityKey, useUiStore } from "@/store/ui";

/** A book as the daemon's `expected_scope` names it. */
export interface DaemonBookScope {
  workspace_id: string;
  profile_id: string;
}

/**
 * The book open right now: a stable key for notification scope (project +
 * workspace + profile, since one project holds several books) and the ids a
 * request carries as `expected_scope`, so the daemon refuses it if another
 * book is open by the time it runs.
 */
export interface ActiveBook {
  key: string;
  expected: DaemonBookScope;
}

interface ProfilesSnapshotIds {
  activeWorkspaceId?: string | null;
  activeProfileId?: string | null;
}

const SNAPSHOT_KIND = "ui.profiles.snapshot";
const SWITCH_KIND = "ui.profiles.switch";

export function bookKeyFor(project: string | null, scope: DaemonBookScope): string {
  return JSON.stringify([project, scope.workspace_id, scope.profile_id]);
}

function activeBookFrom(
  data: ProfilesSnapshotIds | undefined,
  project: string | null,
): ActiveBook | null {
  const workspace = data?.activeWorkspaceId;
  const profile = data?.activeProfileId;
  if (!workspace || !profile) return null;
  const expected = { workspace_id: workspace, profile_id: profile };
  return { key: bookKeyFor(project, expected), expected };
}

let pendingProjectSwitches = 0;

/**
 * Mark a project switch (`ui.projects.select`) in flight. While one is,
 * there is no active book to bind an action to.
 */
export async function trackBookSwitch<T>(work: Promise<T>): Promise<T> {
  pendingProjectSwitches += 1;
  try {
    return await work;
  } finally {
    pendingProjectSwitches -= 1;
  }
}

/** A project or book switch is in flight. */
export function bookSwitchPending(queryClient: QueryClient): boolean {
  const { dataMode } = useUiStore.getState();
  return (
    pendingProjectSwitches > 0 ||
    queryClient.isMutating({ mutationKey: daemonMutationKey(dataMode, SWITCH_KIND) }) > 0
  );
}

function snapshotQueryKey() {
  const { dataMode, daemonSession } = useUiStore.getState();
  return daemonQueryKey(dataMode, daemonSession, SNAPSHOT_KIND);
}

/**
 * The open book from this daemon session's cached `ui.profiles.snapshot`,
 * or null while it is unknown or a switch is in flight. Synchronous, so an
 * action can bind it at the moment of the click.
 */
export function readActiveBook(queryClient: QueryClient): ActiveBook | null {
  if (bookSwitchPending(queryClient)) return null;
  const envelope = queryClient.getQueryData<{ data?: ProfilesSnapshotIds }>(
    snapshotQueryKey(),
  );
  return activeBookFrom(
    envelope?.data,
    bookIdentityKey(useUiStore.getState().identity),
  );
}

/**
 * The open book, reading `ui.profiles.snapshot` (a local read) when this
 * session has not yet. Whatever it returns is bound into the request, so a
 * switch during the read is refused by the daemon, not misattributed.
 */
export async function resolveActiveBook(
  queryClient: QueryClient,
): Promise<ActiveBook | null> {
  const cached = readActiveBook(queryClient);
  if (cached || bookSwitchPending(queryClient)) return cached;
  try {
    await queryClient.fetchQuery({
      queryKey: snapshotQueryKey(),
      // As `useDaemon` reads it, so the cache entry is the same either way.
      queryFn: async () => {
        const envelope = await getTransport().invoke<ProfilesSnapshotIds>({
          kind: SNAPSHOT_KIND,
        });
        if (envelope.kind === "error" || envelope.error) {
          throw new DaemonRequestError(SNAPSHOT_KIND, envelope);
        }
        return envelope;
      },
    });
    return readActiveBook(queryClient);
  } catch {
    return null;
  }
}

/** The open book for rendering; null while unknown or switching. */
export function useActiveBook(enabled: boolean): ActiveBook | null {
  const query = useDaemon<ProfilesSnapshotIds>(SNAPSHOT_KIND, undefined, {
    enabled,
  });
  const dataMode = useUiStore((s) => s.dataMode);
  const identity = useUiStore((s) => s.identity);
  const switching =
    useIsMutating({ mutationKey: daemonMutationKey(dataMode, SWITCH_KIND) }) > 0;
  const data = query.isPlaceholderData ? undefined : query.data?.data;
  const workspace = data?.activeWorkspaceId;
  const profile = data?.activeProfileId;
  const project = bookIdentityKey(identity);
  return React.useMemo(
    () =>
      !enabled || switching
        ? null
        : activeBookFrom(
            { activeWorkspaceId: workspace, activeProfileId: profile },
            project,
          ),
    [enabled, profile, project, switching, workspace],
  );
}

/** The daemon refused a request bound to a book that is no longer open. */
export function isStaleBookError(error: unknown): boolean {
  return (
    error instanceof DaemonRequestError &&
    error.envelope.error?.code === "stale_context"
  );
}

/**
 * Bind a run to its book before it starts. A Retry passes the ids its
 * entry carried; a fresh request takes the open book, reading it if this
 * session has not yet. `start` gets the scope to attribute the outcome to
 * and to send as `expected_scope`. When no book can be bound the run does
 * not start: an unattributable job could neither be retried nor resolved.
 */
export function bindRunBook(
  queryClient: QueryClient,
  expected: DaemonBookScope | undefined,
  start: (scope: { book: string; expected: DaemonBookScope }) => void,
  onUnknown: () => void,
): void {
  if (expected) {
    start({
      book: bookKeyFor(bookIdentityKey(useUiStore.getState().identity), expected),
      expected,
    });
    return;
  }
  const active = readActiveBook(queryClient);
  if (active) {
    start({ book: active.key, expected: active.expected });
    return;
  }
  void resolveActiveBook(queryClient).then((book) => {
    if (book) start({ book: book.key, expected: book.expected });
    else onUnknown();
  });
}

/** No book could be bound to a new run, so it did not start. */
export function reportBookUnknown(): void {
  showFeedback({
    tone: "info",
    title: i18n.t("chrome:notifications.bookUnknown"),
    key: "book-unknown",
  });
}

/** The daemon refused a run bound to a book that is no longer open. */
export function reportStaleBook(): void {
  showFeedback({
    tone: "info",
    title: i18n.t("chrome:notifications.staleBook"),
    key: "stale-book",
  });
}
