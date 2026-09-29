import type {
  HistoryRevertTarget,
  TransactionHistoryEvent,
  TransactionHistoryStaleSummary,
} from "@/lib/transactionHistory";

import { TransactionEditHistoryPanel } from "./TransactionEditHistoryPanel";
import { AttachmentsPanel } from "./TransactionDetailAttachmentsPanel";
import { type AttachmentItem } from "./TransactionDetailSheetParts";

/** Evidence and history; the record's facts live in the Details tab. */
export function TransactionDetailRightRail({
  hideSensitive,
  attachments,
  onAddAttachmentFiles,
  onAddAttachmentLinks,
  onReuseEvidence,
  onOpenAttachment,
  onRenameAttachment,
  onRemoveAttachment,
  historyEvents,
  historyStale,
  historyLoading,
  isRevertingHistory,
  onRevertHistory,
  onProcessJournals,
  isProcessingJournals,
}: {
  hideSensitive: boolean;
  attachments?: AttachmentItem[];
  onAddAttachmentFiles?: (paths: string[]) => void | Promise<void>;
  onAddAttachmentLinks?: (urls: string[]) => void | Promise<void>;
  onReuseEvidence?: () => void;
  onOpenAttachment?: (item: AttachmentItem) => void;
  onRenameAttachment?: (
    item: AttachmentItem,
    label: string,
  ) => void | Promise<void>;
  onRemoveAttachment?: (item: AttachmentItem) => void;
  historyEvents?: TransactionHistoryEvent[];
  historyStale?: TransactionHistoryStaleSummary;
  historyLoading?: boolean;
  isRevertingHistory?: boolean;
  onRevertHistory?: (target: HistoryRevertTarget) => void | Promise<void>;
  onProcessJournals?: () => void;
  isProcessingJournals?: boolean;
}) {
  return (
    <aside className="space-y-3">
      <AttachmentsPanel
        items={attachments}
        hideSensitive={hideSensitive}
        onAddFiles={onAddAttachmentFiles}
        onAddLinks={onAddAttachmentLinks}
        onReuseEvidence={onReuseEvidence}
        onOpen={onOpenAttachment}
        onRename={onRenameAttachment}
        onRemove={onRemoveAttachment}
      />
      <TransactionEditHistoryPanel
        events={historyEvents}
        stale={historyStale}
        hideSensitive={hideSensitive}
        isLoading={historyLoading}
        onRevert={onRevertHistory}
        isReverting={isRevertingHistory}
        onProcessJournals={onProcessJournals}
        isProcessingJournals={isProcessingJournals}
      />
    </aside>
  );
}
