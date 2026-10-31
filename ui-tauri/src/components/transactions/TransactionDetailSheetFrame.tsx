import * as React from "react";
import type { PropsWithChildren } from "react";
import { createPortal } from "react-dom";
import { Dialog as SheetPrimitive } from "radix-ui";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useRightPanelStore } from "@/store/rightPanel";
import { useUiStore } from "@/store/ui";

import type { Transaction } from "./model";
import {
  RightPanelDetailOwnerContext,
  TransactionDetailHostedContext,
  TransactionTrailContext,
  useTransactionTrail,
} from "./TransactionDetailTrail";

/**
 * One modal and animation lifecycle from record resolution through graph
 * loading. Inside the app shell the detail is the Details tab of the right
 * panel, beside the assistant; without one (isolated renders, AI features
 * off) it is its own sheet.
 */
export function TransactionDetailSheetFrame({
  open, onOpenChange, children, isLoading = false, onRetry, transaction = null, onOpenTransaction,
}: PropsWithChildren<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
  isLoading?: boolean;
  onRetry?: () => void;
  /** The row shown, so coins followed inside the sheet can be walked back. */
  transaction?: Transaction | null;
  onOpenTransaction?: (transactionId: string) => void;
}>) {
  const { t } = useTranslation("common");
  const trail = useTransactionTrail(transaction, open, onOpenTransaction);
  const hosted = useRightPanelStore((state) => state.hosts > 0);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  // Match the existing detail close behavior: remove the surface immediately
  // rather than replacing cleared transaction data during Radix exit presence.
  if (!open) return null;
  const body = children ? (
    <TransactionTrailContext.Provider value={trail}>{children}</TransactionTrailContext.Provider>
  ) : <div className="space-y-4 p-4 sm:p-6">
    <SheetTitle>{t("field.details")}</SheetTitle>
    <p role="status">{isLoading ? t("state.loading") : t("state.error")}</p>
    {!isLoading && onRetry && <Button onClick={onRetry}>{t("actions.retry")}</Button>}
  </div>;
  if (hosted) {
    return (
      <HostedDetail
        label={hostedDetailLabel(transaction, hideSensitive) ?? t("field.details")}
        transactionId={transaction?.id ?? null}
        onClose={() => onOpenChange(false)}
      >
        <TransactionDetailHostedContext.Provider value>
          <TooltipProvider delayDuration={150}>{body}</TooltipProvider>
        </TransactionDetailHostedContext.Provider>
      </HostedDetail>
    );
  }
  return (
    <TooltipProvider delayDuration={150}>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent
          // Wide screens get a workspace: the coins beside the tabs, not below them.
          className="w-[min(100vw,1120px)] gap-0 overflow-hidden p-0 sm:max-w-none xl:w-[min(94vw,1680px)]"
          showCloseButton={!children}
        >
          {body}
        </SheetContent>
      </Sheet>
    </TooltipProvider>
  );
}

/**
 * A tab's name for the transaction on show: its counterparty, else a short
 * transaction id. Hidden amounts hide which transaction it is, too.
 */
function hostedDetailLabel(transaction: Transaction | null, hideSensitive: boolean) {
  if (!transaction || hideSensitive) return null;
  const counterparty = transaction.counterparty?.trim();
  if (counterparty) return counterparty;
  const id = transaction.txnId || transaction.id;
  return id.length > 14 ? `${id.slice(0, 8)}…${id.slice(-4)}` : id;
}

/**
 * Registers the detail as a tab of the right panel and renders into it.
 * The detail's header names itself with the sheet's title parts, which need a
 * dialog context in this tree; an open, content-less root supplies it while
 * the panel owns the accessible name (and, as a sheet, focus and Esc).
 */
function HostedDetail({
  label,
  transactionId,
  onClose,
  children,
}: PropsWithChildren<{ label: string; transactionId: string | null; onClose: () => void }>) {
  const owner = React.useContext(RightPanelDetailOwnerContext);
  const [key, setKey] = React.useState<number | null>(null);
  const slot = useRightPanelStore((state) => (key === null ? null : state.detailSlots[key] ?? null));
  const closeRef = React.useRef(onClose);
  React.useLayoutEffect(() => {
    closeRef.current = onClose;
  });
  const close = React.useCallback(() => closeRef.current(), []);
  React.useLayoutEffect(() => {
    const panel = useRightPanelStore.getState();
    const registered = panel.showDetail({ label, close, transactionId, owner });
    setKey(registered);
    return () => {
      useRightPanelStore.getState().hideDetail(registered);
    };
    // Registered once per open; the label and transaction follow below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  React.useEffect(() => {
    if (key === null) return;
    useRightPanelStore.getState().updateDetail(key, { label, close, transactionId, owner });
  }, [key, label, close, transactionId, owner]);
  if (!slot) return null;
  return createPortal(
    <SheetPrimitive.Root open modal={false}>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">{children}</div>
    </SheetPrimitive.Root>,
    slot,
  );
}
