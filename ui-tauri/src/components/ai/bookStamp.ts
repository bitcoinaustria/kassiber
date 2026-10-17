import type { AssistantBookScope } from "@/components/ai/useActiveBookScope";

/**
 * The book something was made in (a pin, a parked handoff, a queued question,
 * a conversation): the daemon session and, once known, that book's ids from
 * the same snapshot as `expected_scope`. Its request names these ids, never
 * the book open when it is sent, so the daemon refuses it in another book.
 */
export interface BookStamp {
  daemonSession: number;
  scope: AssistantBookScope | null;
}

export function sameBookScope(a: AssistantBookScope, b: AssistantBookScope): boolean {
  return a.workspace_id === b.workspace_id && a.profile_id === b.profile_id;
}

/**
 * Whether something stamped `made` belongs to the book open `now`: the same
 * daemon session and, where both are known, the same ids. Creating a book
 * opens it without a new daemon session, so the ids alone change then.
 */
export function sameBook(made: BookStamp, now: BookStamp): boolean {
  return (
    made.daemonSession === now.daemonSession &&
    (made.scope === null || now.scope === null || sameBookScope(made.scope, now.scope))
  );
}
