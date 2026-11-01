import * as React from "react";
import { useTranslation } from "react-i18next";

import type { ReviewArtifact } from "@/components/ai/reviewWorkflow";

import { ReviewProposalDialog, type ReviewProposalCopy } from "./ReviewProposalDialog";
import type { QuarantineBookScope } from "./types";

/** Recorded with each acceptance in the transaction's history. */
export const ACCEPT_PRICES_REASON =
  "The owner accepted the approximate price on the Review screen.";

/** How many transactions the proposal accepts. */
function acceptedCount(artifact: ReviewArtifact) {
  const ids = artifact.operations[0]?.transaction_ids;
  return Array.isArray(ids) ? ids.length : 0;
}

/**
 * Accepts the approximate price of every transaction held for a coarse-price
 * review, in one reviewed step. The daemon names each transaction in the
 * preview. Each acceptance holds only for the price as it is; a price that
 * changes is held again.
 */
export function AcceptCoarsePricesDialog({
  scope,
  onClose,
  hideSensitive,
}: {
  /** The book the cause was read in; null closes the dialog. */
  scope: QuarantineBookScope | null;
  onClose: () => void;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("journals");
  // The daemon names the transactions; the request leaves them to it.
  const request = React.useMemo(
    () =>
      scope
        ? { scope, operations: [{ type: "accept_coarse_prices", reason: ACCEPT_PRICES_REASON }] }
        : null,
    [scope],
  );
  const copy: ReviewProposalCopy = {
    title: (artifact) =>
      artifact
        ? t("quarantine.accept.title", { count: acceptedCount(artifact) })
        : t("quarantine.accept.titlePending"),
    body: t("quarantine.accept.body"),
    footnote: t("quarantine.accept.undo"),
    confirm: (artifact) =>
      artifact
        ? t("quarantine.accept.confirm", { count: acceptedCount(artifact) })
        : t("quarantine.accept.confirmPending"),
    applying: t("quarantine.accept.applying"),
    done: (artifact) => t("quarantine.accept.doneTitle", { count: acceptedCount(artifact) }),
    dedupeKey: "quarantine-accept-prices",
  };
  return <ReviewProposalDialog request={request} copy={copy} onClose={onClose} hideSensitive={hideSensitive} />;
}
