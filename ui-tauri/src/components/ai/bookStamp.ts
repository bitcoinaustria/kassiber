import type { AssistantBookScope } from "@/components/ai/useActiveBookScope";

/**
 * The book a piece of assistant content belongs to (a pin, a parked or sent
 * handoff, a queued question, a held edit, an attached file, a
 * conversation): the daemon session and that book's ids, from the same
 * snapshot as `expected_scope`.
 *
 * The rule: a stamp is captured synchronously at the user's action that
 * creates the content (click, attach, park, pin, submit), before any await.
 * It travels with the content, and a request is sent with its content's
 * stamp, never the one current when it is sent, so the daemon refuses it in
 * another book. When the open book's ids are not known at that moment, the
 * action is refused; nothing is stamped later.
 */
export interface BookStamp {
  daemonSession: number;
  scope: AssistantBookScope;
}

/** The book open now; its ids are unknown while they load. */
export interface OpenBook {
  daemonSession: number;
  scope: AssistantBookScope | null;
}

export function sameBookScope(a: AssistantBookScope, b: AssistantBookScope): boolean {
  return a.workspace_id === b.workspace_id && a.profile_id === b.profile_id;
}

/**
 * Whether content stamped `made` belongs to the book open `now`: the same
 * daemon session and, where the open book's ids are known, the same ids.
 * Creating a book opens it without a new daemon session, so the ids alone
 * change then.
 */
export function sameBook(made: BookStamp, now: OpenBook): boolean {
  return (
    made.daemonSession === now.daemonSession &&
    (now.scope === null || sameBookScope(made.scope, now.scope))
  );
}

/** A stamp read back from storage, or null when it is not a whole one. */
export function storedBookStamp(value: unknown): BookStamp | null {
  if (!value || typeof value !== "object") return null;
  const { daemonSession, scope } = value as Partial<BookStamp>;
  return typeof daemonSession === "number" &&
    scope !== null &&
    typeof scope === "object" &&
    typeof scope.workspace_id === "string" &&
    typeof scope.profile_id === "string"
    ? { daemonSession, scope: { workspace_id: scope.workspace_id, profile_id: scope.profile_id } }
    : null;
}
