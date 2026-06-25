/**
 * Decision logic for the global header "Refresh book set" control.
 *
 * The button is the single entry point for bringing the active book current.
 * The underlying `ui.freshness.run` already chains source sync, auto-pairing,
 * and journal (re)processing into one pass, so one click covers
 * sync + refresh + reprocess. This planner keeps the side effects (navigation,
 * store writes, the daemon mutation) in the shell while the branching stays
 * pure and testable:
 *
 *  - No workspace yet → do nothing here (the caller bounces to onboarding).
 *  - Otherwise → surface the loader (un-minimize the sync card for the active
 *    book, if any) and start the refresh. The refresh is a no-op downstream
 *    when one is already running, so a mid-refresh click just re-opens the card.
 */
export type HeaderRefreshPlan = {
  /**
   * Book key whose sync card should be un-minimized so the loader shows again,
   * or null when there is nothing to re-open (no workspace / unresolved book).
   */
  reopenSyncCardForBook: string | null;
  /** Whether to start the book refresh. */
  startRefresh: boolean;
};

export function planHeaderRefresh(input: {
  hasWorkspace: boolean;
  bookKey: string | null;
}): HeaderRefreshPlan {
  if (!input.hasWorkspace) {
    return { reopenSyncCardForBook: null, startRefresh: false };
  }
  return { reopenSyncCardForBook: input.bookKey, startRefresh: true };
}
