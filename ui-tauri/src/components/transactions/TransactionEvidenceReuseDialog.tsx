import { Check, FileText, Link2, Repeat2 } from "lucide-react";
import * as React from "react";

import type { AttachmentItem } from "@/components/transactions/TransactionDetailSheet";
import type { Transaction } from "@/components/transactions/model";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";

function transactionOptionLabel(transaction: Transaction) {
  const parts = [
    transaction.txnId,
    transaction.date,
    transaction.wallet,
  ].filter(Boolean);
  return parts.join(" · ");
}

export function TransactionEvidenceReuseDialog({
  open,
  onOpenChange,
  targetTransaction,
  sourceTransactions,
  sourceTransactionId,
  onSourceTransactionIdChange,
  sourceAttachments,
  isLoadingSourceAttachments,
  isCopying,
  hideSensitive,
  onCopy,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  targetTransaction: Transaction | null;
  sourceTransactions: Transaction[];
  sourceTransactionId: string;
  onSourceTransactionIdChange: (transactionId: string) => void;
  sourceAttachments: AttachmentItem[];
  isLoadingSourceAttachments?: boolean;
  isCopying?: boolean;
  hideSensitive?: boolean;
  onCopy: (attachmentIds: string[]) => void | Promise<void>;
}) {
  const [selectedIds, setSelectedIds] = React.useState<string[]>([]);

  React.useEffect(() => {
    setSelectedIds([]);
  }, [sourceTransactionId, open]);

  const selectedSet = React.useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedCount = selectedIds.length;

  const toggleAttachment = (attachmentId: string) => {
    setSelectedIds((current) =>
      current.includes(attachmentId)
        ? current.filter((id) => id !== attachmentId)
        : [...current, attachmentId],
    );
  };

  const submit = async () => {
    if (!selectedIds.length) return;
    const attachmentIds = [...selectedIds];
    setSelectedIds([]);
    onOpenChange(false);
    try {
      await onCopy(attachmentIds);
    } catch (error) {
      setSelectedIds(attachmentIds);
      onOpenChange(true);
      throw error;
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Reuse evidence</DialogTitle>
          <DialogDescription>
            Copy selected evidence rows onto the current transaction.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
              Source transaction
            </div>
            <Select
              value={sourceTransactionId}
              onValueChange={onSourceTransactionIdChange}
            >
              <SelectTrigger className={cn(hideSensitive && "sensitive")}>
                <SelectValue placeholder="Choose a transaction" />
              </SelectTrigger>
              <SelectContent>
                {sourceTransactions.map((transaction) => (
                  <SelectItem key={transaction.id} value={transaction.id}>
                    {transactionOptionLabel(transaction)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="rounded-md border">
            <div className="flex items-center justify-between border-b px-3 py-2">
              <div className="flex items-center gap-2 text-sm font-medium">
                <Repeat2 className="size-4 text-muted-foreground" aria-hidden="true" />
                Evidence to copy
              </div>
              <Badge variant="outline" className="rounded-md">
                {selectedCount} selected
              </Badge>
            </div>
            <div className="max-h-72 overflow-y-auto p-2">
              {isLoadingSourceAttachments ? (
                <div className="px-2 py-6 text-center text-sm text-muted-foreground">
                  Loading evidence…
                </div>
              ) : sourceAttachments.length ? (
                <div className="space-y-1.5">
                  {sourceAttachments.map((attachment) => {
                    const checked = selectedSet.has(attachment.id);
                    return (
                      <label
                        key={attachment.id}
                        className={cn(
                          "flex min-w-0 cursor-pointer items-start gap-3 rounded-md border px-3 py-2",
                          checked ? "border-primary bg-primary/5" : "bg-card",
                        )}
                      >
                        <Checkbox
                          checked={checked}
                          onCheckedChange={() => toggleAttachment(attachment.id)}
                          className="mt-0.5"
                        />
                        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                          {attachment.kind === "file" ? (
                            <FileText className="size-3.5" aria-hidden="true" />
                          ) : (
                            <Link2 className="size-3.5" aria-hidden="true" />
                          )}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span
                            className={cn(
                              "block truncate text-sm font-medium",
                              hideSensitive && "sensitive",
                            )}
                          >
                            {attachment.label}
                          </span>
                          {attachment.detail ? (
                            <span
                              className={cn(
                                "block truncate text-xs text-muted-foreground",
                                hideSensitive && "sensitive",
                              )}
                            >
                              {attachment.detail}
                            </span>
                          ) : null}
                        </span>
                        {attachment.copiedFromAttachmentId ? (
                          <Badge variant="secondary" className="rounded-md">
                            reused
                          </Badge>
                        ) : null}
                      </label>
                    );
                  })}
                </div>
              ) : (
                <div className="px-2 py-6 text-center text-sm text-muted-foreground">
                  No evidence is attached to the selected transaction.
                </div>
              )}
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            className="gap-2"
            disabled={!targetTransaction || !selectedIds.length || isCopying}
            onClick={submit}
          >
            {isCopying ? (
              <Repeat2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Check className="size-4" aria-hidden="true" />
            )}
            Copy evidence
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
