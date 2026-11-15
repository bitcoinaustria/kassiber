import type { TFunction } from "i18next";

import type { TransferCandidate } from "./transferModel";

/** Minutes between the two legs, from their ISO times. */
function legMinutes(candidate: TransferCandidate): number | null {
  const a = Date.parse(candidate.out_occurred_at);
  const b = Date.parse(candidate.in_occurred_at);
  return Number.isFinite(a) && Number.isFinite(b) ? Math.round(Math.abs(b - a) / 60_000) : null;
}

/**
 * Why the matcher suggests a pair, in one short phrase: the shared evidence
 * when there is some, otherwise how close the amounts and times are.
 */
export function suggestionReason(
  t: TFunction<"review">,
  candidate: TransferCandidate,
): string {
  switch (candidate.method) {
    case "payment_hash":
      return t("inbox.transfers.reason.paymentHash");
    case "provider_swap_id":
      return t("inbox.transfers.reason.provider");
    case "htlc_refund":
      return t("inbox.transfers.reason.refund");
    case "ownership_graph":
      return t("inbox.transfers.reason.ownership");
    default: {
      const minutes = legMinutes(candidate);
      const time =
        minutes === null
          ? ""
          : minutes < 90
            ? t("inbox.transfers.reason.minutes", { count: minutes })
            : minutes < 48 * 60
              ? t("inbox.transfers.reason.hours", { count: Math.round(minutes / 60) })
              : t("inbox.transfers.reason.days", { count: Math.round(minutes / 1440) });
      // A larger arrival is no fee, so it is only a close amount.
      return candidate.swap_fee_msat === 0
        ? t("inbox.transfers.reason.sameAmount", { time })
        : candidate.swap_fee_msat > 0
          ? t("inbox.transfers.reason.closeAmount", { time })
          : t("inbox.transfers.reason.similarAmount", { time });
    }
  }
}
