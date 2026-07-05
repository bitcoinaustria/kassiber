import {
  CalendarClock,
  ExternalLink,
  Network,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { MISSING_FIAT_LABEL } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  blurClass,
  currencyFormatter,
  formatBtcAmount,
  transactionFlowLabels,
  transactionFlowStyles,
  transactionStatusIcons,
  transactionStatusLabels,
  transactionStatusStyles,
  type Transaction,
  type TransactionEditDraft,
  type TransactionFlow,
} from "./model";
import {
  HeaderChip,
  StatusTimeline,
  networkLabel,
  type TimelineStep,
} from "./TransactionDetailSheetParts";

type ExplorerSummary = {
  label: string;
} | null;

export function TransactionDetailHeader({
  transaction,
  flow,
  reviewStatus,
  pair,
  signedPrefix,
  hideSensitive,
  amountBtc,
  valueAtTimeEur,
  valueNowEur,
  pricedChange,
  timelineSteps,
  explorer,
  onOpenExplorer,
  onClose,
}: {
  transaction: Transaction;
  flow: TransactionFlow;
  reviewStatus: TransactionEditDraft["reviewStatus"];
  pair?: Transaction["pair"];
  signedPrefix: string;
  hideSensitive: boolean;
  amountBtc: number;
  valueAtTimeEur: number | null;
  valueNowEur: number | null;
  pricedChange: number | null;
  timelineSteps: TimelineStep[];
  explorer: ExplorerSummary;
  onOpenExplorer: (transaction: Transaction) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation(["transactions"]);
  const StatusIcon = transactionStatusIcons[reviewStatus];
  const confirmations = transaction.confirmations ?? 0;
  const showReviewStatusChip =
    reviewStatus !== "completed" &&
    !(reviewStatus === "pending" && confirmations <= 0);

  return (
    <SheetHeader className="border-b p-0">
      <div className="flex items-start justify-between gap-4 px-4 pt-5 pb-4 sm:px-6 sm:pt-6">
        <div className="min-w-0">
          <div className="mb-2 flex flex-wrap items-center gap-1.5">
            <HeaderChip className={transactionFlowStyles[flow]}>
              {t(transactionFlowLabels[flow])}
            </HeaderChip>
            <HeaderChip
              icon={
                <Network
                  className="size-3 text-muted-foreground"
                  aria-hidden="true"
                />
              }
            >
              {networkLabel(transaction)}
            </HeaderChip>
            {showReviewStatusChip ? (
              <HeaderChip
                icon={
                  <StatusIcon className="size-3" aria-hidden="true" />
                }
                className={transactionStatusStyles[reviewStatus]}
              >
                {t(transactionStatusLabels[reviewStatus])}
              </HeaderChip>
            ) : null}
          </div>
          <SheetTitle className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-2xl tabular-nums sm:text-3xl">
            <span className="truncate">
              {signedPrefix}
              <span className={blurClass(hideSensitive)}>
                {formatBtcAmount(amountBtc)}
              </span>
            </span>
            {valueAtTimeEur !== null ? (
              <span className="text-sm font-medium text-muted-foreground sm:text-base">
                ≈{" "}
                <span className={blurClass(hideSensitive)}>
                  {currencyFormatter.format(Math.abs(valueAtTimeEur))}
                </span>{" "}
                {t("detailHeader.valueThen")}
                {valueNowEur !== null ? (
                  <>
                    {" "}
                    ·{" "}
                    <span className={blurClass(hideSensitive)}>
                      {currencyFormatter.format(Math.abs(valueNowEur))}
                    </span>{" "}
                    {t("detailHeader.valueNow")}
                    {pricedChange !== null ? (
                      <span
                        className={cn(
                          "ml-1 tabular-nums",
                          pricedChange >= 0
                            ? "text-emerald-600 dark:text-emerald-400"
                            : "text-red-600 dark:text-red-400",
                        )}
                      >
                        ({pricedChange >= 0 ? "+" : ""}
                        {pricedChange.toFixed(1)}%)
                      </span>
                    ) : null}
                  </>
                ) : null}
              </span>
            ) : (
              <span className="text-sm font-medium text-amber-600 dark:text-amber-400 sm:text-base">
                {MISSING_FIAT_LABEL}
              </span>
            )}
          </SheetTitle>
          <SheetDescription className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-sm">
            {transaction.counterparty ? (
              <>
                <span className="font-medium text-foreground">
                  {transaction.counterparty}
                </span>
                <span className="text-muted-foreground">·</span>
              </>
            ) : null}
            {!pair ? (
              <>
                <span className="text-muted-foreground">
                  {transaction.wallet ?? t("detailHeader.unassignedWallet")}
                </span>
                <span className="text-muted-foreground">·</span>
              </>
            ) : null}
            <span className="inline-flex items-center gap-1 text-muted-foreground">
              <CalendarClock className="size-3.5" aria-hidden="true" />
              {transaction.date}
            </span>
          </SheetDescription>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {explorer ? (
            <Button
              type="button"
              variant="outline"
              size="icon"
              aria-label={t("detailHeader.openExplorer")}
              onClick={() => onOpenExplorer(transaction)}
            >
              <ExternalLink className="size-4" aria-hidden="true" />
            </Button>
          ) : null}
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label={t("detailHeader.close")}
            onClick={onClose}
          >
            <X className="size-4" aria-hidden="true" />
          </Button>
        </div>
      </div>
      <div className="border-t bg-muted/50 px-4 py-2 sm:px-6">
        <StatusTimeline steps={timelineSteps} />
      </div>
    </SheetHeader>
  );
}
