import {
  AlertTriangle,
  ArrowRight,
  Check,
  ChevronRight,
  Copy,
  ExternalLink,
  Info,
  ListChecks,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  copyText,
  currencyFormatter,
  formatDisplayMoney,
  formatSignedDisplayMoney,
  transactionFlow,
  type Transaction,
  type TransactionEditDraft,
} from "./model";

// ─── utilities ─────────────────────────────────────────────────────────

function arraysEqual(a: string[], b: string[]) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return false;
  return true;
}

export type DirtyMap = Partial<Record<keyof TransactionEditDraft, boolean>>;

export function diffDraft(
  current: TransactionEditDraft,
  original: TransactionEditDraft,
): DirtyMap {
  const out: DirtyMap = {};
  if (current.label !== original.label) out.label = true;
  if (!arraysEqual(current.tags, original.tags)) out.tags = true;
  if (current.note !== original.note) out.note = true;
  if (current.atRegime !== original.atRegime) out.atRegime = true;
  if (current.atCategory !== original.atCategory) out.atCategory = true;
  if (current.pricingSourceKind !== original.pricingSourceKind)
    out.pricingSourceKind = true;
  if (current.pricingQuality !== original.pricingQuality)
    out.pricingQuality = true;
  if (current.manualCurrency !== original.manualCurrency)
    out.manualCurrency = true;
  if (current.manualPrice !== original.manualPrice) out.manualPrice = true;
  if (current.manualValue !== original.manualValue) out.manualValue = true;
  if (current.manualSource !== original.manualSource) out.manualSource = true;
  if (current.reviewStatus !== original.reviewStatus) out.reviewStatus = true;
  if (current.taxable !== original.taxable) out.taxable = true;
  if (current.excluded !== original.excluded) out.excluded = true;
  return out;
}

export function countDirty(fields: DirtyMap) {
  return Object.values(fields).filter(Boolean).length;
}

export function networkLabel(transaction: Transaction): string {
  if (transaction.paymentMethod === "On-chain") return "Bitcoin · on-chain";
  if (transaction.paymentMethod === "Liquid") return "Liquid";
  if (transaction.paymentMethod === "Lightning") return "Lightning";
  return transaction.paymentMethod;
}

export function confirmationsLabel(
  conf: number | undefined,
  t?: (key: string, opts?: Record<string, unknown>) => string,
) {
  if (conf === undefined) return null;
  if (conf <= 0) return t ? t("transactions:confirmations.zero") : "0 confirmations";
  if (conf >= 6) return t ? t("transactions:confirmations.sixPlus") : "6+ conf";
  return t ? t("transactions:confirmations.count", { count: conf }) : `${conf} conf`;
}

export function formatRateAtTime(rate: number | null | undefined) {
  if (!rate) return null;
  return currencyFormatter.format(rate);
}

export function rateChangePct(now: number | null | undefined, then: number | null | undefined) {
  if (!now || !then) return null;
  return ((now - then) / then) * 100;
}

// ─── helper components ─────────────────────────────────────────────────

export function DirtyDot({ active }: { active?: boolean }) {
  const { t } = useTranslation("transactions");
  if (!active) return null;
  return (
    <span
      aria-label={t("dirty.unsavedChange")}
      title={t("dirty.unsavedChange")}
      className="inline-block size-1.5 shrink-0 rounded-full bg-amber-500"
    />
  );
}

export function InfoHint({
  children,
  label,
}: {
  children: React.ReactNode;
  label?: string;
}) {
  const { t } = useTranslation("transactions");
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="inline-flex size-4 cursor-help items-center justify-center rounded text-current opacity-60 hover:opacity-100"
          aria-label={label || t("infoHint.moreInfo")}
          tabIndex={-1}
        >
          <Info className="size-3" aria-hidden="true" />
        </button>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs text-xs leading-relaxed">
        {children}
      </TooltipContent>
    </Tooltip>
  );
}

export function DetailField({
  label,
  value,
  copyValue,
  hidden,
  dirty,
  hint,
}: {
  label: string;
  value: React.ReactNode;
  copyValue?: string;
  hidden?: boolean;
  dirty?: boolean;
  hint?: React.ReactNode;
}) {
  const { t } = useTranslation("transactions");
  return (
    <div className="min-w-0 rounded-md border bg-background p-3">
      <div className="mb-1 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate text-[10px] font-medium uppercase text-muted-foreground">
            {label}
          </span>
          {hint ? (
            <InfoHint label={t("infoHint.fieldMeaning", { label })}>{hint}</InfoHint>
          ) : null}
          <DirtyDot active={dirty} />
        </div>
        {copyValue ? (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-6 text-muted-foreground"
            aria-label={t("infoHint.copy", { label })}
            onClick={() => copyText(copyValue)}
          >
            <Copy className="size-3.5" aria-hidden="true" />
          </Button>
        ) : null}
      </div>
      <div
        className={cn(
          "min-w-0 truncate text-sm font-medium",
          hidden && "sensitive",
        )}
      >
        {value}
      </div>
    </div>
  );
}

export function LedgerRow({
  label,
  value,
  align = "left",
  muted,
  hint,
  emphasis,
}: {
  label: string;
  value: React.ReactNode;
  align?: "left" | "right";
  muted?: boolean;
  hint?: React.ReactNode;
  emphasis?: boolean;
}) {
  return (
    <div
      className={cn(
        "grid min-h-10 grid-cols-[minmax(140px,0.9fr)_minmax(0,1.1fr)] items-center gap-3 border-b px-3 py-2 last:border-b-0",
        muted && "bg-muted/50",
      )}
    >
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <span className="truncate">{label}</span>
        {hint ? <InfoHint label={label}>{hint}</InfoHint> : null}
      </div>
      <div
        className={cn(
          "min-w-0 text-sm font-medium",
          align === "right" && "text-right tabular-nums",
          emphasis && "text-base",
        )}
      >
        {value}
      </div>
    </div>
  );
}

export function SourceRecordRow({
  icon,
  label,
  value,
  copyValue,
  hidden,
  action,
}: {
  icon: React.ReactNode;
  label: string;
  value: React.ReactNode;
  copyValue?: string;
  hidden?: boolean;
  action?: {
    label: string;
    onClick: () => void;
  };
}) {
  const { t } = useTranslation("transactions");
  return (
    <div className="flex min-w-0 items-center gap-2 rounded-md border px-2 py-2">
      <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="text-[10px] font-medium uppercase text-muted-foreground">
          {label}
        </div>
        <div
          className={cn(
            "truncate text-xs font-medium text-foreground",
            hidden && "sensitive",
          )}
        >
          {value}
        </div>
      </div>
      {copyValue ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-7 shrink-0 text-muted-foreground"
          aria-label={t("infoHint.copy", { label })}
          onClick={() => copyText(copyValue)}
        >
          <Copy className="size-3.5" aria-hidden="true" />
        </Button>
      ) : null}
      {action ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-7 shrink-0 text-muted-foreground"
          aria-label={action.label}
          onClick={action.onClick}
        >
          <ExternalLink className="size-3.5" aria-hidden="true" />
        </Button>
      ) : null}
    </div>
  );
}

export function HeaderChip({
  icon,
  children,
  className,
  title,
}: {
  icon?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  title?: string;
}) {
  return (
    <Badge
      variant="outline"
      className={cn("gap-1 rounded-md", className)}
      title={title}
    >
      {icon}
      <span className="whitespace-nowrap">{children}</span>
    </Badge>
  );
}

export type TimelineStep = {
  key: string;
  label: string;
  done: boolean;
  current?: boolean;
  warn?: boolean;
  hint?: string;
};

export function StatusTimeline({ steps }: { steps: TimelineStep[] }) {
  return (
    <ol className="flex w-full flex-wrap items-center gap-x-2 gap-y-1">
      {steps.map((step, idx) => (
        <li key={step.key} className="flex min-w-0 items-center gap-1.5">
          <span
            aria-hidden="true"
            className={cn(
              "flex size-4 shrink-0 items-center justify-center rounded-full border text-[10px]",
              step.done
                ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
                : step.warn
                  ? "border-amber-500/40 bg-amber-500/15 text-amber-600 dark:text-amber-400"
                  : step.current
                    ? "border-blue-500/40 bg-blue-500/15 text-blue-600 dark:text-blue-400"
                    : "border-border bg-muted/40 text-muted-foreground",
            )}
          >
            {step.done ? (
              <Check className="size-2.5" aria-hidden="true" />
            ) : step.warn ? (
              <AlertTriangle className="size-2.5" aria-hidden="true" />
            ) : (
              idx + 1
            )}
          </span>
          <span
            className={cn(
              "whitespace-nowrap text-xs",
              step.hint && "cursor-help",
              step.done
                ? "text-foreground"
                : step.warn
                  ? "text-amber-700 dark:text-amber-300"
                  : step.current
                    ? "text-foreground"
                    : "text-muted-foreground",
            )}
            title={step.hint}
          >
            {step.label}
          </span>
          {idx < steps.length - 1 ? (
            <ChevronRight
              aria-hidden="true"
              className="size-3 shrink-0 text-muted-foreground"
            />
          ) : null}
        </li>
      ))}
    </ol>
  );
}

export function QuarantineBanner({
  title,
  reason,
  hint,
  primaryActionLabel,
  onPrimaryAction,
  onExclude,
}: {
  title: string;
  reason: string;
  hint?: React.ReactNode;
  primaryActionLabel: string;
  onPrimaryAction?: () => void;
  onExclude?: () => void;
}) {
  const { t } = useTranslation("transactions");
  return (
    <div
      role="status"
      className="flex flex-wrap items-start gap-3 rounded-md border border-l-4 border-amber-500/40 border-l-amber-500 bg-amber-50/60 px-3 py-2.5 dark:bg-amber-900/15"
    >
      <AlertTriangle
        className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-sm font-medium text-amber-700 dark:text-amber-300">
          {title}
          <InfoHint label={t("infoHint.quarantineMeaning")}>
            {hint ?? t("sheet.quarantineHint")}
          </InfoHint>
        </div>
        <div className="mt-0.5 text-xs text-amber-700/80 dark:text-amber-300/80">
          {reason}
        </div>
      </div>
      <div className="flex shrink-0 flex-wrap gap-1.5">
        {onPrimaryAction ? (
          <Button
            size="sm"
            variant="outline"
            type="button"
            onClick={onPrimaryAction}
            className="gap-1.5"
          >
            {primaryActionLabel}
            <ArrowRight className="size-3.5" aria-hidden="true" />
          </Button>
        ) : null}
        {onExclude ? (
          <Button
            size="sm"
            variant="outline"
            type="button"
            onClick={onExclude}
          >
            {t("sheet.banner.exclude")}
          </Button>
        ) : null}
      </div>
    </div>
  );
}

export type ChecklistItem = {
  key: string;
  label: string;
  done: boolean;
  hint?: string;
  warn?: boolean;
};

export function ReviewChecklist({
  items,
  onJump,
}: {
  items: Array<ChecklistItem & { tab?: string }>;
  onJump?: (tab: string) => void;
}) {
  const { t } = useTranslation("transactions");
  const completed = items.filter((i) => i.done).length;
  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <ListChecks
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
          {t("sheet.checklist.title")}
        </div>
        <span className="text-xs tabular-nums text-muted-foreground">
          {completed} / {items.length}
        </span>
      </div>
      <ul className="space-y-1.5">
        {items.map((item) => {
          const interactive = Boolean(item.tab && onJump);
          const Tag = interactive ? "button" : "div";
          return (
            <li key={item.key}>
              <Tag
                type={interactive ? "button" : undefined}
                onClick={
                  interactive && item.tab
                    ? () => onJump?.(item.tab as string)
                    : undefined
                }
                className={cn(
                  "flex w-full items-start gap-2 rounded-md text-left text-sm",
                  interactive &&
                    "group -mx-1.5 w-[calc(100%+0.75rem)] px-1.5 py-1 transition-colors hover:bg-muted/60",
                )}
                title={item.hint}
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border",
                    item.done
                      ? "border-emerald-500/40 bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
                      : item.warn
                        ? "border-amber-500/40 bg-amber-500/15 text-amber-600 dark:text-amber-400"
                        : "border-border bg-muted/40 text-muted-foreground",
                  )}
                >
                  {item.done ? (
                    <Check className="size-2.5" aria-hidden="true" />
                  ) : null}
                </span>
                <span
                  className={cn(
                    "min-w-0 flex-1",
                    item.done
                      ? "text-foreground"
                      : "text-muted-foreground",
                  )}
                >
                  {item.label}
                </span>
                {interactive ? (
                  <ChevronRight
                    aria-hidden="true"
                    className="mt-0.5 size-3.5 shrink-0 text-muted-foreground/50 transition-colors group-hover:text-foreground"
                  />
                ) : null}
              </Tag>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export type AttachmentItem = {
  id: string;
  kind: "file" | "url";
  label: string;
  detail?: string;
  href?: string;
  copiedFromAttachmentId?: string;
  copiedFromTransactionId?: string;
};

export type JournalEventItem = {
  id: string;
  entryType: string;
  asset: string;
  quantity: number;
  fiatValueEur: number;
  unitCostEur?: number | null;
  costBasisEur?: number | null;
  proceedsEur?: number | null;
  gainLossEur?: number | null;
  marketValueEur?: number | null;
  marketDeltaEur?: number | null;
  atCategory?: string | null;
  description?: string;
};

export type CommercialBtcpayRecord = {
  id: string;
  record_type: string;
  invoice_id: string;
  payment_id: string;
  order_id: string;
  status: string;
  occurred_at: string;
  asset: string;
  amount_msat: number | null;
  amount: number | null;
  payment_request_id: string;
  origin_kind: string;
  origin_app_id: string;
  origin_label: string;
  origin_url?: string;
  fiat_currency: string;
  fiat_value_exact: string;
  fiat_rate_exact: string;
  pricing_timestamp: string;
  updated_at: string;
};

export type CommercialContextLink = {
  id: string;
  invoice_id: string;
  payment_id: string;
  document_id: string;
  document_label: string;
  link_type: string;
  state: string;
  confidence: string;
  reconciliation_state: string;
  commercial_kind: string;
  reviewed_at: string;
};

export type CommercialContextDocument = {
  id: string;
  document_type: string;
  label: string;
  external_ref: string;
  review_state: string;
};

export type CommercialBtcpayMatch = {
  link: CommercialContextLink;
  payment: CommercialBtcpayRecord | null;
  invoice: CommercialBtcpayRecord | null;
  payment_request: {
    id: string;
    label: string;
    status: string;
    url?: string;
  } | null;
  origin: {
    kind: string;
    app_id: string;
    label: string;
    url?: string;
  } | null;
};

export type CommercialContextData = {
  transaction_id: string;
  transaction_external_id: string;
  links: CommercialContextLink[];
  btcpay: CommercialBtcpayMatch[];
  documents: CommercialContextDocument[];
};

// ─── money + impact helpers (unchanged) ────────────────────────────────

export function formatSheetMoney(
  eur: number | null,
  btc: number,
  currency: Currency,
  sign = false,
) {
  if (sign) return formatSignedDisplayMoney(eur, btc, currency);
  return formatDisplayMoney(
    eur === null ? null : Math.abs(eur),
    Math.abs(btc),
    currency,
  );
}

export function balanceImpactDirection(
  transaction: Transaction,
  flow: ReturnType<typeof transactionFlow>,
) {
  if (flow === "incoming") return 1;
  if (flow === "outgoing") return -1;
  if (transaction.direction === "Receive") return 1;
  if (transaction.direction === "Send") return -1;
  return 0;
}
