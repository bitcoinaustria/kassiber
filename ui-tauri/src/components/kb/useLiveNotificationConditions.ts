import * as React from "react";

import { useDaemon } from "@/daemon/client";
import type { ActiveBook } from "@/lib/activeBook";
import {
  reviewConditionsFromBadges,
  sourceConditionsFromSnapshot,
} from "@/lib/notificationConditions";
import type { ReviewBadges } from "@/lib/reviewCount";
import type { FreshnessSourceState } from "@/lib/syncResults";
import { useNotificationStore } from "@/store/notifications";

interface FreshnessStatusSnapshot {
  sources?: FreshnessSourceState[];
}

/**
 * Rebuild the center's live conditions from local book state whenever the
 * shell is unlocked: journals and Review from `ui.review.badges`, and
 * per-source refresh problems from `ui.freshness.status`. Both are local
 * reads with no egress, bound to the open book with `expected_scope`, so a
 * read the daemon answered for another book is refused rather than
 * attributed to this one. Only a successful read reconciles; a failed or
 * missing one resolves nothing, and reconciling never counts a repeat.
 *
 * Neither read polls. Every mutation that changes their answer invalidates
 * them (see `TARGETED_DAEMON_QUERY_INVALIDATIONS`), as do daemon events, and
 * the re-read reconciles here.
 */
export function useLiveNotificationConditions(
  enabled: boolean,
  book: ActiveBook | null,
) {
  const bound = enabled && book !== null;
  const args = book ? { expected_scope: book.expected } : undefined;
  const badges = useDaemon<ReviewBadges>("ui.review.badges", args, {
    enabled: bound,
  });
  const freshness = useDaemon<FreshnessStatusSnapshot>(
    "ui.freshness.status",
    args,
    { enabled: bound },
  );
  const badgeData = badges.isSuccess ? badges.data?.data : undefined;
  const sources = freshness.isSuccess ? freshness.data?.data?.sources : undefined;
  const key = book?.key ?? null;
  React.useEffect(() => {
    if (!bound || !badgeData || key === null) return;
    useNotificationStore
      .getState()
      .reconcileConditions(reviewConditionsFromBadges(badgeData, key));
  }, [badgeData, bound, key]);
  React.useEffect(() => {
    if (!bound || !Array.isArray(sources) || key === null) return;
    useNotificationStore
      .getState()
      .reconcileConditions(sourceConditionsFromSnapshot(sources, key));
  }, [bound, key, sources]);
}
