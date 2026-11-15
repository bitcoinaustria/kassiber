import type { TFunction } from "i18next";

import type { CustodyLineageItem } from "./custodyGapsModel";

type LineageEvidenceKey =
  | "lineage.evidence.recordedFanout"
  | "lineage.evidence.multiSourceConsolidation"
  | "lineage.evidence.rowMatched"
  | "lineage.evidence.nativeTransfer"
  | "lineage.evidence.exchangeTxid"
  | "lineage.evidence.barkTxid"
  | "lineage.evidence.reviewedTransfer"
  | "lineage.evidence.reviewedGapBridge"
  | "lineage.evidence.channelLifecycle";

const LINEAGE_EVIDENCE_KEYS: Record<string, LineageEvidenceKey> = {
  recorded_fanout: "lineage.evidence.recordedFanout",
  multi_source_consolidation: "lineage.evidence.multiSourceConsolidation",
  row_matched: "lineage.evidence.rowMatched",
  verified_native_transfer: "lineage.evidence.nativeTransfer",
  exchange_txid_exact: "lineage.evidence.exchangeTxid",
  bark_txid_exact: "lineage.evidence.barkTxid",
  reviewed_transfer_pair: "lineage.evidence.reviewedTransfer",
  reviewed_gap_bridge: "lineage.evidence.reviewedGapBridge",
  manual: "lineage.evidence.reviewedTransfer",
  bulk_exact: "lineage.evidence.reviewedTransfer",
  bulk_selected: "lineage.evidence.reviewedTransfer",
  rule_auto: "lineage.evidence.reviewedTransfer",
  channel_lifecycle: "lineage.evidence.channelLifecycle",
};

/** Why a move is listed as between the owner's wallets, in words. */
export function lineageEvidenceLabel(
  item: Pick<CustodyLineageItem, "evidence_reason" | "custody_state">,
  t: TFunction<"custodyGaps">,
): string {
  const key = LINEAGE_EVIDENCE_KEYS[item.evidence_reason];
  if (key) return t(key);
  return item.custody_state === "internal_reviewed"
    ? t("lineage.evidence.reviewedDefault")
    : t("lineage.evidence.verifiedDefault");
}
