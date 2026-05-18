import {
  AlertTriangle,
  ArrowRight,
  BookMarked,
  CalendarClock,
  Check,
  ChevronRight,
  Copy,
  ExternalLink,
  FileText,
  Hash,
  History,
  Info,
  Layers,
  Link2,
  ListChecks,
  Network,
  Paperclip,
  Plus,
  Repeat2,
  RotateCcw,
  Save,
  Tags,
  X,
} from "lucide-react";
import * as React from "react";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Switch } from "@/components/ui/switch";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { MISSING_FIAT_LABEL, type Currency } from "@/lib/currency";
import { isFilePickerAvailable, pickFiles } from "@/lib/filePicker";
import { cn } from "@/lib/utils";
import type { ExplorerSettings } from "@/lib/explorer";

import {
  allTransactionStatuses,
  austrianSelectionValue,
  austrianTaxClassificationFor,
  austrianTaxClassificationForValue,
  austrianTaxClassificationOptions,
  blurClass,
  classificationOptions,
  copyText,
  currencyFormatter,
  explorerForTransaction,
  formatBtcAmount,
  formatDisplayMoney,
  formatFee,
  formatManualFiat,
  formatManualPrice,
  formatShortTxid,
  formatSignedDisplayMoney,
  parseManualDecimal,
  pricingSelectionValue,
  pricingSourceLabel,
  tagSuggestions,
  transactionBtc,
  transactionFlow,
  transactionFlowLabels,
  transactionFlowStyles,
  transactionPricingOptions,
  transactionStatusIcons,
  transactionStatusLabels,
  transactionStatusStyles,
  type Transaction,
  type TransactionEditDraft,
  type TransactionStatus,
  uniqueTags,
} from "./model";

// ─── utilities ─────────────────────────────────────────────────────────

function arraysEqual(a: string[], b: string[]) {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) if (a[i] !== b[i]) return false;
  return true;
}

type DirtyMap = Partial<Record<keyof TransactionEditDraft, boolean>>;

function diffDraft(
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

function countDirty(fields: DirtyMap) {
  return Object.values(fields).filter(Boolean).length;
}

function networkLabel(transaction: Transaction): string {
  if (transaction.paymentMethod === "On-chain") return "Bitcoin · on-chain";
  if (transaction.paymentMethod === "Liquid") return "Liquid";
  if (transaction.paymentMethod === "Lightning") return "Lightning";
  return transaction.paymentMethod;
}

function confirmationsLabel(conf: number | undefined) {
  if (conf === undefined) return null;
  if (conf <= 0) return "0 confirmations";
  if (conf >= 6) return "6+ conf";
  return `${conf} conf`;
}

function formatRateAtTime(rate: number | null | undefined) {
  if (!rate) return null;
  return currencyFormatter.format(rate);
}

function rateChangePct(now: number | null | undefined, then: number | null | undefined) {
  if (!now || !then) return null;
  return ((now - then) / then) * 100;
}

// ─── helper components ─────────────────────────────────────────────────

function DirtyDot({ active }: { active?: boolean }) {
  if (!active) return null;
  return (
    <span
      aria-label="Unsaved change"
      title="Unsaved change"
      className="inline-block size-1.5 shrink-0 rounded-full bg-amber-500"
    />
  );
}

function InfoHint({
  children,
  label,
}: {
  children: React.ReactNode;
  label?: string;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="inline-flex size-4 items-center justify-center rounded text-current opacity-60 hover:opacity-100"
          aria-label={label || "More info"}
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

function DetailField({
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
  return (
    <div className="min-w-0 rounded-md border bg-background p-3">
      <div className="mb-1 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate text-[10px] font-medium uppercase text-muted-foreground">
            {label}
          </span>
          {hint ? (
            <InfoHint label={`What does "${label}" mean?`}>{hint}</InfoHint>
          ) : null}
          <DirtyDot active={dirty} />
        </div>
        {copyValue ? (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-6 text-muted-foreground"
            aria-label={`Copy ${label}`}
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

function LedgerRow({
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

function SourceRecordRow({
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
          aria-label={`Copy ${label}`}
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

function HeaderChip({
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

type TimelineStep = {
  key: string;
  label: string;
  done: boolean;
  current?: boolean;
  hint?: string;
};

function StatusTimeline({ steps }: { steps: TimelineStep[] }) {
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
                : step.current
                  ? "border-blue-500/40 bg-blue-500/15 text-blue-600 dark:text-blue-400"
                  : "border-border bg-muted/40 text-muted-foreground",
            )}
          >
            {step.done ? (
              <Check className="size-2.5" aria-hidden="true" />
            ) : (
              idx + 1
            )}
          </span>
          <span
            className={cn(
              "whitespace-nowrap text-xs",
              step.done
                ? "text-foreground"
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

function QuarantineBanner({
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
          <InfoHint label="What does quarantined mean?">
            {hint ??
              "Journal-quarantined transactions are skipped during journal processing and don't affect tax reports until the blocker is resolved."}
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
            Exclude
          </Button>
        ) : null}
      </div>
    </div>
  );
}

type ChecklistItem = {
  key: string;
  label: string;
  done: boolean;
  hint?: string;
  warn?: boolean;
};

function ReviewChecklist({
  items,
  onJump,
}: {
  items: Array<ChecklistItem & { tab?: string }>;
  onJump?: (tab: string) => void;
}) {
  const completed = items.filter((i) => i.done).length;
  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-3 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <ListChecks
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
          Review checklist
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
                  "flex w-full items-start gap-2 rounded text-left text-sm",
                  interactive && "hover:text-foreground",
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
};

function AttachLinksDialog({
  open,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (urls: string[]) => void | Promise<void>;
}) {
  const [text, setText] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setText("");
      setError(null);
      setSubmitting(false);
    }
  }, [open]);

  const parsed = React.useMemo(() => {
    const lines = text
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter(Boolean);
    const errors: string[] = [];
    const urls: string[] = [];
    for (const line of lines) {
      try {
        const u = new URL(line);
        if (
          u.protocol !== "http:" &&
          u.protocol !== "https:" &&
          u.protocol !== "ipfs:"
        ) {
          errors.push(`${line} — must start with http://, https://, or ipfs://`);
          continue;
        }
        urls.push(line);
      } catch {
        errors.push(`${line} — not a valid URL`);
      }
    }
    return { urls, errors };
  }, [text]);

  const submit = async () => {
    if (!parsed.urls.length) {
      setError("Add at least one URL.");
      return;
    }
    if (parsed.errors.length) {
      setError(parsed.errors[0]);
      return;
    }
    setSubmitting(true);
    try {
      await onSubmit(parsed.urls);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add links.");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Attach links</DialogTitle>
          <DialogDescription>
            One URL per line. Links are stored as references — Kassiber does
            not fetch or index the content.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="attach-links-text">URLs</Label>
          <Textarea
            id="attach-links-text"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setError(null);
            }}
            className="min-h-32 font-mono text-xs"
            placeholder={"https://btcpay.example/invoice/abc123\nhttps://drive.example/receipt-2026-04.pdf"}
            autoFocus
            aria-describedby="attach-links-status"
            aria-invalid={parsed.errors.length > 0}
          />
          <div className="flex items-center justify-between gap-2 text-xs">
            <span
              id="attach-links-status"
              role="status"
              aria-live="polite"
              className="text-muted-foreground"
            >
              {parsed.urls.length === 0
                ? "No URLs yet"
                : `${parsed.urls.length} URL${parsed.urls.length === 1 ? "" : "s"} ready`}
              {parsed.errors.length ? (
                <span className="ml-1 text-amber-600 dark:text-amber-400">
                  · {parsed.errors.length} invalid line
                  {parsed.errors.length === 1 ? "" : "s"} to fix
                </span>
              ) : null}
            </span>
            {error ? (
              <span role="alert" className="text-destructive">
                {error}
              </span>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={submitting}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={
              submitting ||
              parsed.urls.length === 0 ||
              parsed.errors.length > 0
            }
            onClick={submit}
          >
            {submitting
              ? "Attaching"
              : parsed.urls.length > 1
                ? `Attach ${parsed.urls.length} links`
                : "Attach link"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AttachmentsPanel({
  items = [],
  hideSensitive,
  onAddFiles,
  onAddLinks,
  onOpen,
  onRemove,
}: {
  items?: AttachmentItem[];
  hideSensitive: boolean;
  onAddFiles?: (paths: string[]) => void | Promise<void>;
  onAddLinks?: (urls: string[]) => void | Promise<void>;
  onOpen?: (item: AttachmentItem) => void;
  onRemove?: (item: AttachmentItem) => void;
}) {
  const [linkDialogOpen, setLinkDialogOpen] = React.useState(false);
  const [pickerBusy, setPickerBusy] = React.useState(false);
  const wired = Boolean(onAddFiles || onAddLinks);
  const filePickerEnabled = Boolean(onAddFiles) && isFilePickerAvailable;

  const [pickerError, setPickerError] = React.useState<string | null>(null);
  const handlePickFiles = async () => {
    if (!onAddFiles) return;
    setPickerError(null);
    setPickerBusy(true);
    try {
      const paths = await pickFiles({
        title: "Attach files to this transaction",
      });
      if (paths.length) {
        await onAddFiles(paths);
      }
    } catch (err) {
      setPickerError(
        err instanceof Error ? err.message : "Could not open file picker.",
      );
    } finally {
      setPickerBusy(false);
    }
  };

  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <Paperclip
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
          Attachments
        </div>
        <span className="text-xs tabular-nums text-muted-foreground">
          {items.length}
        </span>
      </div>
      {items.length ? (
        <ul className="mb-2 space-y-1.5">
          {items.map((item) => {
            const hiddenTitle = hideSensitive
              ? "Attachment detail hidden"
              : item.detail;
            return (
              <li
                key={item.id}
                className="flex min-w-0 items-center gap-2 rounded-md border bg-background px-2 py-1.5"
              >
                <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                  {item.kind === "url" ? (
                    <Link2 className="size-3.5" aria-hidden="true" />
                  ) : (
                    <FileText className="size-3.5" aria-hidden="true" />
                  )}
                </span>
                {onOpen ? (
                  <button
                    type="button"
                    onClick={() => onOpen(item)}
                    className="min-w-0 flex-1 text-left hover:underline"
                    title={hiddenTitle}
                  >
                    <div
                      className={cn(
                        "truncate text-xs font-medium",
                        blurClass(hideSensitive),
                      )}
                    >
                      {item.label}
                    </div>
                    {item.detail ? (
                      <div
                        className={cn(
                          "truncate text-[10px] text-muted-foreground",
                          blurClass(hideSensitive),
                        )}
                      >
                        {item.detail}
                      </div>
                    ) : null}
                  </button>
                ) : (
                  <div className="min-w-0 flex-1" title={hiddenTitle}>
                    <div
                      className={cn(
                        "truncate text-xs font-medium",
                        blurClass(hideSensitive),
                      )}
                    >
                      {item.label}
                    </div>
                    {item.detail ? (
                      <div
                        className={cn(
                          "truncate text-[10px] text-muted-foreground",
                          blurClass(hideSensitive),
                        )}
                      >
                        {item.detail}
                      </div>
                    ) : null}
                  </div>
                )}
                {onRemove ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-6 shrink-0 text-muted-foreground"
                    aria-label={
                      hideSensitive ? "Remove attachment" : `Remove ${item.label}`
                    }
                    onClick={() => onRemove(item)}
                  >
                    <X className="size-3.5" aria-hidden="true" />
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="mb-2 text-xs text-muted-foreground">
          Attach receipts, invoices, or accountant notes. Add as many as you
          need — local files are copied into the attachments folder, links
          stay as references.
        </p>
      )}
      {pickerError ? (
        <p
          role="alert"
          className="mb-2 text-xs text-destructive"
        >
          {pickerError}
        </p>
      ) : null}
      <div className="grid grid-cols-2 gap-1.5">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!wired || !filePickerEnabled || pickerBusy}
          title={
            !filePickerEnabled && wired
              ? "Native file picker not available in this runtime"
              : undefined
          }
          onClick={handlePickFiles}
        >
          <FileText className="size-3.5" aria-hidden="true" />
          {pickerBusy ? "Picking…" : "Attach files"}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!onAddLinks}
          onClick={() => setLinkDialogOpen(true)}
        >
          <Link2 className="size-3.5" aria-hidden="true" />
          Attach links
        </Button>
      </div>
      {onAddLinks ? (
        <AttachLinksDialog
          open={linkDialogOpen}
          onOpenChange={setLinkDialogOpen}
          onSubmit={onAddLinks}
        />
      ) : null}
    </div>
  );
}

// ─── money + impact helpers (unchanged) ────────────────────────────────

function formatSheetMoney(
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

function balanceImpactDirection(
  transaction: Transaction,
  flow: ReturnType<typeof transactionFlow>,
) {
  if (flow === "incoming") return 1;
  if (flow === "outgoing") return -1;
  if (transaction.direction === "Receive") return 1;
  if (transaction.direction === "Send") return -1;
  return 0;
}

// ─── main component ────────────────────────────────────────────────────

export function TransactionDetailSheet({
  transaction,
  draft,
  initialTab,
  hideSensitive,
  currency,
  explorerSettings,
  isSaving,
  saveError,
  nowRate,
  attachments,
  onAddAttachmentFiles,
  onAddAttachmentLinks,
  onOpenAttachment,
  onRemoveAttachment,
  onOpenChange,
  onOpenExplorer,
  onSave,
  onSaveAndNext,
  hasNext,
}: {
  transaction: Transaction | null;
  draft: TransactionEditDraft | null;
  initialTab: string;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  isSaving?: boolean;
  saveError?: string | null;
  nowRate?: number | null;
  attachments?: AttachmentItem[];
  onAddAttachmentFiles?: (paths: string[]) => void | Promise<void>;
  onAddAttachmentLinks?: (urls: string[]) => void | Promise<void>;
  onOpenAttachment?: (item: AttachmentItem) => void;
  onRemoveAttachment?: (item: AttachmentItem) => void;
  onOpenChange: (open: boolean) => void;
  onOpenExplorer: (transaction: Transaction) => void;
  onSave: (
    transactionId: string,
    draft: TransactionEditDraft,
  ) => void | Promise<void>;
  onSaveAndNext?: (
    transactionId: string,
    draft: TransactionEditDraft,
  ) => void | Promise<void>;
  hasNext?: boolean;
}) {
  const [activeTab, setActiveTab] = React.useState(initialTab);
  const [localDraft, setLocalDraft] =
    React.useState<TransactionEditDraft | null>(draft);
  const [originalDraft, setOriginalDraft] =
    React.useState<TransactionEditDraft | null>(draft);
  const [tagInput, setTagInput] = React.useState("");
  const [balanceCurrency, setBalanceCurrency] =
    React.useState<Currency>(currency);
  const manualPriceRef = React.useRef<HTMLInputElement | null>(null);

  React.useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab, transaction?.id]);

  React.useEffect(() => {
    setLocalDraft(draft);
    setOriginalDraft(draft);
    setTagInput("");
    setBalanceCurrency(currency);
  }, [currency, draft, transaction?.id]);

  const tagInputRef = React.useRef<HTMLInputElement | null>(null);
  const dirty = React.useMemo(
    () => (localDraft && originalDraft ? diffDraft(localDraft, originalDraft) : {}),
    [localDraft, originalDraft],
  );
  const dirtyCount = countDirty(dirty);

  const updateDraft = React.useCallback(
    <K extends keyof TransactionEditDraft>(
      key: K,
      value: TransactionEditDraft[K],
    ) => {
      setLocalDraft((current) =>
        current ? { ...current, [key]: value } : current,
      );
    },
    [],
  );

  // Keyboard shortcuts: 1-6 tabs, Cmd/Ctrl+S save, Esc close, e excluded, t focus tag.
  // Suppress shortcuts while focus is inside another modal (e.g. AttachLinksDialog)
  // so dialog keys don't reach back into the underlying sheet.
  React.useEffect(() => {
    if (!transaction || !localDraft) return;
    const handler = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const isTyping =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;
      const insideNestedDialog = Boolean(
        target?.closest('[data-slot="dialog-content"]'),
      );
      if (insideNestedDialog) return;
      if ((event.metaKey || event.ctrlKey) && event.key === "s") {
        event.preventDefault();
        if (dirtyCount > 0 && !isSaving) {
          void onSave(transaction.id, localDraft);
        }
        return;
      }
      if (event.key === "Escape" && !isTyping) {
        onOpenChange(false);
        return;
      }
      if (isTyping) return;
      if (["1", "2", "3", "4", "5", "6"].includes(event.key)) {
        const order = ["details", "classify", "pricing", "tax", "linked", "ledger"];
        const next = order[Number(event.key) - 1];
        if (next) setActiveTab(next);
        return;
      }
      if (event.key === "e") {
        updateDraft("excluded", !localDraft.excluded);
        return;
      }
      if (event.key === "t") {
        event.preventDefault();
        setActiveTab("classify");
        setTimeout(() => tagInputRef.current?.focus(), 0);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [transaction, localDraft, dirtyCount, isSaving, onOpenChange, onSave, updateDraft]);

  if (!transaction || !localDraft) return null;

  const StatusIcon = transactionStatusIcons[localDraft.reviewStatus];
  const flow = transactionFlow(transaction);
  const explorer = explorerForTransaction(transaction, explorerSettings);
  const amountBtc = transactionBtc(transaction);
  const feeBtc = transaction.feeBtc ?? 0;
  const feeEur = transaction.feeEur ?? null;
  const impactDirection = balanceImpactDirection(transaction, flow);
  const principalImpactBtc = impactDirection * amountBtc;
  const principalImpactEur =
    transaction.amount === null ? null : impactDirection * transaction.amount;
  const feeImpactBtc = feeBtc ? -feeBtc : 0;
  const feeImpactEur = feeBtc ? (feeEur === null ? null : -feeEur) : 0;
  const netImpactBtc = principalImpactBtc + feeImpactBtc;
  const netImpactEur =
    principalImpactEur === null || feeImpactEur === null
      ? null
      : principalImpactEur + feeImpactEur;
  const pair = transaction.pair;
  const signedPrefix =
    flow === "incoming" ? "+" : flow === "outgoing" ? "-" : "";
  const tags = localDraft.tags;
  const taxClassification = austrianTaxClassificationFor(
    localDraft.atRegime,
    localDraft.atCategory,
  );
  const pricingValue = pricingSelectionValue(
    localDraft.pricingSourceKind,
    localDraft.pricingQuality,
  );
  const sourceRecordId = transaction.explorerId ?? transaction.txnId;
  const sourceName = transaction.wallet || transaction.paymentMethod;
  const sourceType = transaction.sourceType ?? transaction.paymentMethod;
  const settlementLabel = transactionStatusLabels[transaction.status];
  const valueAtTimeEur = transaction.amount;
  const valueNowEur =
    nowRate && amountBtc ? nowRate * amountBtc * (impactDirection || 1) : null;
  const pricedChange = rateChangePct(nowRate ?? null, transaction.rate ?? null);
  const isPricingMissing =
    localDraft.pricingSourceKind === null ||
    localDraft.pricingQuality === "missing" ||
    transaction.amount === null;
  const isLabeled =
    localDraft.label !== "Unlabeled" && localDraft.label.trim().length > 0;
  const quarantineReason = transaction.quarantineReason ?? null;
  const hasJournalQuarantine = Boolean(quarantineReason) && !localDraft.excluded;
  const hasPricingBlocker = isPricingMissing && !localDraft.excluded;
  const showReviewBanner = hasJournalQuarantine || hasPricingBlocker;
  const confLabel = confirmationsLabel(transaction.confirmations);
  const dirtyTags = dirty.tags;
  const dirtyLabel = dirty.label;
  const dirtyNote = dirty.note;
  const dirtyExcluded = dirty.excluded;

  const timelineSteps: TimelineStep[] = [
    {
      key: "imported",
      label: "Imported",
      done: true,
      hint: "Transaction read from a wallet, exchange, or import file.",
    },
    {
      key: "settled",
      label: settlementLabel,
      done: transaction.status === "completed",
      current: transaction.status === "pending",
      hint:
        transaction.status === "completed"
          ? "On-chain settled."
          : transaction.status === "pending"
            ? "Waiting for confirmation."
            : "Settlement issue — check details.",
    },
    {
      key: "reviewed",
      label: localDraft.reviewStatus === "review" ? "Needs review" : "Reviewed",
      done: localDraft.reviewStatus !== "review",
      hint: "Marked off the review queue.",
    },
    {
      key: "journaled",
      label: localDraft.excluded
        ? "Excluded"
        : hasJournalQuarantine
          ? "Quarantined"
          : isPricingMissing
          ? "Pending journal"
          : "Journaled",
      done: !localDraft.excluded && !hasJournalQuarantine && !isPricingMissing,
      hint:
        "Included in the RP2 journal once pricing is set and the tx isn't excluded.",
    },
  ];

  const reviewChecklistItems: Array<ChecklistItem & { tab?: string }> = [
    {
      key: "pricing",
      label: isPricingMissing
        ? "Set a pricing source"
        : `Priced via ${pricingSourceLabel(
            localDraft.pricingSourceKind,
            localDraft.pricingQuality,
          )}`,
      done: !isPricingMissing,
      warn: isPricingMissing,
      tab: "pricing",
    },
    {
      key: "classified",
      label: isLabeled ? `Labeled ${localDraft.label}` : "Pick a label",
      done: isLabeled,
      tab: "classify",
    },
    {
      key: "tax",
      label: localDraft.excluded
        ? "Excluded from tax reports"
        : `Tax: ${taxClassification.shortLabel}`,
      done: true,
      tab: "tax",
    },
    {
      key: "quarantine",
      label: hasJournalQuarantine
        ? "Resolve quarantine to include in reports"
        : hasPricingBlocker
          ? "Pricing incomplete"
          : "No quarantine",
      done: !hasJournalQuarantine && !hasPricingBlocker,
      warn: hasJournalQuarantine || hasPricingBlocker,
      tab: "pricing",
    },
  ];

  const addTag = (rawTag: string) => {
    const tag = rawTag.trim();
    if (!tag) return;
    updateDraft("tags", uniqueTags([...localDraft.tags, tag]));
    setTagInput("");
  };
  const removeTag = (tag: string) => {
    updateDraft(
      "tags",
      localDraft.tags.filter((candidate) => candidate !== tag),
    );
  };
  const availableTagSuggestions = tagSuggestions.filter(
    (suggestion) => !localDraft.tags.includes(suggestion),
  );
  const updateManualPrice = (rawPrice: string) => {
    const parsedPrice = parseManualDecimal(rawPrice);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            pricingSourceKind: "manual_override",
            pricingQuality: "exact",
            manualPrice: rawPrice,
            manualValue:
              parsedPrice !== null && amountBtc > 0
                ? formatManualFiat(parsedPrice * amountBtc)
                : "",
          }
        : current,
    );
  };
  const updateManualValue = (rawValue: string) => {
    const parsedValue = parseManualDecimal(rawValue);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            pricingSourceKind: "manual_override",
            pricingQuality: "exact",
            manualValue: rawValue,
            manualPrice:
              parsedValue !== null && amountBtc > 0
                ? formatManualPrice(parsedValue / amountBtc)
                : "",
          }
        : current,
    );
  };

  const jumpToManualPrice = () => {
    setActiveTab("pricing");
    setTimeout(() => manualPriceRef.current?.focus(), 0);
  };
  const setExcluded = () => updateDraft("excluded", true);
  const normalizedQuarantineReason = quarantineReason
    ? quarantineReason.replace(/_/g, " ")
    : null;

  const taxNarrative = (() => {
    const action =
      flow === "incoming"
        ? "received"
        : flow === "outgoing"
          ? "sent"
          : "moved";
    const counterparty = transaction.counterparty || "the counterparty";
    const at = transaction.date;
    const fiat = valueAtTimeEur
      ? `worth ${currencyFormatter.format(valueAtTimeEur)} at the time`
      : "with no fiat price recorded yet";
    const treatment = localDraft.excluded
      ? "It is excluded from journal processing"
      : `It is currently treated as ${taxClassification.label}`;
    return `You ${action} ${formatBtcAmount(amountBtc)} ${flow === "outgoing" ? "to" : "from"} ${counterparty} on ${at}, ${fiat}. ${treatment}.`;
  })();

  return (
    <TooltipProvider delayDuration={150}>
      <Sheet open={Boolean(transaction)} onOpenChange={onOpenChange}>
        <SheetContent
          className="w-[min(100vw,1120px)] overflow-hidden p-0 sm:max-w-none"
          showCloseButton={false}
        >
          <SheetHeader className="border-b p-0">
            <div className="flex items-start justify-between gap-4 px-4 pt-5 pb-4 sm:px-6 sm:pt-6">
              <div className="min-w-0">
                <div className="mb-2 flex flex-wrap items-center gap-1.5">
                  <HeaderChip
                    className={transactionFlowStyles[flow]}
                  >
                    {transactionFlowLabels[flow]}
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
                  {confLabel ? (
                    <HeaderChip
                      title={`${transaction.confirmations} on-chain confirmations`}
                    >
                      {confLabel}
                    </HeaderChip>
                  ) : null}
                  {localDraft.reviewStatus !== "completed" ? (
                    <HeaderChip
                      icon={
                        <StatusIcon className="size-3" aria-hidden="true" />
                      }
                      className={transactionStatusStyles[localDraft.reviewStatus]}
                    >
                      {transactionStatusLabels[localDraft.reviewStatus]}
                    </HeaderChip>
                  ) : null}
                  {pair ? (
                    <HeaderChip
                      icon={
                        <Repeat2
                          className="size-3 text-sky-500"
                          aria-hidden="true"
                        />
                      }
                      className="border-sky-500/30 text-sky-700 dark:text-sky-300"
                    >
                      Paired ·{" "}
                      {pair.outWallet ?? "Out"} → {pair.inWallet ?? "In"}
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
                      then
                      {valueNowEur !== null ? (
                        <>
                          {" "}
                          ·{" "}
                          <span className={blurClass(hideSensitive)}>
                            {currencyFormatter.format(Math.abs(valueNowEur))}
                          </span>{" "}
                          now
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
                  <span className="font-medium text-foreground">
                    {transaction.counterparty}
                  </span>
                  <span className="text-muted-foreground">·</span>
                  <span className="text-muted-foreground">
                    {transaction.wallet ?? "Unassigned wallet"}
                  </span>
                  <span className="text-muted-foreground">·</span>
                  <span className="inline-flex items-center gap-1 text-muted-foreground">
                    <CalendarClock
                      className="size-3.5"
                      aria-hidden="true"
                    />
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
                    aria-label="Open explorer"
                    onClick={() => onOpenExplorer(transaction)}
                  >
                    <ExternalLink className="size-4" aria-hidden="true" />
                  </Button>
                ) : null}
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  aria-label="Close transaction detail"
                  onClick={() => onOpenChange(false)}
                >
                  <X className="size-4" aria-hidden="true" />
                </Button>
              </div>
            </div>
            <div className="border-t bg-muted/50 px-4 py-2 sm:px-6">
              <StatusTimeline steps={timelineSteps} />
            </div>
          </SheetHeader>

          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className="grid gap-4 p-4 sm:p-6 xl:grid-cols-[minmax(0,1fr)_320px]">
              <div className="min-w-0 space-y-4">
                {showReviewBanner ? (
                  <QuarantineBanner
                    title={
                      hasJournalQuarantine
                        ? "Journal quarantine"
                        : transaction.amount === null
                        ? "Missing fiat price"
                        : localDraft.pricingSourceKind === null
                          ? "No pricing source"
                          : "Pricing flagged for review"
                    }
                    reason={
                      hasJournalQuarantine
                        ? `Current journal blocker: ${normalizedQuarantineReason}.`
                        : transaction.amount === null
                          ? `No fiat price recorded for ${transaction.date}. Pricing edits are preview-only until the metadata daemon kind is extended.`
                          : localDraft.pricingSourceKind === null
                            ? "No persisted pricing source is available yet. Pricing edits are preview-only until the metadata daemon kind is extended."
                            : "Pricing source is marked as missing or under review; editing it is deferred to the metadata daemon follow-up."
                    }
                    hint={
                      hasJournalQuarantine
                        ? undefined
                        : "This is a pricing readiness warning from the transaction row. It is not an active journal quarantine unless the daemon returns a quarantine reason."
                    }
                    primaryActionLabel={
                      hasJournalQuarantine ? "View Pricing" : "Open Pricing"
                    }
                    onPrimaryAction={jumpToManualPrice}
                    onExclude={setExcluded}
                  />
                ) : null}

                <Tabs value={activeTab} onValueChange={setActiveTab}>
                  <TabsList className="grid w-full grid-cols-6">
                    <TabsTrigger value="details">Details</TabsTrigger>
                    <TabsTrigger value="classify">
                      Classify
                      {dirtyLabel || dirtyTags || dirtyNote ? (
                        <DirtyDot active />
                      ) : null}
                    </TabsTrigger>
                    <TabsTrigger value="pricing">Pricing</TabsTrigger>
                    <TabsTrigger value="tax">
                      Tax
                      {dirtyExcluded ? <DirtyDot active /> : null}
                    </TabsTrigger>
                    <TabsTrigger value="linked">Linked</TabsTrigger>
                    <TabsTrigger value="ledger">Ledger</TabsTrigger>
                  </TabsList>

                  {/* Details — read-only source-of-record + book metadata */}
                  <TabsContent value="details" className="mt-4 space-y-4">
                    <div className="grid gap-3 sm:grid-cols-3">
                      <DetailField
                        label="Transaction ID"
                        value={formatShortTxid(
                          transaction.explorerId ?? transaction.txnId,
                        )}
                        copyValue={transaction.explorerId ?? transaction.txnId}
                        hidden={hideSensitive}
                        hint="Canonical on-chain identifier or import row id, depending on the source."
                      />
                      <DetailField
                        label="Price at time"
                        value={
                          localDraft.pricingSourceKind === "manual_override" &&
                          localDraft.manualPrice
                            ? `${localDraft.manualPrice} ${localDraft.manualCurrency}/BTC`
                            : transaction.rate
                              ? `${currencyFormatter.format(transaction.rate)} / BTC`
                              : "Missing"
                        }
                        hidden={hideSensitive}
                        hint="BTC/fiat rate used to value this tx at the time it occurred."
                      />
                      <DetailField
                        label="Fee"
                        value={
                          feeBtc ? (
                            <CurrencyToggleText
                              className={blurClass(hideSensitive)}
                            >
                              {formatFee(transaction, currency)}
                            </CurrencyToggleText>
                          ) : (
                            "None"
                          )
                        }
                        hidden={hideSensitive}
                        hint="Network or settlement fee paid for this transaction."
                      />
                    </div>
                    <div className="grid gap-3 lg:grid-cols-2">
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          Source record
                        </div>
                        <LedgerRow
                          label="Type"
                          value={
                            transaction.sourceType ?? transaction.direction
                          }
                        />
                        <LedgerRow
                          label="Network"
                          value={networkLabel(transaction)}
                        />
                        <LedgerRow
                          label="Counterparty"
                          value={transaction.counterparty}
                        />
                        <LedgerRow
                          label="External id"
                          value={formatShortTxid(transaction.txnId)}
                          hint="Wallet/exchange internal id. Different from the on-chain Transaction ID for off-chain sources."
                        />
                      </div>
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          Book metadata
                        </div>
                        <LedgerRow
                          label="Label"
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.label}
                              <DirtyDot active={dirtyLabel} />
                            </span>
                          }
                        />
                        <LedgerRow
                          label="Tags"
                          value={
                            tags.length ? (
                              <div
                                className={cn(
                                  "flex flex-wrap items-center gap-1",
                                  blurClass(hideSensitive),
                                )}
                              >
                                {tags.map((tag) => (
                                  <Badge
                                    key={tag}
                                    variant="secondary"
                                    className="rounded-md"
                                  >
                                    {tag}
                                  </Badge>
                                ))}
                                {dirtyTags ? <DirtyDot active /> : null}
                              </div>
                            ) : (
                              <span className="inline-flex items-center gap-1.5 text-muted-foreground">
                                None
                                <DirtyDot active={dirtyTags} />
                              </span>
                            )
                          }
                        />
                        <LedgerRow
                          label="Included"
                          value={
                            <span className="inline-flex items-center gap-1.5">
                              {localDraft.excluded ? "Excluded" : "Included"}
                              <DirtyDot active={dirtyExcluded} />
                            </span>
                          }
                        />
                      </div>
                    </div>
                    <div className="grid gap-2">
                      <Label
                        htmlFor="tx-detail-note"
                        className="flex items-center gap-1.5"
                      >
                        Note
                        <DirtyDot active={dirtyNote} />
                      </Label>
                      <Textarea
                        id="tx-detail-note"
                        value={localDraft.note}
                        onChange={(event) =>
                          updateDraft("note", event.target.value)
                        }
                        className={cn(
                          "min-h-24 resize-none",
                          blurClass(hideSensitive),
                        )}
                        placeholder="Receipt, invoice, counterparty, or review context"
                      />
                    </div>
                  </TabsContent>

                  {/* Classify — label, tags, note, review status. NO tax handling. */}
                  <TabsContent value="classify" className="mt-4">
                    <div className="grid gap-4 lg:grid-cols-2">
                      <div className="grid gap-2">
                        <Label
                          htmlFor="tx-label"
                          className="flex items-center gap-1.5"
                        >
                          Label
                          <DirtyDot active={dirtyLabel} />
                        </Label>
                        <Select
                          value={localDraft.label}
                          onValueChange={(value) => updateDraft("label", value)}
                        >
                          <SelectTrigger id="tx-label">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {classificationOptions.map((label) => (
                              <SelectItem key={label} value={label}>
                                {label}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-2">
                        <Label
                          htmlFor="tx-status"
                          className="flex items-center gap-1.5 text-muted-foreground"
                        >
                          Review status
                          <InfoHint label="Review status">
                            Settled from on-chain confirmations. Editing the
                            review state lands when the metadata daemon kind
                            is extended (tracked in TODO.md).
                          </InfoHint>
                        </Label>
                        <Select
                          value={localDraft.reviewStatus}
                          disabled
                          onValueChange={(value) =>
                            updateDraft(
                              "reviewStatus",
                              value as TransactionStatus,
                            )
                          }
                        >
                          <SelectTrigger id="tx-status">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {allTransactionStatuses.map((status) => (
                              <SelectItem key={status} value={status}>
                                {transactionStatusLabels[status]}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-2 lg:col-span-2">
                        <Label
                          htmlFor="tx-tag-input"
                          className="flex items-center gap-1.5"
                        >
                          Tags
                          <DirtyDot active={dirtyTags} />
                          <span className="text-xs font-normal text-muted-foreground">
                            (press <kbd className="rounded border bg-muted px-1">t</kbd>{" "}
                            anywhere to focus)
                          </span>
                        </Label>
                        <div className="rounded-md border bg-background p-2">
                          <div className="mb-2 flex min-h-8 flex-wrap gap-1.5">
                            {tags.length ? (
                              tags.map((tag) => (
                                <button
                                  key={tag}
                                  type="button"
                                  className={cn(
                                    "inline-flex items-center gap-1 rounded-md bg-secondary px-2 py-1 text-xs font-medium text-secondary-foreground",
                                    blurClass(hideSensitive),
                                  )}
                                  onClick={() => removeTag(tag)}
                                  aria-label={`Remove ${tag} tag`}
                                >
                                  {tag}
                                  <X className="size-3" aria-hidden="true" />
                                </button>
                              ))
                            ) : (
                              <span className="px-1 py-1 text-sm text-muted-foreground">
                                No tags yet
                              </span>
                            )}
                          </div>
                          <div className="flex gap-2">
                            <Input
                              id="tx-tag-input"
                              ref={tagInputRef}
                              value={tagInput}
                              className={blurClass(hideSensitive)}
                              onChange={(event) =>
                                setTagInput(event.target.value)
                              }
                              onKeyDown={(event) => {
                                if (
                                  event.key === "Enter" ||
                                  event.key === ","
                                ) {
                                  event.preventDefault();
                                  addTag(tagInput);
                                }
                              }}
                              placeholder="Add tag"
                            />
                            <Button
                              type="button"
                              variant="outline"
                              size="icon"
                              aria-label="Add tag"
                              onClick={() => addTag(tagInput)}
                            >
                              <Plus className="size-4" aria-hidden="true" />
                            </Button>
                          </div>
                        </div>
                        <div className="flex flex-wrap gap-1.5">
                          {availableTagSuggestions.slice(0, 7).map((tag) => (
                            <button
                              key={tag}
                              type="button"
                              className="rounded-md border px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                              onClick={() => addTag(tag)}
                            >
                              + {tag}
                            </button>
                          ))}
                        </div>
                      </div>
                      <div className="grid gap-2 lg:col-span-2">
                        <Label
                          htmlFor="tx-note"
                          className="flex items-center gap-1.5"
                        >
                          Note
                          <DirtyDot active={dirtyNote} />
                        </Label>
                        <Textarea
                          id="tx-note"
                          value={localDraft.note}
                          onChange={(event) =>
                            updateDraft("note", event.target.value)
                          }
                          className={cn(
                            "min-h-28 resize-none",
                            blurClass(hideSensitive),
                          )}
                          placeholder="Receipt, invoice, counterparty, or review context"
                        />
                      </div>
                    </div>
                  </TabsContent>

                  {/* Pricing — single workstation for pricing source + manual override */}
                  <TabsContent value="pricing" className="mt-4">
                    <div className="grid gap-4">
                      <div className="grid gap-3 md:grid-cols-4">
                        {transactionPricingOptions.map((option) => (
                          <button
                            key={option.value}
                            type="button"
                            disabled
                            className={cn(
                              "rounded-md border p-3 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-70",
                              pricingValue === option.value &&
                                "border-primary bg-accent",
                            )}
                            onClick={() => {
                              updateDraft(
                                "pricingSourceKind",
                                option.sourceKind,
                              );
                              updateDraft("pricingQuality", option.quality);
                            }}
                          >
                            <div className="text-sm font-medium">
                              {option.label}
                            </div>
                            <div className="mt-1 text-xs text-muted-foreground">
                              {option.description}
                            </div>
                          </button>
                        ))}
                      </div>
                      <div className="grid gap-3 rounded-md border bg-muted/50 p-3">
                        <div className="flex flex-wrap items-center justify-between gap-2">
                          <div>
                            <div className="flex items-center gap-1.5 text-sm font-medium">
                              Manual price override
                              <InfoHint label="Manual price override">
                                Pricing edits land when the metadata daemon
                                kind is extended to accept a price source +
                                manual override (tracked in TODO.md). The
                                editor below is preview-only for now.
                              </InfoHint>
                            </div>
                            <div className="text-xs text-muted-foreground">
                              Calculated from the fixed amount:{" "}
                              {formatBtcAmount(amountBtc)}.
                            </div>
                          </div>
                          <Badge
                            variant="outline"
                            className={cn(
                              "rounded-md",
                              localDraft.pricingSourceKind === "manual_override"
                                ? "border-amber-600/30 bg-amber-50 text-amber-700 dark:bg-amber-900/25 dark:text-amber-300"
                                : "text-muted-foreground",
                            )}
                          >
                            {pricingSourceLabel(
                              localDraft.pricingSourceKind,
                              localDraft.pricingQuality,
                            )}
                          </Badge>
                        </div>
                        <div className="grid gap-3 md:grid-cols-[100px_1fr_1fr]">
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-currency">Currency</Label>
                            <Input
                              id="tx-manual-currency"
                              value={localDraft.manualCurrency}
                              disabled
                              onChange={(event) =>
                                updateDraft(
                                  "manualCurrency",
                                  event.target.value.toUpperCase(),
                                )
                              }
                              maxLength={3}
                            />
                          </div>
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-price">Price / BTC</Label>
                            <Input
                              id="tx-manual-price"
                              ref={manualPriceRef}
                              inputMode="decimal"
                              value={localDraft.manualPrice}
                              disabled
                              onChange={(event) =>
                                updateManualPrice(event.target.value)
                              }
                              placeholder="69453.46"
                            />
                          </div>
                          <div className="grid gap-2">
                            <Label htmlFor="tx-manual-value">Total value</Label>
                            <Input
                              id="tx-manual-value"
                              inputMode="decimal"
                              value={localDraft.manualValue}
                              disabled
                              onChange={(event) =>
                                updateManualValue(event.target.value)
                              }
                              placeholder="17086.29"
                            />
                          </div>
                        </div>
                        <div className="grid gap-2">
                          <Label
                            htmlFor="tx-manual-source"
                            className="flex items-center gap-1.5"
                          >
                            Evidence / source
                            <InfoHint label="Evidence">
                              The proof for the price you typed — invoice
                              number, screenshot of an OTC quote, bank receipt,
                              or accountant note. Required for an auditable
                              manual override.
                            </InfoHint>
                          </Label>
                          <Input
                            id="tx-manual-source"
                            value={localDraft.manualSource}
                            className={blurClass(hideSensitive)}
                            disabled
                            onChange={(event) =>
                              updateDraft("manualSource", event.target.value)
                            }
                            placeholder="BTCPay invoice, bank receipt, accountant review"
                          />
                          <p className="text-[11px] text-muted-foreground">
                            Attach the actual file or URL via the{" "}
                            <span className="font-medium">Attachments</span>{" "}
                            panel on the right.
                          </p>
                        </div>
                      </div>
                      <div className="grid gap-3 sm:grid-cols-2">
                        <DetailField
                          label="Imported price"
                          value={
                            transaction.rate
                              ? `${currencyFormatter.format(transaction.rate)} / BTC`
                              : "None"
                          }
                          hidden={hideSensitive}
                          hint="The price that came in with the import. Kept here as audit reference even if you override it."
                        />
                        <DetailField
                          label="Spot now"
                          value={
                            nowRate
                              ? `${formatRateAtTime(nowRate)} / BTC`
                              : "Unknown"
                          }
                          hidden={hideSensitive}
                          hint="Current cached spot rate. Useful for sanity-checking a manual override."
                        />
                      </div>
                    </div>
                  </TabsContent>

                  {/* Tax — owns Austrian classification, taxable, excluded; ends with gain/loss */}
                  <TabsContent value="tax" className="mt-4 space-y-3">
                    <div className="rounded-md border bg-muted/50 p-3 text-sm leading-relaxed">
                      <div className="mb-1 flex items-center gap-1.5 text-xs font-semibold uppercase text-muted-foreground">
                        Plain English
                        <InfoHint label="Plain English summary">
                          Generated from the tx and your current draft. Use
                          this to sanity-check the legal labels below.
                        </InfoHint>
                      </div>
                      <p className={blurClass(hideSensitive)}>{taxNarrative}</p>
                    </div>
                    <div className="rounded-md border bg-background p-3">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <h3 className="flex items-center gap-1.5 text-sm font-semibold">
                          Tax handling
                          <DirtyDot active={dirtyExcluded} />
                        </h3>
                        <Badge
                          variant={
                            localDraft.taxable && !localDraft.excluded
                              ? "default"
                              : "outline"
                          }
                        >
                          {localDraft.excluded
                            ? "Excluded"
                            : localDraft.taxable
                              ? "Taxable"
                              : "Not taxable"}
                        </Badge>
                      </div>
                      <div className="grid gap-3 xl:grid-cols-[minmax(220px,0.9fr)_minmax(0,1fr)_minmax(0,1fr)]">
                        <div className="grid gap-2">
                          <Label
                            htmlFor="tx-tax-treatment"
                            className="flex items-center gap-1.5"
                          >
                            Austrian category
                            <InfoHint label="Austrian category">
                              Maps to § 27b EStG buckets. "Neu" covers
                              post-2022 holdings; "Alt" covers pre-2022
                              speculation-period inventory; "Own-wallet
                              transfer" stays outside the realization rules.
                            </InfoHint>
                          </Label>
                          <Select
                            value={austrianSelectionValue(
                              localDraft.atRegime,
                              localDraft.atCategory,
                            )}
                            disabled
                            onValueChange={(value) => {
                              const option =
                                austrianTaxClassificationForValue(value);
                              updateDraft("atRegime", option.atRegime);
                              updateDraft("atCategory", option.atCategory);
                              updateDraft("taxable", option.taxable);
                            }}
                          >
                            <SelectTrigger id="tx-tax-treatment">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              {austrianTaxClassificationOptions.map(
                                (option) => (
                                  <SelectItem
                                    key={option.value}
                                    value={option.value}
                                  >
                                    {option.label}
                                  </SelectItem>
                                ),
                              )}
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label
                              htmlFor="tx-taxable"
                              className="flex items-center gap-1.5"
                            >
                              Taxable
                            </Label>
                            <p className="text-xs text-muted-foreground">
                              Included in journal processing.
                            </p>
                          </div>
                          <Switch
                            id="tx-taxable"
                            checked={localDraft.taxable}
                            disabled
                            onCheckedChange={(checked) =>
                              updateDraft("taxable", checked)
                            }
                          />
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label
                              htmlFor="tx-excluded"
                              className="flex items-center gap-1.5"
                            >
                              Excluded
                              <DirtyDot active={dirtyExcluded} />
                              <span className="text-xs font-normal text-muted-foreground">
                                (<kbd className="rounded border bg-muted px-1">e</kbd>)
                              </span>
                            </Label>
                            <p className="text-xs text-muted-foreground">
                              Kept out of journal processing.
                            </p>
                          </div>
                          <Switch
                            id="tx-excluded"
                            checked={localDraft.excluded}
                            onCheckedChange={(checked) =>
                              updateDraft("excluded", checked)
                            }
                          />
                        </div>
                      </div>
                    </div>
                    <div className="overflow-hidden rounded-md border">
                      <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                        Projected effect
                      </div>
                      <LedgerRow
                        label="Cost basis"
                        value={
                          transaction.amount === null
                            ? MISSING_FIAT_LABEL
                            : currencyFormatter.format(transaction.amount)
                        }
                        align="right"
                        hint="Acquisition value used by the tax engine."
                      />
                      <LedgerRow
                        label="Proceeds"
                        value={
                          flow !== "outgoing"
                            ? currencyFormatter.format(0)
                            : transaction.amount === null
                              ? MISSING_FIAT_LABEL
                              : currencyFormatter.format(transaction.amount)
                        }
                        align="right"
                        hint="Disposal value applied on outgoing tx."
                      />
                      <LedgerRow
                        label="Gain / loss"
                        value="Pending journal run"
                        align="right"
                        muted
                        hint="Calculated by RP2 once journals are processed."
                      />
                      {localDraft.pricingSourceKind === "manual_override" ? (
                        <LedgerRow
                          label="Price evidence"
                          value={
                            <span className={blurClass(hideSensitive)}>
                              {localDraft.manualSource || "Source missing"}
                            </span>
                          }
                          align="right"
                          muted
                        />
                      ) : null}
                    </div>
                  </TabsContent>

                  {/* Linked — pairs, source-of-funds (placeholder), journal entries (placeholder) */}
                  <TabsContent value="linked" className="mt-4 space-y-3">
                    {pair ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="flex items-center justify-between border-b bg-muted px-3 py-1.5">
                          <div className="flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                            <Repeat2
                              className="size-3"
                              aria-hidden="true"
                            />
                            Paired movement
                            {pair.policy ? (
                              <Badge
                                variant="outline"
                                className="rounded-md text-[10px]"
                              >
                                {pair.policy}
                              </Badge>
                            ) : null}
                          </div>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="h-7 px-2 text-xs text-muted-foreground"
                            disabled
                          >
                            Unpair
                          </Button>
                        </div>
                        <LedgerRow
                          label="Out wallet"
                          value={pair.outWallet ?? "Unknown"}
                          align="right"
                        />
                        <LedgerRow
                          label="Out amount"
                          value={`${Math.abs(
                            (pair.outAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.outAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label="In wallet"
                          value={pair.inWallet ?? "Unknown"}
                          align="right"
                        />
                        <LedgerRow
                          label="In amount"
                          value={`${Math.abs(
                            (pair.inAmountSat ?? 0) / 100_000_000,
                          ).toFixed(8)} ${pair.inAsset ?? "BTC"}`}
                          align="right"
                        />
                        <LedgerRow
                          label="Pair fee"
                          value={
                            pair.feeSat
                              ? formatBtcAmount(
                                  Math.abs(pair.feeSat / 100_000_000),
                                )
                              : "-"
                          }
                          align="right"
                          muted
                          hint="Signed fee computed at pair time."
                        />
                        {pair.kind ? (
                          <LedgerRow
                            label="Pair kind"
                            value={pair.kind}
                            align="right"
                            muted
                          />
                        ) : null}
                      </div>
                    ) : (
                      <div className="rounded-md border border-dashed bg-muted/40 p-4 text-sm">
                        <div className="flex items-center gap-2 font-medium">
                          <Repeat2
                            className="size-4 text-muted-foreground"
                            aria-hidden="true"
                          />
                          No paired movement
                        </div>
                        <p className="mt-1 text-xs text-muted-foreground">
                          When this tx is the other leg of a transfer, swap, or
                          peg-in/out, the matched leg shows up here. You can
                          also create a manual pair from the swap candidate
                          queue.
                        </p>
                      </div>
                    )}

                    <div className="rounded-md border border-dashed bg-muted/40 p-4 text-sm">
                      <div className="flex items-center gap-2 font-medium">
                        <Layers
                          className="size-4 text-muted-foreground"
                          aria-hidden="true"
                        />
                        Source of funds
                      </div>
                      <p className="mt-1 text-xs text-muted-foreground">
                        Linked sources and downstream uses will appear here
                        once the source-of-funds workstation has reviewed this
                        row.
                      </p>
                    </div>
                    <div className="rounded-md border border-dashed bg-muted/40 p-4 text-sm">
                      <div className="flex items-center gap-2 font-medium">
                        <FileText
                          className="size-4 text-muted-foreground"
                          aria-hidden="true"
                        />
                        Journal entries
                      </div>
                      <p className="mt-1 text-xs text-muted-foreground">
                        After the next journal run, the RP2 entries this tx
                        produced will appear here with a deep link into the
                        ledger view.
                      </p>
                    </div>
                  </TabsContent>

                  {/* Ledger — lead with Net wallet impact, breakdown below */}
                  <TabsContent value="ledger" className="mt-4 space-y-3">
                    <div className="rounded-md border bg-card p-4">
                      <div className="flex flex-wrap items-center justify-between gap-3">
                        <div>
                          <div className="flex items-center gap-1.5 text-xs uppercase text-muted-foreground">
                            Net wallet impact
                            <InfoHint label="Net wallet impact">
                              The signed change to this wallet after principal
                              and fees. This is the bottom-line number for
                              accounting.
                            </InfoHint>
                          </div>
                          <div className="mt-1 text-2xl font-semibold tabular-nums">
                            {impactDirection === 0 && !feeBtc ? (
                              "See paired movement"
                            ) : (
                              <span className={blurClass(hideSensitive)}>
                                {formatSheetMoney(
                                  netImpactEur,
                                  netImpactBtc,
                                  balanceCurrency,
                                  true,
                                )}
                              </span>
                            )}
                          </div>
                        </div>
                        <div className="flex rounded-md border bg-background p-0.5">
                          {(["btc", "eur"] satisfies Currency[]).map(
                            (value) => (
                              <button
                                key={value}
                                type="button"
                                aria-pressed={balanceCurrency === value}
                                onClick={() => setBalanceCurrency(value)}
                                className={cn(
                                  "h-7 min-w-10 rounded px-2 text-xs font-medium transition-colors",
                                  balanceCurrency === value
                                    ? "bg-primary text-primary-foreground"
                                    : "text-muted-foreground hover:text-foreground",
                                )}
                              >
                                {value === "btc" ? "BTC" : "EUR"}
                              </button>
                            ),
                          )}
                        </div>
                      </div>
                    </div>

                    {feeBtc > 0 || impactDirection === 0 ? (
                      <div className="overflow-hidden rounded-md border">
                        <div className="border-b bg-muted px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
                          How it adds up
                        </div>
                        <LedgerRow
                          label="Principal"
                          value={
                            impactDirection === 0 ? (
                              <span className="text-muted-foreground">
                                Paired — see Linked tab
                              </span>
                            ) : (
                              <span className={blurClass(hideSensitive)}>
                                {formatSheetMoney(
                                  principalImpactEur,
                                  principalImpactBtc,
                                  balanceCurrency,
                                  true,
                                )}
                              </span>
                            )
                          }
                          align="right"
                          hint="Signed principal applied to this wallet."
                        />
                        {feeBtc > 0 ? (
                          <LedgerRow
                            label="Fee"
                            value={
                              <span className={blurClass(hideSensitive)}>
                                {formatSheetMoney(
                                  feeImpactEur,
                                  feeImpactBtc,
                                  balanceCurrency,
                                  true,
                                )}
                              </span>
                            }
                            align="right"
                            hint="Network or settlement fee subtracted from this wallet."
                          />
                        ) : null}
                        <LedgerRow
                          label="Net"
                          value={
                            <span
                              className={cn(
                                "font-semibold",
                                blurClass(hideSensitive),
                              )}
                            >
                              {impactDirection === 0 && !feeBtc
                                ? "See paired movement"
                                : formatSheetMoney(
                                    netImpactEur,
                                    netImpactBtc,
                                    balanceCurrency,
                                    true,
                                  )}
                            </span>
                          }
                          align="right"
                          muted
                        />
                      </div>
                    ) : null}
                  </TabsContent>
                </Tabs>
              </div>

              <aside className="space-y-3">
                <ReviewChecklist
                  items={reviewChecklistItems}
                  onJump={(tab) => setActiveTab(tab)}
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
                      icon={<Link2 className="size-3.5" aria-hidden="true" />}
                      label="Source id"
                      value={
                        <span className="font-mono">
                          {formatShortTxid(sourceRecordId)}
                        </span>
                      }
                      copyValue={sourceRecordId}
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
                  onOpen={onOpenAttachment}
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
                <div className="rounded-md border border-dashed bg-muted/40 p-3 text-xs text-muted-foreground">
                  <div className="mb-2 flex items-center gap-2 font-semibold text-foreground">
                    <History
                      className="size-4 text-muted-foreground"
                      aria-hidden="true"
                    />
                    Edit history
                  </div>
                  Once enabled, every metadata save shows up here with who
                  changed what, when, and the prior value.
                </div>
              </aside>
            </div>
          </div>

          <SheetFooter className="border-t p-3 sm:flex-row sm:items-center sm:justify-between sm:px-6">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              {dirtyCount > 0 ? (
                <span className="inline-flex items-center gap-1.5 font-medium text-amber-600 dark:text-amber-400">
                  <span className="inline-block size-1.5 rounded-full bg-amber-500" />
                  {dirtyCount} unsaved {dirtyCount === 1 ? "change" : "changes"}
                </span>
              ) : null}
              <span className="hidden items-center gap-1.5 sm:inline-flex">
                <kbd className="rounded border bg-muted px-1">⌘S</kbd> save ·{" "}
                <kbd className="rounded border bg-muted px-1">1–6</kbd> tabs ·{" "}
                <kbd className="rounded border bg-muted px-1">e</kbd> exclude
              </span>
              {saveError ? (
                <span className="basis-full text-destructive sm:basis-auto">
                  {saveError}
                </span>
              ) : null}
            </div>
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="outline"
                disabled={isSaving}
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              {dirtyCount > 0 ? (
                <Button
                  type="button"
                  variant="ghost"
                  className="gap-1.5 text-muted-foreground"
                  disabled={isSaving}
                  onClick={() => {
                    setLocalDraft(originalDraft);
                    setTagInput("");
                  }}
                >
                  <RotateCcw className="size-4" aria-hidden="true" />
                  Discard
                </Button>
              ) : null}
              <Button
                type="button"
                className="gap-2"
                disabled={isSaving || dirtyCount === 0}
                onClick={async () => {
                  try {
                    if (onSaveAndNext && hasNext) {
                      await onSaveAndNext(transaction.id, localDraft);
                    } else {
                      await onSave(transaction.id, localDraft);
                      onOpenChange(false);
                    }
                  } catch {
                    // The parent renders the daemon error in the footer.
                  }
                }}
              >
                <Save className="size-4" aria-hidden="true" />
                {isSaving
                  ? "Saving"
                  : onSaveAndNext && hasNext
                    ? "Save & next"
                    : "Save"}
                {onSaveAndNext && hasNext && !isSaving ? (
                  <ArrowRight className="size-4" aria-hidden="true" />
                ) : null}
              </Button>
            </div>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </TooltipProvider>
  );
}
