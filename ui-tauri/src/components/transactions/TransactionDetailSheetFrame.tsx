import type { PropsWithChildren } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { TooltipProvider } from "@/components/ui/tooltip";

/** One modal and animation lifecycle from record resolution through graph loading. */
export function TransactionDetailSheetFrame({
  open, onOpenChange, children, isLoading = false, onRetry,
}: PropsWithChildren<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
  isLoading?: boolean;
  onRetry?: () => void;
}>) {
  const { t } = useTranslation("common");
  // Match the existing detail close behavior: remove the surface immediately
  // rather than replacing cleared transaction data during Radix exit presence.
  if (!open) return null;
  return (
    <TooltipProvider delayDuration={150}>
      <Sheet open={open} onOpenChange={onOpenChange}>
        <SheetContent
          className="w-[min(100vw,1120px)] gap-0 overflow-hidden p-0 sm:max-w-none"
          showCloseButton={!children}
        >
          {children ?? <div className="space-y-4 p-4 sm:p-6">
            <SheetTitle>{t("field.details")}</SheetTitle>
            <p role="status">{isLoading ? t("state.loading") : t("state.error")}</p>
            {!isLoading && onRetry && <Button onClick={onRetry}>{t("actions.retry")}</Button>}
          </div>}
        </SheetContent>
      </Sheet>
    </TooltipProvider>
  );
}
