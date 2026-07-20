import { Plus } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
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
import { Textarea } from "@/components/ui/textarea";
import { formatBtc } from "@/lib/currency";
import { pageHeaderActionClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";

import {
  allTransactionStatuses,
  austrianSelectionValue,
  austrianTaxClassificationFor,
  austrianTaxClassificationForValue,
  btcFromSatsInput,
  calculateNewTransactionPricing,
  classificationOptions,
  classificationOptionLabelKeys,
  formatAssetAmount,
  formatBtcAmount,
  formatDraftFiat,
  inferredAssetForDraft,
  isExternalPricingSource,
  isTwoLegNewTransactionFlow,
  austrianTaxClassificationOptions,
  mockNewTransactionMovementCandidates,
  newTransactionFlowOptions,
  newTransactionNetworkOptions,
  newTransactionPricingOptions,
  nextLabelForFlow,
  nextTaxClassificationForFlow,
  pricingOptionForValue,
  pricingSelectionValue,
  pricingSourceLabel,
  pricingSourceStyles,
  parseManualDecimal,
  showConfirmedAtForDraft,
  showSingleAssetForDraft,
  signedNewTransactionBtc,
  sourceKindForNetwork,
  splitDraftTags,
  transactionFlowLabels,
  transactionStatusLabels,
  uniqueTags,
  type NewTransactionDraft,
  type NewTransactionEvidence,
  type PricingDraftField,
  type PricingSelectionValue,
  type TransactionFlow,
  type TransactionStatus,
} from "./model";

export function NewTransactionDialog({
  open,
  draft,
  walletSourceOptions,
  movementCandidates = mockNewTransactionMovementCandidates,
  onOpenChange,
  onDraftChange,
  onSaveDraft,
}: {
  open: boolean;
  draft: NewTransactionDraft;
  walletSourceOptions: string[];
  movementCandidates?: typeof mockNewTransactionMovementCandidates;
  onOpenChange: (open: boolean) => void;
  onDraftChange: (draft: NewTransactionDraft) => void;
  onSaveDraft: () => void;
}) {
  const { t } = useTranslation(["transactions", "common"]);
  const bodyRef = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    if (open) bodyRef.current?.scrollTo({ top: 0 });
  }, [open]);
  const updateDraft = React.useCallback(
    (patch: Partial<NewTransactionDraft>) => {
      onDraftChange({ ...draft, ...patch });
    },
    [draft, onDraftChange],
  );
  const updateEvidence = React.useCallback(
    (patch: Partial<NewTransactionEvidence>) => {
      onDraftChange({
        ...draft,
        evidence: { ...draft.evidence, ...patch },
      });
    },
    [draft, onDraftChange],
  );
  const updateFlow = React.useCallback(
    (flow: TransactionFlow) => {
      const taxClassification = nextTaxClassificationForFlow(flow);
      const fallbackWallet =
        draft.wallet && draft.wallet !== "External" ? draft.wallet : "Cold Storage";
      const fromWallet =
        flow === "incoming"
          ? draft.fromWallet
          : draft.fromWallet === "External"
            ? fallbackWallet
            : draft.fromWallet || fallbackWallet;
      onDraftChange({
        ...draft,
        flow,
        fromWallet,
        toWallet: draft.toWallet === "External" ? fallbackWallet : draft.toWallet,
        label: nextLabelForFlow(flow),
        atRegime: taxClassification.atRegime,
        atCategory: taxClassification.atCategory,
        taxable: taxClassification.taxable,
      });
    },
    [draft, onDraftChange],
  );
  const updatePricingField = React.useCallback(
    (field: PricingDraftField, value: string) => {
      onDraftChange(calculateNewTransactionPricing({ ...draft, [field]: value }, field));
    },
    [draft, onDraftChange],
  );
  const twoLegFlow = isTwoLegNewTransactionFlow(draft.flow);
  const showConfirmedAt = showConfirmedAtForDraft(draft);
  const showSingleAsset = showSingleAssetForDraft(draft);
  const ownWalletOptions = walletSourceOptions.filter((wallet) => wallet !== "External");
  const singleLegBtc = btcFromSatsInput(draft.amountSats) ?? 0;
  const sendLegBtc = btcFromSatsInput(draft.sendAmountSats) ?? 0;
  const receiveLegBtc = btcFromSatsInput(draft.receiveAmountSats) ?? 0;
  const movementBtc = twoLegFlow
    ? Math.max(sendLegBtc, receiveLegBtc)
    : singleLegBtc;
  const feeBtc = btcFromSatsInput(draft.feeSats) ?? 0;
  const signedBtc = signedNewTransactionBtc(draft);
  const totalValue = parseManualDecimal(draft.totalValue);
  const priceValue = parseManualDecimal(draft.pricePerBtc);
  const tags = uniqueTags(splitDraftTags(draft.tags));
  const taxClassification = austrianTaxClassificationFor(
    draft.atRegime,
    draft.atCategory,
  );
  const selectedMovement = movementCandidates.find(
    (candidate) => candidate.id === draft.movementId,
  );
  const movementLabel =
    draft.movementId === "new"
      ? t("newDialog.newMovement")
      : selectedMovement
        ? // loose translator
          (t as (key: string) => string)(selectedMovement.labelKey)
        : t("newDialog.standalone");
  const fromDisplay =
    draft.flow === "incoming"
      ? draft.fromExternal || "External"
      : draft.fromWallet || t("fallback.unassigned");
  const toDisplay =
    draft.flow === "outgoing"
      ? draft.toExternal || "External"
      : draft.toWallet || t("fallback.unassigned");
  const primaryEvidence =
    draft.evidence.txidOrPermalink ||
    draft.evidence.btcpayInvoiceId ||
    draft.evidence.swapId ||
    draft.evidence.exchangeCsvRow ||
    draft.evidence.preimage;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogTrigger asChild>
        <Button
          size="sm"
          className={pageHeaderActionClassName}
          aria-label={t("newDialog.triggerAria")}
          disabled
        >
          <Plus className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">{t("newDialog.trigger")}</span>
        </Button>
      </DialogTrigger>
      <DialogContent className="flex max-h-[calc(100vh-1rem)] flex-col overflow-hidden p-0 sm:max-w-[80rem]">
        <DialogHeader className="shrink-0 px-5 pt-4 pb-2 pr-12">
          <DialogTitle>{t("newDialog.title")}</DialogTitle>
          <DialogDescription>{t("newDialog.description")}</DialogDescription>
        </DialogHeader>

        <div ref={bodyRef} className="min-h-0 flex-1 overflow-y-auto px-5 pb-3">
          <div className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_300px]">
          <div className="space-y-3">
            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">{t("newDialog.section.networkTiming")}</h3>
                <Badge variant="outline">{draft.network}</Badge>
              </div>
              <div className="grid gap-2 md:grid-cols-2">
                <div className="space-y-1.5 md:col-span-2">
                  <Label>{t("newDialog.field.network")}</Label>
                  <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-1 sm:grid-cols-3 xl:grid-cols-6">
                    {newTransactionNetworkOptions.map((network) => (
                      <button
                        key={network}
                        type="button"
                        aria-pressed={draft.network === network}
                        className={cn(
                          "min-w-0 truncate rounded-md px-2.5 py-1.5 text-center text-xs font-medium text-muted-foreground transition-colors hover:text-foreground",
                          draft.network === network && "bg-card text-foreground shadow-sm",
                        )}
                        onClick={() =>
                          updateDraft({
                            network,
                            sourceKind: sourceKindForNetwork(network),
                            asset:
                              network === "Liquid" && draft.asset === "BTC"
                                ? "LBTC"
                                : draft.asset,
                            receiveAsset:
                              network === "Liquid" && draft.receiveAsset === "BTC"
                                ? "LBTC"
                                : draft.receiveAsset,
                          })
                        }
                      >
                        {network}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="space-y-1.5 md:col-span-2">
                  <Label>{t("newDialog.field.flow")}</Label>
                  <div className="grid grid-cols-2 gap-1 rounded-lg bg-muted p-1 sm:grid-cols-3 xl:grid-cols-5">
                    {newTransactionFlowOptions.map((option) => (
                      <button
                        key={option.value}
                        type="button"
                        aria-pressed={draft.flow === option.value}
                        className={cn(
                          "min-w-0 truncate rounded-md px-2.5 py-1.5 text-center text-xs font-medium text-muted-foreground transition-colors hover:text-foreground",
                          draft.flow === option.value &&
                            "bg-card text-foreground shadow-sm",
                        )}
                        onClick={() => updateFlow(option.value)}
                      >
                        {/* loose translator */}
                        {(t as (key: string) => string)(option.label)}
                      </button>
                    ))}
                  </div>
                </div>

                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-occurred-at">{t("newDialog.field.occurredAt")}</Label>
                  <Input
                    id="new-txn-occurred-at"
                    type="datetime-local"
                    value={draft.occurredAt}
                    onChange={(event) => updateDraft({ occurredAt: event.target.value })}
                  />
                </div>
                {showConfirmedAt ? (
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-confirmed-at">{t("newDialog.field.confirmedAt")}</Label>
                    <Input
                      id="new-txn-confirmed-at"
                      type="datetime-local"
                      value={draft.confirmedAt}
                      onChange={(event) =>
                        updateDraft({ confirmedAt: event.target.value })
                      }
                    />
                  </div>
                ) : null}
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">{t("newDialog.section.partiesRoute")}</h3>
              {twoLegFlow ? (
                <div className="grid gap-2 md:grid-cols-3">
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-from-wallet">{t("newDialog.field.from")}</Label>
                    <Select
                      value={draft.fromWallet}
                      onValueChange={(value) =>
                        updateDraft({ fromWallet: value, wallet: value })
                      }
                    >
                      <SelectTrigger id="new-txn-from-wallet" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {ownWalletOptions.map((wallet) => (
                          <SelectItem key={wallet} value={wallet}>
                            {wallet}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-to-wallet">{t("newDialog.field.to")}</Label>
                    <Select
                      value={draft.toWallet}
                      onValueChange={(value) => updateDraft({ toWallet: value })}
                    >
                      <SelectTrigger id="new-txn-to-wallet" className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {ownWalletOptions.map((wallet) => (
                          <SelectItem key={wallet} value={wallet}>
                            {wallet}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="new-txn-swap-service">{t("newDialog.field.swapService")}</Label>
                    <Input
                      id="new-txn-swap-service"
                      value={draft.swapService}
                      onChange={(event) =>
                        updateDraft({ swapService: event.target.value })
                      }
                      placeholder={t("newDialog.field.swapServicePlaceholder")}
                    />
                  </div>
                </div>
              ) : (
                <div className="grid gap-2 md:grid-cols-2">
                  {draft.flow === "incoming" ? (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-external">{t("newDialog.field.from")}</Label>
                        <Input
                          id="new-txn-from-external"
                          value={draft.fromExternal}
                          onChange={(event) =>
                            updateDraft({
                              fromExternal: event.target.value,
                              counterparty: event.target.value,
                            })
                          }
                          placeholder={t("newDialog.field.fromExternalPlaceholder")}
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-wallet">{t("newDialog.field.to")}</Label>
                        <Select
                          value={draft.toWallet}
                          onValueChange={(value) =>
                            updateDraft({ toWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-to-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    </>
                  ) : draft.flow === "outgoing" ? (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-wallet">{t("newDialog.field.from")}</Label>
                        <Select
                          value={draft.fromWallet}
                          onValueChange={(value) =>
                            updateDraft({ fromWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-from-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-external">{t("newDialog.field.to")}</Label>
                        <Input
                          id="new-txn-to-external"
                          value={draft.toExternal}
                          onChange={(event) =>
                            updateDraft({
                              toExternal: event.target.value,
                              counterparty: event.target.value,
                            })
                          }
                          placeholder={t("newDialog.field.toExternalPlaceholder")}
                        />
                      </div>
                    </>
                  ) : (
                    <>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-from-wallet">{t("newDialog.field.from")}</Label>
                        <Select
                          value={draft.fromWallet}
                          onValueChange={(value) =>
                            updateDraft({ fromWallet: value, wallet: value })
                          }
                        >
                          <SelectTrigger id="new-txn-from-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-to-wallet">{t("newDialog.field.to")}</Label>
                        <Select
                          value={draft.toWallet}
                          onValueChange={(value) => updateDraft({ toWallet: value })}
                        >
                          <SelectTrigger id="new-txn-to-wallet" className="w-full">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            {ownWalletOptions.map((wallet) => (
                              <SelectItem key={wallet} value={wallet}>
                                {wallet}
                              </SelectItem>
                            ))}
                          </SelectContent>
                        </Select>
                      </div>
                    </>
                  )}
                </div>
              )}
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">{t("newDialog.section.amountPricing")}</h3>
              {twoLegFlow ? (
                <div className="grid gap-2 md:grid-cols-2">
                  <div className="rounded-md border bg-background p-2">
                    <h4 className="mb-2 text-xs font-semibold text-muted-foreground">
                      {t("newDialog.field.leg1Out")}
                    </h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-send-amount">{t("newDialog.field.sendSats")}</Label>
                        <Input
                          id="new-txn-send-amount"
                          inputMode="numeric"
                          value={draft.sendAmountSats}
                          onChange={(event) =>
                            updatePricingField("sendAmountSats", event.target.value)
                          }
                          placeholder="2450000"
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-send-asset">{t("newDialog.field.sendAsset")}</Label>
                        <Input
                          id="new-txn-send-asset"
                          value={draft.sendAsset}
                          onChange={(event) =>
                            updateDraft({ sendAsset: event.target.value })
                          }
                          placeholder="BTC"
                        />
                      </div>
                    </div>
                  </div>
                  <div className="rounded-md border bg-background p-2">
                    <h4 className="mb-2 text-xs font-semibold text-muted-foreground">
                      {t("newDialog.field.leg2In")}
                    </h4>
                    <div className="grid gap-2 sm:grid-cols-2">
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-receive-amount">{t("newDialog.field.receiveSats")}</Label>
                        <Input
                          id="new-txn-receive-amount"
                          inputMode="numeric"
                          value={draft.receiveAmountSats}
                          onChange={(event) =>
                            updatePricingField("receiveAmountSats", event.target.value)
                          }
                          placeholder="2450000"
                        />
                      </div>
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-receive-asset">{t("newDialog.field.receiveAsset")}</Label>
                        <Input
                          id="new-txn-receive-asset"
                          value={draft.receiveAsset}
                          onChange={(event) =>
                            updateDraft({ receiveAsset: event.target.value })
                          }
                          placeholder={draft.network === "Liquid" ? "LBTC" : "BTC"}
                        />
                      </div>
                    </div>
                  </div>
                </div>
              ) : null}

              <div className="mt-2 grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                {!twoLegFlow ? (
                  <>
                    <div className="grid gap-1.5">
                      <Label htmlFor="new-txn-amount">{t("newDialog.field.amountSats")}</Label>
                      <Input
                        id="new-txn-amount"
                        inputMode="numeric"
                        value={draft.amountSats}
                        onChange={(event) =>
                          updatePricingField("amountSats", event.target.value)
                        }
                        placeholder="2450000"
                      />
                    </div>
                    {showSingleAsset ? (
                      <div className="grid gap-1.5">
                        <Label htmlFor="new-txn-asset">{t("newDialog.field.asset")}</Label>
                        <Input
                          id="new-txn-asset"
                          value={draft.asset}
                          onChange={(event) =>
                            updateDraft({ asset: event.target.value })
                          }
                        />
                      </div>
                    ) : null}
                  </>
                ) : null}
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-fee">{t("newDialog.field.feeSats")}</Label>
                  <Input
                    id="new-txn-fee"
                    inputMode="numeric"
                    value={draft.feeSats}
                    onChange={(event) => updateDraft({ feeSats: event.target.value })}
                    placeholder="0"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-price">
                    {t("newDialog.field.pricePerBtc", { currency: draft.fiatCurrency })}
                  </Label>
                  <Input
                    id="new-txn-price"
                    inputMode="decimal"
                    value={draft.pricePerBtc}
                    onChange={(event) =>
                      updatePricingField("pricePerBtc", event.target.value)
                    }
                    placeholder="71420.18"
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-value">
                    {t("newDialog.field.totalValue", { currency: draft.fiatCurrency })}
                  </Label>
                  <Input
                    id="new-txn-value"
                    inputMode="decimal"
                    value={draft.totalValue}
                    onChange={(event) =>
                      updatePricingField("totalValue", event.target.value)
                    }
                    placeholder="1749.79"
                  />
                </div>
                <div className="grid gap-1.5 md:col-span-2 xl:col-span-1">
                  <Label>{t("newDialog.field.pricingMethod")}</Label>
                  <Select
                    value={pricingSelectionValue(
                      draft.pricingSourceKind,
                      draft.pricingQuality,
                    )}
                    onValueChange={(value) => {
                      const option = pricingOptionForValue(
                        value as PricingSelectionValue,
                        newTransactionPricingOptions,
                      );
                      updateDraft({
                        pricingSourceKind: option.sourceKind,
                        pricingQuality: option.quality,
                      });
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {newTransactionPricingOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {/* loose translator */}
                          {(t as (key: string) => string)(option.label)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">{t("newDialog.section.partOfMovement")}</h3>
                <Button
                  type="button"
                  variant={draft.movementId === "new" ? "default" : "outline"}
                  size="sm"
                  className="h-7"
                  onClick={() => updateDraft({ movementId: "new" })}
                >
                  {t("newDialog.newMovement")}
                </Button>
              </div>
              <div className="grid gap-2">
                <Input
                  value={
                    selectedMovement
                      ? // loose translator
                        (t as (key: string) => string)(selectedMovement.labelKey)
                      : draft.movementId === "new"
                        ? ""
                        : draft.movementId
                  }
                  onChange={(event) =>
                    updateDraft({ movementId: event.target.value })
                  }
                  placeholder={t("newDialog.field.movementPlaceholder")}
                />
                <div className="grid gap-1 sm:grid-cols-3">
                  {movementCandidates.map((candidate) => (
                    <button
                      key={candidate.id}
                      type="button"
                      className={cn(
                        "min-w-0 rounded-md border p-1.5 text-left text-[11px] transition-colors hover:bg-muted/40",
                        draft.movementId === candidate.id && "bg-muted",
                      )}
                      onClick={() => updateDraft({ movementId: candidate.id })}
                    >
                      <span className="block truncate font-medium">
                        {/* loose translator */}
                        {(t as (key: string) => string)(candidate.labelKey)}
                      </span>
                      <span className="block truncate text-muted-foreground">
                        {/* loose translator */}
                        {(t as (key: string) => string)(candidate.detailKey)}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 className="text-sm font-semibold">{t("newDialog.section.classification")}</h3>
                <Badge variant={taxClassification.taxable ? "default" : "outline"}>
                  {taxClassification.taxable
                    ? t("taxable.taxable")
                    : t("taxable.notTaxable")}
                </Badge>
              </div>
              <div className="grid gap-2 md:grid-cols-[minmax(150px,0.8fr)_minmax(220px,1.2fr)]">
                <div className="grid gap-1.5">
                  <Label>{t("newDialog.field.label")}</Label>
                  <Select
                    value={draft.label}
                    onValueChange={(value) => updateDraft({ label: value })}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {classificationOptions.map((option) => (
                        <SelectItem key={option} value={option}>
                          {classificationOptionLabelKeys[option]
                            ? // loose translator
                              (t as (key: string) => string)(
                                classificationOptionLabelKeys[option],
                              )
                            : option}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1.5">
                  <Label>{t("newDialog.field.taxTreatment")}</Label>
                  <Select
                    value={austrianSelectionValue(
                      draft.atRegime,
                      draft.atCategory,
                    )}
                    onValueChange={(value) => {
                      const option = austrianTaxClassificationForValue(value);
                      updateDraft({
                        atRegime: option.atRegime,
                        atCategory: option.atCategory,
                        taxable: option.taxable,
                      });
                    }}
                  >
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {austrianTaxClassificationOptions.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {/* loose translator */}
                          {(t as (key: string) => string)(option.label)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-tags">{t("newDialog.field.tags")}</Label>
                  <Input
                    id="new-txn-tags"
                    value={draft.tags}
                    onChange={(event) => updateDraft({ tags: event.target.value })}
                    placeholder={t("newDialog.field.tagsPlaceholder")}
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-note">{t("newDialog.field.note")}</Label>
                  <Textarea
                    id="new-txn-note"
                    value={draft.note}
                    onChange={(event) => updateDraft({ note: event.target.value })}
                    placeholder={t("newDialog.field.notePlaceholder")}
                  />
                </div>
              </div>
            </section>

            <section className="rounded-lg border p-2">
              <h3 className="mb-2 text-sm font-semibold">{t("newDialog.section.evidence")}</h3>
              <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-txid">{t("newDialog.field.evidenceTxid")}</Label>
                  <Input
                    id="new-txn-evidence-txid"
                    value={draft.evidence.txidOrPermalink}
                    onChange={(event) =>
                      updateEvidence({ txidOrPermalink: event.target.value })
                    }
                    placeholder={t("newDialog.field.evidenceTxidPlaceholder")}
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-btcpay">{t("newDialog.field.evidenceBtcpay")}</Label>
                  <Input
                    id="new-txn-evidence-btcpay"
                    value={draft.evidence.btcpayInvoiceId}
                    onChange={(event) =>
                      updateEvidence({ btcpayInvoiceId: event.target.value })
                    }
                    placeholder={t("newDialog.field.evidenceBtcpayPlaceholder")}
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-exchange">{t("newDialog.field.evidenceExchange")}</Label>
                  <Input
                    id="new-txn-evidence-exchange"
                    value={draft.evidence.exchangeCsvRow}
                    onChange={(event) =>
                      updateEvidence({ exchangeCsvRow: event.target.value })
                    }
                    placeholder={t("newDialog.field.evidenceExchangePlaceholder")}
                  />
                </div>
                <div className="grid gap-1.5">
                  <Label htmlFor="new-txn-evidence-swap">{t("newDialog.field.evidenceSwap")}</Label>
                  <Input
                    id="new-txn-evidence-swap"
                    value={draft.evidence.swapId}
                    onChange={(event) => updateEvidence({ swapId: event.target.value })}
                    placeholder={t("newDialog.field.evidenceSwapPlaceholder")}
                  />
                </div>
                <div className="grid gap-1.5 md:col-span-2">
                  <Label htmlFor="new-txn-evidence-preimage">{t("newDialog.field.evidencePreimage")}</Label>
                  <Input
                    id="new-txn-evidence-preimage"
                    value={draft.evidence.preimage}
                    onChange={(event) =>
                      updateEvidence({ preimage: event.target.value })
                    }
                    placeholder={t("newDialog.field.evidencePreimagePlaceholder")}
                  />
                </div>
              </div>
            </section>
          </div>

          <aside className="space-y-2.5 rounded-lg border bg-muted/20 p-2.5 lg:sticky lg:top-0 lg:self-start">
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="text-xs text-muted-foreground">{t("newDialog.preview.live")}</p>
                <p className="truncate text-lg font-semibold">
                  {t(transactionFlowLabels[draft.flow])}
                </p>
                <p className="truncate text-xs text-muted-foreground">
                  {fromDisplay} → {toDisplay}
                </p>
              </div>
              {isExternalPricingSource(
                draft.pricingSourceKind,
                draft.pricingQuality,
              ) ? (
                <Badge
                  className={cn(
                    pricingSourceStyles[
                      pricingSelectionValue(
                        draft.pricingSourceKind,
                        draft.pricingQuality,
                      )
                    ],
                  )}
                >
                  {/* loose translator */}
                  {(t as (key: string) => string)(
                    pricingSourceLabel(
                      draft.pricingSourceKind,
                      draft.pricingQuality,
                      newTransactionPricingOptions,
                    ),
                  )}
                </Badge>
              ) : null}
            </div>

            <div className="rounded-md border bg-background p-3">
              {twoLegFlow ? (
                <div className="grid gap-2 text-sm">
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">{t("newDialog.preview.out")}</span>
                    <span className="truncate text-right font-semibold">
                      {formatAssetAmount(sendLegBtc, draft.sendAsset)}
                    </span>
                  </div>
                  <div className="flex items-center justify-between gap-3">
                    <span className="text-muted-foreground">{t("newDialog.preview.in")}</span>
                    <span className="truncate text-right font-semibold">
                      {formatAssetAmount(
                        receiveLegBtc,
                        draft.receiveAsset || inferredAssetForDraft(draft),
                      )}
                    </span>
                  </div>
                </div>
              ) : (
                <>
                  <p
                    className={cn(
                      "text-xl font-semibold",
                      draft.flow === "incoming"
                        ? "text-emerald-500"
                        : draft.flow === "outgoing"
                          ? "text-rose-400"
                          : "text-foreground",
                    )}
                  >
                    {draft.flow === "outgoing"
                      ? "− "
                      : draft.flow === "incoming"
                        ? "+ "
                        : ""}
                    {formatBtcAmount(Math.abs(movementBtc))}
                  </p>
                  <p className="mt-1 text-sm text-muted-foreground">
                    {t("newDialog.preview.net", {
                      value: formatBtc(signedBtc, { sign: true }),
                    })}
                  </p>
                </>
              )}
              <p className="mt-2 text-sm text-muted-foreground">
                {totalValue !== null
                  ? formatDraftFiat(totalValue, draft.fiatCurrency)
                  : t("newDialog.preview.noFiatValue", {
                      currency: draft.fiatCurrency,
                    })}
              </p>
            </div>

            <div className="grid gap-2 text-sm">
              <PreviewRow label={t("newDialog.preview.network")} value={draft.network} />
              <PreviewRow label={t("newDialog.preview.from")} value={fromDisplay} />
              <PreviewRow label={t("newDialog.preview.to")} value={toDisplay} />
              {draft.swapService ? (
                <PreviewRow label={t("newDialog.preview.service")} value={draft.swapService} />
              ) : null}
              <PreviewRow
                label={t("newDialog.preview.movement")}
                value={
                  <button
                    type="button"
                    className="truncate underline-offset-4 hover:underline"
                    onClick={() =>
                      updateDraft({
                        movementId: draft.movementId ? "" : "new",
                      })
                    }
                  >
                    {movementLabel}
                  </button>
                }
              />
              <PreviewRow
                label={t("newDialog.preview.asset")}
                value={
                  twoLegFlow
                    ? `${draft.sendAsset || "BTC"} → ${
                        draft.receiveAsset || inferredAssetForDraft(draft)
                      }`
                    : inferredAssetForDraft(draft)
                }
              />
              <PreviewRow
                label={t("newDialog.preview.fee")}
                value={feeBtc ? formatBtcAmount(feeBtc) : "-"}
              />
              <PreviewRow
                label={t("newDialog.preview.value", { currency: draft.fiatCurrency })}
                value={
                  totalValue !== null
                    ? formatDraftFiat(totalValue, draft.fiatCurrency)
                    : "-"
                }
              />
              <PreviewRow
                label={t("newDialog.preview.price", { currency: draft.fiatCurrency })}
                value={
                  priceValue !== null
                    ? t("newDialog.preview.pricePerBtc", {
                        value: formatDraftFiat(priceValue, draft.fiatCurrency),
                      })
                    : "-"
                }
              />
              <PreviewRow
                label={t("newDialog.preview.pricing")}
                // loose translator
                value={(t as (key: string) => string)(
                  pricingSourceLabel(
                    draft.pricingSourceKind,
                    draft.pricingQuality,
                    newTransactionPricingOptions,
                  ),
                )}
              />
              <PreviewRow
                label={t("newDialog.preview.tax")}
                // loose translator
                value={(t as (key: string) => string)(
                  taxClassification.shortLabel,
                )}
              />
              {primaryEvidence ? (
                <PreviewRow label={t("newDialog.preview.evidence")} value={primaryEvidence} />
              ) : null}
            </div>

            <div className="grid gap-1.5">
              <Label>{t("newDialog.status")}</Label>
              <Select
                value={draft.reviewStatus}
                onValueChange={(value) =>
                  updateDraft({ reviewStatus: value as TransactionStatus })
                }
              >
                <SelectTrigger className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {allTransactionStatuses.map((status) => (
                    <SelectItem key={status} value={status}>
                      {t(transactionStatusLabels[status])}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="flex flex-wrap gap-1">
              <Badge variant="secondary">
                {classificationOptionLabelKeys[draft.label]
                  ? // loose translator
                    (t as (key: string) => string)(
                      classificationOptionLabelKeys[draft.label],
                    )
                  : draft.label}
              </Badge>
              <Badge variant={taxClassification.taxable ? "default" : "outline"}>
                {taxClassification.taxable
                  ? t("taxable.taxable")
                  : t("taxable.notTaxable")}
              </Badge>
              {tags.map((tag) => (
                <Badge key={tag} variant="outline">
                  {tag}
                </Badge>
              ))}
            </div>
          </aside>
          </div>
        </div>

        <DialogFooter className="shrink-0 border-t bg-background/95 px-5 py-2.5 backdrop-blur sm:items-center sm:justify-between">
          <div className="text-left text-xs text-muted-foreground">
            {t("newDialog.footer.demoNote")}
          </div>
          <DialogClose asChild>
            <Button type="button" variant="outline">
              {t("common:actions.cancel")}
            </Button>
          </DialogClose>
          <Button type="button" onClick={onSaveDraft}>
            {t("newDialog.footer.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function PreviewRow({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b pb-2 last:border-b-0 last:pb-0">
      <span className="text-muted-foreground">{label}</span>
      <span className="min-w-0 truncate text-right font-medium">{value}</span>
    </div>
  );
}
