import { ExternalLink, ShieldAlert } from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { openExternalUrl } from "@/daemon/transport";
import type { ExplorerTarget } from "@/lib/explorer";

import type { Transaction } from "./model";

function explorerOpenErrorMessage(error: unknown) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "Could not open explorer in the default browser.";
}

export function ExplorerOpenDialog({
  transaction,
  target,
  onTransactionChange,
}: {
  transaction: Transaction | null;
  target: ExplorerTarget | null;
  onTransactionChange: (transaction: Transaction | null) => void;
}) {
  const [openError, setOpenError] = React.useState<string | null>(null);
  const [opening, setOpening] = React.useState(false);

  React.useEffect(() => {
    if (!transaction) {
      setOpenError(null);
    }
  }, [transaction]);

  const openExplorer = async () => {
    if (!target) return;
    setOpenError(null);
    setOpening(true);
    try {
      await openExternalUrl(target.url);
      onTransactionChange(null);
    } catch (error) {
      setOpenError(explorerOpenErrorMessage(error));
    } finally {
      setOpening(false);
    }
  };

  return (
    <Dialog
      open={Boolean(transaction)}
      onOpenChange={(open) => {
        if (!open) {
          onTransactionChange(null);
        }
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] w-[min(calc(100vw-2rem),34rem)] overflow-hidden p-0 sm:max-w-none">
        <div className="grid max-h-[calc(100dvh-2rem)] min-w-0 gap-4 overflow-y-auto p-4 sm:p-6">
        <DialogHeader className="min-w-0 pr-8">
          <div className="mb-1 flex size-10 shrink-0 items-center justify-center rounded-full bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
            <ShieldAlert className="size-5" aria-hidden="true" />
          </div>
          <DialogTitle>Open transaction in a browser?</DialogTitle>
          <DialogDescription className="max-w-prose">
            This opens {target?.label ?? "the configured explorer"} outside Kassiber.
            The explorer can see your IP address and the transaction id you
            request.
          </DialogDescription>
        </DialogHeader>
        {transaction && target ? (
          <div className="min-w-0 rounded-md border bg-muted/35 p-3 text-sm">
            <p className="truncate font-medium">{transaction.txnId}</p>
            <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
              {target.url}
            </p>
          </div>
        ) : null}
        {openError ? (
          <p
            role="alert"
            className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          >
            {openError}
          </p>
        ) : null}
        <DialogFooter className="gap-2 sm:flex-wrap">
          <DialogClose asChild>
            <Button type="button" variant="outline">
              Cancel
            </Button>
          </DialogClose>
          <Button
            type="button"
            disabled={!target || opening}
            onClick={() => void openExplorer()}
          >
            <ExternalLink className="size-4" aria-hidden="true" />
            {opening ? "Opening..." : "Open explorer"}
          </Button>
        </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}
