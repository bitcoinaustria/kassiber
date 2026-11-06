/**
 * The counts every surface shares (`ui.review.badges`), and the one number
 * for what is left to review: the sidebar, the bell, the Review screen and
 * the assistant all show the same.
 */
export interface ReviewBadges {
  quarantine: number;
  /** Root causes: what needs the user. Older daemons send only the row count. */
  quarantine_attention?: number;
  journals_needs_processing: boolean;
  /** Transfer candidates from the last matcher run; null until it has run. */
  swaps: number | null;
  /** quarantine_attention plus swaps; absent from older daemons. */
  review?: number;
}

/** What is left to decide; the daemon's total when it sends one. */
export function reviewTotal(badges: ReviewBadges | null | undefined): number {
  if (!badges) return 0;
  return (
    badges.review ??
    (badges.quarantine_attention ?? badges.quarantine) + (badges.swaps ?? 0)
  );
}
