import type { QueryClient } from "@tanstack/react-query";

import { readActiveBook } from "@/lib/activeBook";
import type { NotificationScope } from "@/store/notifications";

/**
 * The app's QueryClient, registered once at startup (`main.tsx`), so a
 * sender outside React's tree or without a provider in reach can still read
 * the cached `ui.profiles.snapshot`. It is the one client the app renders
 * with; nothing else is kept here.
 */
let appQueryClient: QueryClient | null = null;

export function registerNotificationQueryClient(client: QueryClient | null): void {
  appQueryClient = client;
}

/**
 * The scope to file a sender's entry under: the book open right now (its
 * key and the daemon ids an operation sends as `expected_scope`), bound at
 * the moment the work starts. Null while the open book is unknown or a
 * switch is in flight; a sender then keeps its toast but files nothing, so
 * no entry lands in, or acts on, the wrong book.
 */
export function activeBookScope(
  queryClient: QueryClient | null = appQueryClient,
): (NotificationScope & { book: string }) | null {
  if (!queryClient) return null;
  const active = readActiveBook(queryClient);
  return active ? { book: active.key, expected: active.expected } : null;
}
