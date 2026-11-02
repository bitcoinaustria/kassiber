import * as React from "react";
import { CheckCircle2, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { QuarantineSection } from "@/components/kb/quarantine";
import { ReviewTransfers } from "@/components/review/ReviewTransfers";
import type { TransferSuggestions } from "@/components/review/transferModel";
import { useJournalRefreshAfterDecisions } from "@/components/review/useJournalRefreshAfterDecisions";
import { useDaemon } from "@/daemon/client";
import type { ReviewBadges } from "@/lib/reviewCount";
import { screenShellClassName } from "@/lib/screen-layout";

/**
 * Everything in the book that waits on the owner, in one place: moves
 * between wallets to confirm, then what cannot be booked yet. Each item is
 * decided here, and the journals recalculate after a decision.
 */
export function Review() {
  const { t } = useTranslation("review");
  const badgesQuery = useDaemon<ReviewBadges>("ui.review.badges");
  const { requestRefresh, isProcessingJournals, refreshPending } =
    useJournalRefreshAfterDecisions();
  // The transfer list's own read: the badge's transfer count is cached from
  // the last journal run and can lag a decision.
  const suggestQuery = useDaemon<TransferSuggestions>("ui.transfers.suggest", {});
  const badges = badgesQuery.data?.data ?? null;
  const candidates = suggestQuery.data?.data?.candidates;
  const updating = isProcessingJournals || refreshPending;
  // A failed run leaves the lists as before it; that is not a clear book.
  const [failedRun, setFailedRun] = React.useState(false);
  const clear =
    !updating &&
    !failedRun &&
    Boolean(badges) &&
    (badges?.quarantine_attention ?? badges?.quarantine ?? 0) === 0 &&
    !badges?.journals_needs_processing &&
    candidates?.length === 0;

  return (
    <div className={screenShellClassName}>
      {/* Each section counts what it lists; the sidebar's Review badge
          carries the total, which this screen does not repeat. */}
      {updating ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Loader2 className="size-3.5 animate-spin" aria-hidden="true" />
          {t("inbox.updating")}
        </p>
      ) : null}

      {clear ? (
        <div className="kb-surface flex items-start gap-3 p-(--kb-card-padding)" data-testid="review-clear">
          <CheckCircle2 className="mt-0.5 size-4 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden="true" />
          <div className="space-y-0.5">
            <p className="text-sm font-semibold">{t("inbox.clear.title")}</p>
            <p className="text-sm text-muted-foreground">{t("inbox.clear.body")}</p>
          </div>
        </div>
      ) : null}

      {/* Quick yes-or-no decisions first: pairing a move often clears a
          cause below with it. */}
      {/* Wider apart than the rows inside them, so the second heading reads
          as a section of its own, not a footer of the moves. */}
      <div className="space-y-6">
        <ReviewTransfers onDecided={requestRefresh} refreshing={updating} />
        <QuarantineSection onFailedRunChange={setFailedRun} />
      </div>
    </div>
  );
}
