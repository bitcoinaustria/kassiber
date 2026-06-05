import {
  BookMarked,
  ExternalLink,
  Hash,
  Tags,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import type {
  HistoryRevertTarget,
  TransactionHistoryEvent,
  TransactionHistoryStaleSummary,
} from "@/lib/transactionHistory";
import { cn } from "@/lib/utils";

import { TransactionEditHistoryPanel } from "./TransactionEditHistoryPanel";
import { AttachmentsPanel } from "./TransactionDetailAttachmentsPanel";
import {
  DirtyDot,
  ReviewChecklist,
  SourceRecordRow,
  type AttachmentItem,
  type ChecklistItem,
} from "./TransactionDetailSheetParts";
import { blurClass, type Transaction } from "./model";

type ExplorerSummary = {
  label: string;
} | null;

export function TransactionDetailRightRail({
  transaction,
  sourceName,
  sourceType,
  explorer,
  reviewChecklistItems,
  onJumpTab,
  hideSensitive,
  attachments,
  onAddAttachmentFiles,
  onAddAttachmentLinks,
  onReuseEvidence,
  onOpenAttachment,
  onRenameAttachment,
  onRemoveAttachment,
  tags,
  dirtyTags,
  historyEvents,
  historyStale,
  historyLoading,
  isRevertingHistory,
  onRevertHistory,
  onProcessJournals,
  isProcessingJournals,
  onOpenExplorer,
}: {
  transaction: Transaction;
  sourceName: string;
  sourceType: string;
  explorer: ExplorerSummary;
  reviewChecklistItems: Array<ChecklistItem & { tab?: string }>;
  onJumpTab: (tab: string) => void;
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
  tags: string[];
  dirtyTags?: boolean;
  historyEvents?: TransactionHistoryEvent[];
  historyStale?: TransactionHistoryStaleSummary;
  historyLoading?: boolean;
  isRevertingHistory?: boolean;
  onRevertHistory?: (target: HistoryRevertTarget) => void | Promise<void>;
  onProcessJournals?: () => void;
  isProcessingJournals?: boolean;
  onOpenExplorer: (transaction: Transaction) => void;
}) {
  return (
    <aside className="space-y-3">
      <ReviewChecklist
        items={reviewChecklistItems}
        onJump={onJumpTab}
      />
      <div className="rounded-md border bg-card p-3">
        <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Hash
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
          Source record
        </div>
        <div className="space-y-2">
          <SourceRecordRow
            icon={<Hash className="size-3.5" aria-hidden="true" />}
            label="Kassiber row"
            value={transaction.id}
            copyValue={transaction.id}
            hidden={hideSensitive}
          />
          <SourceRecordRow
            icon={
              <BookMarked
                className="size-3.5"
                aria-hidden="true"
              />
            }
            label="Source"
            value={`${sourceName} · ${sourceType}`}
            hidden={hideSensitive}
          />
          <SourceRecordRow
            icon={
              <ExternalLink
                className="size-3.5"
                aria-hidden="true"
              />
            }
            label="Explorer"
            value={explorer ? explorer.label : "No public explorer"}
            action={
              explorer
                ? {
                    label: `Open ${transaction.txnId} on ${explorer.label}`,
                    onClick: () => onOpenExplorer(transaction),
                  }
                : undefined
            }
          />
        </div>
      </div>
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
      {tags.length ? (
        <div className="rounded-md border bg-card p-3">
          <div className="mb-3 flex items-center justify-between gap-2 text-sm font-semibold">
            <div className="flex items-center gap-2">
              <Tags
                className="size-4 text-muted-foreground"
                aria-hidden="true"
              />
              Tags
            </div>
            <DirtyDot active={dirtyTags} />
          </div>
          <div className="flex min-h-8 flex-wrap gap-1.5">
            {tags.map((tag) => (
              <Badge
                key={tag}
                variant="secondary"
                className={cn(
                  "rounded-md",
                  blurClass(hideSensitive),
                )}
              >
                {tag}
              </Badge>
            ))}
          </div>
        </div>
      ) : null}
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
