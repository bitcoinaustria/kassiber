import type { PropsWithChildren } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { TooltipProvider } from "@/components/ui/tooltip";

import type { Transaction } from "./model";
import { TransactionTrailContext, useTransactionTrail } from "./TransactionDetailTrail";

/** One modal and animation lifecycle from record resolution through graph loading. */
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
  // Match the existing detail close behavior: remove the surface immediately
  // rather than replacing cleared transaction data during Radix exit presence.
  if (!open) return null;
  return (
    <TooltipProvider delayDuration={150}>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent
          // Wide screens get a workspace: the coins beside the tabs, not below them.
          className="w-[min(100vw,1120px)] gap-0 overflow-hidden p-0 sm:max-w-none xl:w-[min(94vw,1680px)]"
          showCloseButton={!children}
        >
          {children ? (
            <TransactionTrailContext.Provider value={trail}>{children}</TransactionTrailContext.Provider>
          ) : <div className="space-y-4 p-4 sm:p-6">
            <SheetTitle>{t("field.details")}</SheetTitle>
            <p role="status">{isLoading ? t("state.loading") : t("state.error")}</p>
            {!isLoading && onRetry && <Button onClick={onRetry}>{t("actions.retry")}</Button>}
          </div>}
        </SheetContent>
      </Sheet>
    </TooltipProvider>
  );
}
