import {
  Bitcoin,
  BookMarked,
  CalendarClock,
  Copy,
  ExternalLink,
  Hash,
  Link2,
  ListChecks,
  Plus,
  Save,
  Tags,
  X,
} from "lucide-react";
import * as React from "react";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import type { Currency } from "@/lib/currency";
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

function DetailField({
  label,
  value,
  copyValue,
  hidden,
}: {
  label: string;
  value: React.ReactNode;
  copyValue?: string;
  hidden?: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md border bg-background p-3">
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="text-[10px] font-medium uppercase text-muted-foreground">
          {label}
        </span>
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
}: {
  label: string;
  value: React.ReactNode;
  align?: "left" | "right";
  muted?: boolean;
}) {
  return (
    <div
      className={cn(
        "grid min-h-10 grid-cols-[minmax(140px,0.9fr)_minmax(0,1.1fr)] items-center gap-3 border-b px-3 py-2 last:border-b-0",
        muted && "bg-muted/35",
      )}
    >
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className={cn(
          "min-w-0 text-sm font-medium",
          align === "right" && "text-right tabular-nums",
        )}
      >
        {value}
      </div>
    </div>
  );
}

export function TransactionDetailSheet({
  transaction,
  draft,
  initialTab,
  hideSensitive,
  currency,
  explorerSettings,
  onOpenChange,
  onOpenExplorer,
  onSave,
}: {
  transaction: Transaction | null;
  draft: TransactionEditDraft | null;
  initialTab: string;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  onOpenChange: (open: boolean) => void;
  onOpenExplorer: (transaction: Transaction) => void;
  onSave: (transactionId: string, draft: TransactionEditDraft) => void;
}) {
  const [activeTab, setActiveTab] = React.useState(initialTab);
  const [localDraft, setLocalDraft] = React.useState<TransactionEditDraft | null>(
    draft,
  );
  const [tagInput, setTagInput] = React.useState("");

  React.useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab, transaction?.id]);

  React.useEffect(() => {
    setLocalDraft(draft);
    setTagInput("");
  }, [draft, transaction?.id]);

  if (!transaction || !localDraft) return null;

  const StatusIcon = transactionStatusIcons[localDraft.reviewStatus];
  const flow = transactionFlow(transaction);
  const explorer = explorerForTransaction(transaction, explorerSettings);
  const amountBtc = transactionBtc(transaction);
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

  const updateDraft = <K extends keyof TransactionEditDraft>(
    key: K,
    value: TransactionEditDraft[K],
  ) => {
    setLocalDraft((current) =>
      current ? { ...current, [key]: value } : current,
    );
  };
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

  return (
    <Sheet open={Boolean(transaction)} onOpenChange={onOpenChange}>
      <SheetContent
        className="w-[min(100vw,1120px)] overflow-hidden p-0 sm:max-w-none"
        showCloseButton={false}
      >
        <SheetHeader className="border-b p-0">
          <div className="flex items-start justify-between gap-4 px-4 py-4 sm:px-6">
            <div className="min-w-0">
              <div className="mb-2 flex flex-wrap items-center gap-2">
                <Badge variant="outline" className="gap-1 rounded-md">
                  <Bitcoin className="size-3 text-amber-500" aria-hidden="true" />
                  {transaction.asset ?? "BTC"}
                </Badge>
                <Badge
                  variant="outline"
                  className={cn("rounded-md", transactionFlowStyles[flow])}
                >
                  {transactionFlowLabels[flow]}
                </Badge>
                <Badge
                  variant="outline"
                  className={cn(
                    "gap-1 rounded-md",
                    transactionStatusStyles[localDraft.reviewStatus],
                  )}
                >
                  <StatusIcon className="size-3" aria-hidden="true" />
                  {transactionStatusLabels[localDraft.reviewStatus]}
                </Badge>
              </div>
              <SheetTitle className="truncate text-xl sm:text-2xl">
                {signedPrefix}
                <span className={blurClass(hideSensitive)}>
                  {formatBtcAmount(amountBtc)}
                </span>
              </SheetTitle>
              <SheetDescription className="mt-1 truncate">
                {transaction.wallet} · {transaction.counterparty}
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
        </SheetHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="grid gap-4 p-4 sm:p-6 xl:grid-cols-[minmax(0,1fr)_340px]">
            <div className="min-w-0 space-y-4">
              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                <DetailField
                  label="Timestamp"
                  value={transaction.date}
                  copyValue={transaction.date}
                />
                <DetailField
                  label="Wallet"
                  value={transaction.wallet ?? "Unassigned"}
                  hidden={hideSensitive}
                />
                <DetailField
                  label="Transaction ID"
                  value={formatShortTxid(transaction.explorerId ?? transaction.txnId)}
                  copyValue={transaction.explorerId ?? transaction.txnId}
                  hidden={hideSensitive}
                />
                <DetailField
                  label="Price"
                  value={
                    localDraft.pricingSourceKind === "manual_override" &&
                    localDraft.manualPrice
                      ? `${localDraft.manualPrice} ${localDraft.manualCurrency}/BTC`
                      : transaction.rate
                      ? `${currencyFormatter.format(transaction.rate)} / BTC`
                      : "Missing"
                  }
                  hidden={hideSensitive}
                />
              </div>

              <Tabs value={activeTab} onValueChange={setActiveTab}>
                <TabsList className="grid w-full grid-cols-5">
                  <TabsTrigger value="details">Details</TabsTrigger>
                  <TabsTrigger value="classify">Classify</TabsTrigger>
                  <TabsTrigger value="pricing">Pricing</TabsTrigger>
                  <TabsTrigger value="tax">Tax</TabsTrigger>
                  <TabsTrigger value="ledger">Ledger</TabsTrigger>
                </TabsList>

                <TabsContent value="details" className="mt-4 space-y-4">
                  <div className="grid gap-3 lg:grid-cols-2">
                    <div className="rounded-md border">
                      <LedgerRow
                        label="Type"
                        value={transaction.sourceType ?? transaction.direction}
                      />
                      <LedgerRow label="Network" value={transaction.paymentMethod} />
                      <LedgerRow label="Counterparty" value={transaction.counterparty} />
                      <LedgerRow
                        label="External id"
                        value={formatShortTxid(transaction.txnId)}
                      />
                    </div>
                    <div className="rounded-md border">
                      <LedgerRow label="Label" value={localDraft.label} />
                      <LedgerRow
                        label="Tags"
                        value={
                          tags.length ? (
                            <div
                              className={cn(
                                "flex flex-wrap justify-end gap-1",
                                blurClass(hideSensitive),
                              )}
                            >
                              {tags.map((tag) => (
                                <Badge key={tag} variant="secondary" className="rounded-md">
                                  {tag}
                                </Badge>
                              ))}
                            </div>
                          ) : (
                            "None"
                          )
                        }
                      />
                      <LedgerRow
                        label="Included"
                        value={localDraft.excluded ? "Excluded" : "Included"}
                      />
                      <LedgerRow label="Last edited" value="Local draft" />
                    </div>
                  </div>
                  <div className="rounded-md border bg-muted/25 p-3">
                    <div className="mb-2 text-xs font-medium text-muted-foreground">
                      Note
                    </div>
                    <p
                      className={cn(
                        "min-h-10 whitespace-pre-wrap text-sm",
                        blurClass(hideSensitive),
                      )}
                    >
                      {localDraft.note || "-"}
                    </p>
                  </div>
                </TabsContent>

                <TabsContent value="ledger" className="mt-4">
                  <div className="overflow-hidden rounded-md border">
                    <LedgerRow
                      label="Asset"
                      value={transaction.asset ?? "BTC"}
                      align="right"
                    />
                    <LedgerRow
                      label="Amount"
                      value={
                        <span className={blurClass(hideSensitive)}>
                          {signedPrefix}
                          {formatBtcAmount(amountBtc)}
                        </span>
                      }
                      align="right"
                      muted
                    />
                    <LedgerRow
                      label="Value"
                      value={
                        <CurrencyToggleText className={blurClass(hideSensitive)}>
                          {formatDisplayMoney(transaction.amount, amountBtc, currency)}
                        </CurrencyToggleText>
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Fee"
                      value={
                        <span className={blurClass(hideSensitive)}>
                          {formatFee(transaction, currency)}
                        </span>
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Running balance"
                      value="Calculated after journal processing"
                      align="right"
                    />
                  </div>
                </TabsContent>

                <TabsContent value="pricing" className="mt-4">
                  <div className="grid gap-4">
                    <div className="grid gap-3 md:grid-cols-4">
                      {transactionPricingOptions.map((option) => (
                        <button
                          key={option.value}
                          type="button"
                          className={cn(
                            "rounded-md border p-3 text-left transition-colors hover:bg-muted/40",
                            pricingValue === option.value &&
                              "border-primary bg-muted/60",
                          )}
                          onClick={() => {
                            updateDraft("pricingSourceKind", option.sourceKind);
                            updateDraft("pricingQuality", option.quality);
                          }}
                        >
                          <div className="text-sm font-medium">{option.label}</div>
                          <div className="mt-1 text-xs text-muted-foreground">
                            {option.description}
                          </div>
                        </button>
                      ))}
                    </div>
                    <div className="grid gap-3 rounded-md border bg-muted/20 p-3">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div>
                          <div className="text-sm font-medium">
                            Manual price override
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
                            inputMode="decimal"
                            value={localDraft.manualPrice}
                            onFocus={() => {
                              updateDraft("pricingSourceKind", "manual_override");
                              updateDraft("pricingQuality", "exact");
                            }}
                            onChange={(event) => updateManualPrice(event.target.value)}
                            placeholder="69453.46"
                          />
                        </div>
                        <div className="grid gap-2">
                          <Label htmlFor="tx-manual-value">Total value</Label>
                          <Input
                            id="tx-manual-value"
                            inputMode="decimal"
                            value={localDraft.manualValue}
                            onFocus={() => {
                              updateDraft("pricingSourceKind", "manual_override");
                              updateDraft("pricingQuality", "exact");
                            }}
                            onChange={(event) => updateManualValue(event.target.value)}
                            placeholder="17086.29"
                          />
                        </div>
                      </div>
                      <div className="grid gap-2">
                        <Label htmlFor="tx-manual-source">Evidence / source</Label>
                        <Input
                          id="tx-manual-source"
                          value={localDraft.manualSource}
                          className={blurClass(hideSensitive)}
                          onFocus={() => {
                            updateDraft("pricingSourceKind", "manual_override");
                            updateDraft("pricingQuality", "exact");
                          }}
                          onChange={(event) =>
                            updateDraft("manualSource", event.target.value)
                          }
                          placeholder="BTCPay invoice, bank receipt, accountant review"
                        />
                      </div>
                    </div>
                    <div className="grid gap-3 md:grid-cols-3">
                      <DetailField
                        label="Imported price"
                        value={
                          transaction.rate
                            ? `${currencyFormatter.format(transaction.rate)} / BTC`
                            : "None"
                        }
                        hidden={hideSensitive}
                      />
                      <DetailField
                        label="Source value"
                        value={currencyFormatter.format(transaction.amount)}
                        hidden={hideSensitive}
                      />
                      <DetailField
                        label="Manual source"
                        value={localDraft.manualSource || "-"}
                        hidden={hideSensitive}
                      />
                    </div>
                  </div>
                </TabsContent>

                <TabsContent value="tax" className="mt-4 space-y-3">
                  <div className="grid gap-3 md:grid-cols-4">
                    <DetailField label="AT regime" value={localDraft.atRegime} />
                    <DetailField
                      label="AT category"
                      value={taxClassification.shortLabel}
                    />
                    <DetailField label="Taxable" value={localDraft.taxable ? "Yes" : "No"} />
                    <DetailField
                      label="Price source"
                      value={pricingSourceLabel(
                        localDraft.pricingSourceKind,
                        localDraft.pricingQuality,
                      )}
                    />
                  </div>
                  <div className="overflow-hidden rounded-md border">
                    <LedgerRow
                      label="Cost basis"
                      value={currencyFormatter.format(transaction.amount)}
                      align="right"
                    />
                    <LedgerRow
                      label="Proceeds"
                      value={
                        flow === "outgoing"
                          ? currencyFormatter.format(transaction.amount)
                          : currencyFormatter.format(0)
                      }
                      align="right"
                    />
                    <LedgerRow
                      label="Gain / loss"
                      value="Pending journal run"
                      align="right"
                      muted
                    />
                    <LedgerRow
                      label="Austrian bucket"
                      value={taxClassification.label}
                      align="right"
                    />
                    {localDraft.pricingSourceKind === "manual_override" ? (
                      <LedgerRow
                        label="Manual price evidence"
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

                <TabsContent value="classify" className="mt-4">
                  <div className="grid gap-4 lg:grid-cols-2">
                    <div className="grid gap-2">
                      <Label htmlFor="tx-label">Label</Label>
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
                      <Label htmlFor="tx-status">Review status</Label>
                      <Select
                        value={localDraft.reviewStatus}
                        onValueChange={(value) =>
                          updateDraft("reviewStatus", value as TransactionStatus)
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
                      <Label htmlFor="tx-tag-input">Tags</Label>
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
                            value={tagInput}
                            className={blurClass(hideSensitive)}
                            onChange={(event) => setTagInput(event.target.value)}
                            onKeyDown={(event) => {
                              if (event.key === "Enter" || event.key === ",") {
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
                    <div className="rounded-md border bg-background p-3 lg:col-span-2">
                      <div className="mb-3 flex items-center justify-between gap-3">
                        <h3 className="text-sm font-semibold">Tax handling</h3>
                        <Badge variant={localDraft.taxable ? "default" : "outline"}>
                          {localDraft.excluded
                            ? "Excluded"
                            : localDraft.taxable
                              ? "Taxable"
                              : "Not taxable"}
                        </Badge>
                      </div>
                      <div className="grid gap-3 xl:grid-cols-[minmax(220px,0.9fr)_minmax(0,1fr)_minmax(0,1fr)]">
                        <div className="grid gap-2">
                          <Label htmlFor="tx-tax-treatment">Austrian category</Label>
                          <Select
                            value={austrianSelectionValue(
                              localDraft.atRegime,
                              localDraft.atCategory,
                            )}
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
                              {austrianTaxClassificationOptions.map((option) => (
                                <SelectItem key={option.value} value={option.value}>
                                  {option.label}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label htmlFor="tx-taxable">Taxable</Label>
                            <p className="text-xs text-muted-foreground">
                              Included in tax event preparation.
                            </p>
                          </div>
                          <Switch
                            id="tx-taxable"
                            checked={localDraft.taxable}
                            onCheckedChange={(checked) =>
                              updateDraft("taxable", checked)
                            }
                          />
                        </div>
                        <div className="flex min-h-[76px] items-center justify-between gap-3 rounded-md border p-3">
                          <div className="min-w-0">
                            <Label htmlFor="tx-excluded">Excluded</Label>
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
                    <div className="grid gap-2 lg:col-span-2">
                      <Label htmlFor="tx-note">Note</Label>
                      <Textarea
                        id="tx-note"
                        value={localDraft.note}
                        onChange={(event) => updateDraft("note", event.target.value)}
                          className={cn("min-h-28 resize-none", blurClass(hideSensitive))}
                          placeholder="Receipt, invoice, counterparty, or review context"
                      />
                    </div>
                  </div>
                </TabsContent>
              </Tabs>
            </div>

            <aside className="space-y-3">
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <ListChecks className="size-4 text-muted-foreground" aria-hidden="true" />
                  Review
                </div>
                <div className="space-y-2 text-sm">
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Label</span>
                    <span className="font-medium">{localDraft.label}</span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Tax</span>
                    <span className="text-right font-medium">
                      {taxClassification.shortLabel}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Price</span>
                    <span className="text-right font-medium">
                      {pricingSourceLabel(
                        localDraft.pricingSourceKind,
                        localDraft.pricingQuality,
                      )}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-muted-foreground">Flags</span>
                    <span className="font-medium">
                      {localDraft.excluded
                        ? "Excluded"
                        : localDraft.taxable
                          ? "Taxable"
                          : "Non-taxable"}
                    </span>
                  </div>
                </div>
              </div>
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <Hash className="size-4 text-muted-foreground" aria-hidden="true" />
                  Identity
                </div>
                <div className="space-y-2 text-xs text-muted-foreground">
                  <button
                    type="button"
                    className="flex w-full min-w-0 items-center justify-between gap-2 rounded-md border px-2 py-2 text-left hover:bg-muted/40"
                    onClick={() => copyText(transaction.txnId)}
                  >
                    <span>Transaction</span>
                    <span className={cn("truncate font-mono", blurClass(hideSensitive))}>
                      {formatShortTxid(transaction.txnId)}
                    </span>
                  </button>
                  <div className="flex items-center gap-2 rounded-md border px-2 py-2">
                    <CalendarClock className="size-3.5" aria-hidden="true" />
                    <span>{transaction.date}</span>
                  </div>
                  <div className="flex items-center gap-2 rounded-md border px-2 py-2">
                    <Link2 className="size-3.5" aria-hidden="true" />
                    <span>{explorer ? explorer.label : "No public explorer"}</span>
                  </div>
                </div>
              </div>
              <div className="rounded-md border bg-card p-3">
                <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
                  <Tags className="size-4 text-muted-foreground" aria-hidden="true" />
                  Tags
                </div>
                <div className="flex min-h-8 flex-wrap gap-1.5">
                  {tags.length ? (
                    tags.map((tag) => (
                      <Badge
                        key={tag}
                        variant="secondary"
                        className={cn("rounded-md", blurClass(hideSensitive))}
                      >
                        {tag}
                      </Badge>
                    ))
                  ) : (
                    <span className="text-sm text-muted-foreground">None</span>
                  )}
                </div>
              </div>
            </aside>
          </div>
        </div>

        <SheetFooter className="border-t p-3 sm:flex-row sm:items-center sm:justify-between sm:px-6">
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <BookMarked className="size-4" aria-hidden="true" />
            <span>Demo only: changes stay local until persistence is wired.</span>
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              className="gap-2"
              onClick={() => {
                onSave(transaction.id, localDraft);
                onOpenChange(false);
              }}
            >
              <Save className="size-4" aria-hidden="true" />
              Save local draft
            </Button>
          </div>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  );
}
