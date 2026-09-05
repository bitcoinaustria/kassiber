// The four work surfaces of the case dossier. Each stage is purely
// presentational over the shared case state; the rail owns navigation.

import {
  AlertTriangle,
  Check,
  FileCheck,
  FileDown,
  GitBranch,
  Info,
  Link2,
  Plus,
  RefreshCw,
  Search,
  ShieldAlert,
  SlidersHorizontal,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Collapsible,
  CollapsibleContent,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";

import {
  CONFIDENCE_LEVELS,
  LINK_TYPES,
  SOURCE_TYPES,
  formatBtc,
  pretty,
  txRef,
  type SourceFundsFinding,
} from "./model";
import {
  CaseBrief,
  CoveragePanel,
  DisclosureList,
  DisclosureNarrative,
  DisclosureNodeOverrides,
  DisclosureTxidList,
  EmptyState,
  EvidenceSelect,
  Field,
  FlowLevelDetailPreview,
  GateRow,
  OptionalSection,
  PurposeButton,
  RecipientPicker,
  RecipientPreferenceAdvisory,
  ReportControlFields,
  ReportDiagram,
  SelectField,
  StatusPill,
  TracedCoverageHero,
  TransactionSelect,
  TransactionTargetHeader,
  TransactionTargetRow,
} from "./panels";
import { type SourceFundsCaseState } from "./useSourceFundsCase";

function StageHeader({
  title,
  lede,
  children,
}: {
  title: string;
  lede: string;
  children?: React.ReactNode;
}) {
  return (
    <header className="flex flex-wrap items-start justify-between gap-3 border-b pb-4">
      <div className="min-w-0">
        <h1 className="text-lg font-semibold tracking-tight">{title}</h1>
        <p className="mt-0.5 max-w-2xl text-sm text-muted-foreground">{lede}</p>
      </div>
      {children && <div className="flex flex-wrap gap-2">{children}</div>}
    </header>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 1 — Target                                                    */
/* ------------------------------------------------------------------ */

export function TargetStage({ state }: { state: SourceFundsCaseState }) {
  const { t } = useTranslation("sourceFunds");
  const planned = state.reportPurpose === "planned_exchange_sale";
  const targetLabel = t(planned ? "setup.targetLabelPlanned" : "setup.targetLabelExisting");
  const amountLabel = t(planned ? "setup.amountLabelPlanned" : "setup.amountLabelExisting");

  return (
    <div className="space-y-5">
      <StageHeader
        title={t("workstation.whatNeedsExplaining")}
        lede={t("workstation.everyDossierIsAnchoredTo")}
      />

      <div className="grid gap-3 md:grid-cols-2">
        <PurposeButton
          active={planned}
          title={t("purpose.planned.title")}
          body={t("purpose.planned.body")}
          onClick={() => state.setReportPurpose("planned_exchange_sale")}
        />
        <PurposeButton
          active={!planned}
          title={t("purpose.existing.title")}
          body={t("purpose.existing.body")}
          onClick={() => state.setReportPurpose("existing_transaction")}
        />
      </div>

      {planned && (
        <div className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2.5 text-sm text-muted-foreground">
          <Info className="mt-0.5 size-4 shrink-0 opacity-70" aria-hidden="true" />
          <p>
            {t("purpose.plannedHint")}</p>
        </div>
      )}

      <div className="grid gap-3 sm:grid-cols-[180px_150px]">
        <ReportControlFields
          amountLabel={amountLabel}
          targetAmount={state.targetAmount}
          selectedTx={state.selectedTx}
          revealMode={state.revealMode}
          onAmountChange={state.setTargetAmount}
          onRevealModeChange={state.setRevealMode}
        />
      </div>

      {planned && (
        <div className="grid gap-3 md:grid-cols-[220px_minmax(0,1fr)]">
          <Field label={t("setup.exchangeBroker.label")} htmlFor="planned-destination">
            <Input
              id="planned-destination"
              value={state.plannedDestination}
              onChange={(event) =>
                state.setPlannedDestination(event.target.value)
              }
              placeholder={t("workstation.krakenBitpandaOtcDesk")}
            />
          </Field>
          <Field label={t("setup.bankNote.label")} htmlFor="planned-note">
            <Input
              id="planned-note"
              value={state.plannedNote}
              onChange={(event) => state.setPlannedNote(event.target.value)}
              placeholder={t("setup.bankNote.placeholder")}
            />
          </Field>
        </div>
      )}

      <Field label={t("case.targetReference")} htmlFor="source-funds-target-reference">
        <Input id="source-funds-target-reference" value={state.target} onChange={(event) => state.setTarget(event.target.value)} placeholder={t("case.targetReferenceHint")} />
      </Field>
      {state.selectedTarget && state.resolvedTarget.isError && <p role="alert" className="text-sm text-destructive">{t("case.targetUnavailable")}</p>}
      <div className="kb-surface-inset overflow-hidden">
        <div className="flex flex-col gap-3 border-b p-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <div className="text-sm font-medium">{targetLabel}</div>
            <div className="text-xs text-muted-foreground">
              {t("case.loadedTransactions", { count: state.rows.length })}
            </div>
          </div>
          <div className="space-y-3">
            <div className="flex flex-wrap gap-2">
              <div className="relative min-w-[220px] flex-1">
                <Search
                  className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
                  aria-hidden="true"
                />
                <Input
                  type="search"
                  value={state.targetSearch}
                  onChange={(event) => state.setTargetSearch(event.target.value)}
                  placeholder={t("workstation.searchTxidWalletNote")}
                  className="h-8 pl-9"
                />
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-8"
                onClick={() =>
                  state.setShowAdvancedTargetFilters(
                    !state.showAdvancedTargetFilters,
                  )
                }
                aria-expanded={state.showAdvancedTargetFilters}
              >
                <SlidersHorizontal className="mr-2 size-4" aria-hidden="true" />
                {t("filters.title")}</Button>
              {state.targetFiltersActive && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8"
                  onClick={state.clearTargetFilters}
                >
                  <X className="mr-2 size-4" aria-hidden="true" />
                  {t("filters.clear")}</Button>
              )}
            </div>
            <Collapsible
              open={state.showAdvancedTargetFilters}
              onOpenChange={state.setShowAdvancedTargetFilters}
            >
              <CollapsibleContent className="grid gap-2 border-t pt-3 sm:grid-cols-2 xl:grid-cols-5">
                <Select
                  value={state.targetDirectionFilter}
                  onValueChange={state.setTargetDirectionFilter}
                >
                  <SelectTrigger className="h-8 w-full" aria-label={t("filters.direction.ariaLabel")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.direction.all")}</SelectItem>
                    <SelectItem value="incoming">{t("flow.incoming")}</SelectItem>
                    <SelectItem value="outgoing">{t("flow.outgoing")}</SelectItem>
                    <SelectItem value="transfer">{t("flow.transfer")}</SelectItem>
                    <SelectItem value="swap">{t("linkType.swap")}</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetDateFilter}
                  onValueChange={state.setTargetDateFilter}
                >
                  <SelectTrigger className="h-8 w-full" aria-label={t("filters.date.ariaLabel")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.date.all")}</SelectItem>
                    <SelectItem value="today">{t("filters.date.today")}</SelectItem>
                    <SelectItem value="yesterday">{t("filters.date.yesterday")}</SelectItem>
                    <SelectItem value="7days">{t("filters.date.last7days")}</SelectItem>
                    <SelectItem value="30days">{t("filters.date.last30days")}</SelectItem>
                    <SelectItem value="older">{t("filters.date.older")}</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetStatusFilter}
                  onValueChange={state.setTargetStatusFilter}
                >
                  <SelectTrigger className="h-8 w-full" aria-label={t("filters.status.ariaLabel")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.status.all")}</SelectItem>
                    <SelectItem value="confirmed">{t("filters.status.confirmed")}</SelectItem>
                    <SelectItem value="pending">{t("filters.status.pending")}</SelectItem>
                    <SelectItem value="review">{t("filters.status.review")}</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetNetworkFilter}
                  onValueChange={state.setTargetNetworkFilter}
                >
                  <SelectTrigger className="h-8 w-full" aria-label={t("filters.network.ariaLabel")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.network.all")}</SelectItem>
                    {state.targetNetworkOptions.map((network) => (
                      <SelectItem key={network} value={network}>
                        {network}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetAssetFilter}
                  onValueChange={state.setTargetAssetFilter}
                >
                  <SelectTrigger className="h-8 w-full" aria-label={t("filters.asset.ariaLabel")}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.asset.all")}</SelectItem>
                    {state.targetAssetOptions.map((asset) => (
                      <SelectItem key={asset} value={asset}>
                        {asset}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetWalletFilter}
                  onValueChange={state.setTargetWalletFilter}
                >
                  <SelectTrigger
                    className="h-8 w-full xl:col-span-2"
                    aria-label={t("filters.wallet.ariaLabel")}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">{t("filters.wallet.all")}</SelectItem>
                    {state.targetWalletOptions.map((wallet) => (
                      <SelectItem key={wallet} value={wallet}>
                        {wallet}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </CollapsibleContent>
            </Collapsible>
          </div>
        </div>
        {state.transactions.isError && <p role="alert" className="p-3 text-sm text-destructive">{t("case.transactionsError")}</p>}
        <TransactionTargetHeader />
        <div className="max-h-[430px] overflow-y-auto">
          {state.filteredTargetRows.length === 0 ? (
            <div className="p-3">
              <EmptyState text={t("setup.noMatches")} />
            </div>
          ) : (
            <div className="divide-y">
              {state.filteredTargetRows.map((row) => (
                <TransactionTargetRow
                  key={txRef(row)}
                  row={row}
                  active={txRef(row) === state.selectedTarget}
                  onSelect={() => { state.setTarget(txRef(row)); state.setStage("trace"); }}
                  onOpenDetails={() => {
                    state.openTxDetailById(txRef(row));
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {state.transactions.hasNextPage && <Button variant="outline" disabled={state.transactions.isFetchingNextPage} onClick={() => void state.transactions.fetchNextPage()}>{state.transactions.isFetchingNextPage ? t("case.loading") : t("case.loadMore")}</Button>}
      <OptionalSection
        open={state.showCoverage}
        onOpenChange={state.setShowCoverage}
        icon={<GitBranch className="size-4" aria-hidden="true" />}
        title={t("coverage.sectionTitle")}
        summary={state.coverageQuery.data?.data ? t("coverage.summary", { amount: state.coverageQuery.data.data.totals.buckets.fully_traced.amount.toFixed(8), count: state.coverageQuery.data.data.totals.tx_count }) : t("coverage.summaryEmpty")}
      >
        <TracedCoverageHero coverage={state.coverageQuery.data?.data} />
        <CoveragePanel
          coverage={state.coverageQuery.data?.data}
          loading={state.coverageQuery.isLoading}
        />
      </OptionalSection>

    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 2 — Trace                                                     */
/* ------------------------------------------------------------------ */

export function TraceStage({ state }: { state: SourceFundsCaseState }) {
  const { t } = useTranslation("sourceFunds");
  const assembled = state.assembleLinks.data?.data;
  const gaps: SourceFundsFinding[] = [...state.blockers, ...state.warnings];
  const methodLabel = (method: string) =>
    t(`method.${method}`, { defaultValue: pretty(method) });

  const dispatchGapAction = (action: string, gap: SourceFundsFinding) => {
    if (action === "open_source_creator") {
      state.prefillGapForm(gap);
      state.setShowAdvancedReview(true);
      return;
    }
    if (action === "open_link_review") {
      if (gap.ref && state.links.some((link) => link.id === gap.ref)) {
        state.setSelectedLinkId(gap.ref);
      } else if (gap.ref && state.txById.has(gap.ref)) {
        state.openTxDetailById(gap.ref);
      }
      state.setShowAdvancedReview(true);
      return;
    }
    if (action === "open_review_queue" || action === "open_source") {
      state.setShowAdvancedReview(true);
      return;
    }
    if (action === "open_transaction" && gap.ref && state.txById.has(gap.ref)) {
      state.openTxDetailById(gap.ref);
    }
  };

  return (
    <div className="space-y-5">
      <StageHeader
        title={t("caseStages.trace.hint")}
        lede={t("workstation.kassiberProvesEveryHopIt")}
      >
        <Button
          type="button"
          variant="outline"
          onClick={() => void state.runAssembly()}
          disabled={!state.selectedTarget || state.assembleLinks.isPending}
        >
          <GitBranch className="mr-2 size-4" aria-hidden="true" />
          {t(state.assembleLinks.isPending ? "actionsBar.assembling" : "actionsBar.assemble")}
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={() => {
            state.prefillGapForm();
            state.setShowAdvancedReview(true);
          }}
        >
          <AlertTriangle className="mr-2 size-4" aria-hidden="true" />
          {t("actionsBar.markGap")}</Button>
      </StageHeader>

      {assembled && (
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 rounded-md border bg-muted/30 px-3 py-2 text-sm">
          <span className="font-semibold">
            {t("actionsBar.assembleResult", { count: assembled.auto_reviewed })}
          </span>
          <span className="text-xs text-muted-foreground">
            {Object.entries(assembled.methods)
              .map(([method, count]) => `${methodLabel(method)} ×${count}`)
              .join(" · ") || t("actionsBar.assembleResultEmpty")}
          </span>
          {assembled.awaiting_manual_review > 0 && (
            <span className="text-xs text-muted-foreground">
              {t("actionsBar.manualReviewHint", { count: assembled.awaiting_manual_review })}
            </span>
          )}
        </div>
      )}

      <details className="rounded-md border p-3"><summary className="cursor-pointer text-sm font-medium">{t("case.evidenceSection")}</summary><div className="pt-3"><CaseBrief
        report={state.report}
        bulkReviewable={state.bulkReviewableSuggestions.length}
        manualReview={state.manualSuggestionCount}
        onOpenTransaction={state.openTxDetailById}
      /></div></details>

      <section id="source-funds-findings" aria-label={t("case.findingsTitle")} className="space-y-2">
        <div className="flex items-baseline justify-between">
          <h2 className="text-sm font-semibold uppercase tracking-[0.1em] text-muted-foreground">
            {t("workstation.workLedger")}</h2>
          <span className="text-xs text-muted-foreground">
            {t("case.findings", { blockers: state.blockers.length, warnings: state.warnings.length })}
          </span>
        </div>
        {state.preview.isLoading && (
          <EmptyState text={t("workstation.buildingReviewedFlow")} />
        )}
        {state.preview.isError && (
          <GateRow
            finding={{
              code: "preview_unavailable",
              message: t("gates.previewUnavailable"),
            }}
          />
        )}
        {!state.preview.isLoading && !state.preview.isError && gaps.length === 0 && state.report && (
          <div className="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200">
            {t("case.noBlockers")}
          </div>
        )}
        <div className="grid gap-2 xl:grid-cols-2">
          {gaps.map((finding) => (
            <GateRow
              key={`${finding.code}-${finding.ref ?? ""}-${finding.message}`}
              finding={finding}
              onOpenTransaction={
                finding.ref && state.txById.has(finding.ref)
                  ? () => state.openTxDetailById(finding.ref as string)
                  : undefined
              }
              onAction={dispatchGapAction}
            />
          ))}
        </div>
      </section>

      <OptionalSection
        open={state.showAdvancedReview}
        onOpenChange={state.setShowAdvancedReview}
        icon={<SlidersHorizontal className="size-4" aria-hidden="true" />}
        title={t("advancedReview.title")}
        summary={t("advancedReview.summary", { links: state.reviewQueueLinks.length, sources: state.sources.length, evidence: state.evidence.length })}
      >
        <AdvancedReviewEditor state={state} />
      </OptionalSection>

    </div>
  );
}

function AdvancedReviewEditor({ state }: { state: SourceFundsCaseState }) {
  const { t } = useTranslation("sourceFunds");
  const methodLabel = (method: string) =>
    t(`method.${method}`, { defaultValue: pretty(method) });

  return (
    <div className="space-y-4">
      <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_420px]">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <Link2 className="size-4" aria-hidden="true" />
              {t("reviewQueue.title")}</CardTitle>
            <CardDescription>
              {t("reviewQueue.description")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => void state.runSuggestions()}
                disabled={!state.selectedTarget || state.suggestLinks.isPending}
              >
                <RefreshCw className="mr-2 size-4" aria-hidden="true" />
                {t("actionsBar.findLinks")}</Button>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => void state.bulkReviewDeterministicLinks()}
                disabled={
                  !state.selectedTarget ||
                  state.bulkReviewLinks.isPending ||
                  state.bulkReviewableSuggestions.length === 0
                }
              >
                <Check className="mr-2 size-4" aria-hidden="true" />
                {t("actionsBar.reviewDeterministic")}</Button>
            </div>
            {state.reviewQueueLinks.length === 0 ? (
              <EmptyState text={t("workstation.noMatchedLinksYetAssemble")} />
            ) : (
              state.reviewQueueLinks.map((link) => (
                <button
                  key={link.id}
                  type="button"
                  className={[
                    "w-full rounded-md border px-3 py-2 text-left text-sm transition-colors",
                    link.id === state.selectedLink?.id
                      ? "border-primary bg-primary/5"
                      : "hover:bg-muted/60",
                  ].join(" ")}
                  onClick={() => state.setSelectedLinkId(link.id)}
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <StatusPill state={link.state} />
                    <span className="rounded-md bg-muted px-2 py-0.5 text-xs text-muted-foreground">
                      {state.reachableLinkIds.has(link.id)
                        ? t("reviewQueue.badge.path")
                        : link.to_transaction_id === state.selectedTxId
                          ? t("reviewQueue.badge.target")
                          : t("reviewQueue.badge.suggested")}
                    </span>
                    <span className="font-medium">{t(`linkType.${link.link_type}`, { defaultValue: pretty(link.link_type) })}</span>
                    <span className="text-muted-foreground">
                      {methodLabel(link.method)}
                    </span>
                  </div>
                  <div className="mt-2 grid gap-1 text-xs text-muted-foreground">
                    <span>
                      {link.from_source_id
                        ? state.sourceName(link.from_source_id)
                        : state.txName(link.from_transaction_id)}{" "}
                      {"->"} {state.txName(link.to_transaction_id)}
                    </span>
                    <span>
                      {formatBtc(link.allocation_amount ?? null, link.asset)} ·{" "}
                      {t(`confidence.${link.confidence}`, { defaultValue: pretty(link.confidence) })}
                    </span>
                  </div>
                </button>
              ))
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <FileCheck className="size-4" aria-hidden="true" />
              {t("linkReview.title")}</CardTitle>
            <CardDescription>
              {t("linkReview.description")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            {!state.selectedLink ? (
              <EmptyState text={t("linkReview.empty")} />
            ) : (
              <>
                <div className="rounded-md border p-3 text-sm">
                  <div className="font-medium">
                    {state.selectedSource?.label ??
                      state.txName(state.selectedLink.from_transaction_id)}
                  </div>
                  <div className="text-muted-foreground">
                    to {state.txName(state.selectedLink.to_transaction_id)}
                  </div>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <SelectField
                    id="review-link-type"
                    label={t("manualLink.type")}
                    value={state.linkForm.link_type}
                    options={LINK_TYPES}
                    onChange={(value) =>
                      state.setLinkForm((current) => ({
                        ...current,
                        link_type: value,
                      }))
                    }
                  />
                  <SelectField
                    id="review-confidence"
                    label={t("manualLink.confidence")}
                    value={state.linkForm.confidence}
                    options={CONFIDENCE_LEVELS}
                    onChange={(value) =>
                      state.setLinkForm((current) => ({
                        ...current,
                        confidence: value,
                      }))
                    }
                  />
                  <Field label={t("manualLink.allocation")} htmlFor="review-allocation">
                    <Input
                      id="review-allocation"
                      value={state.linkForm.allocation_amount}
                      onChange={(event) =>
                        state.setLinkForm((current) => ({
                          ...current,
                          allocation_amount: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label={t("manualLink.fromAmount")} htmlFor="review-from-allocation">
                    <Input
                      id="review-from-allocation"
                      value={state.linkForm.from_allocation_amount}
                      onChange={(event) =>
                        state.setLinkForm((current) => ({
                          ...current,
                          from_allocation_amount: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
                <EvidenceSelect
                  id="review-evidence"
                  value={state.linkForm.attachment_id}
                  evidence={state.evidence}
                  onChange={(value) =>
                    state.setLinkForm((current) => ({
                      ...current,
                      attachment_id: value,
                    }))
                  }
                />
                <Field label={t("manualLink.reviewNote")} htmlFor="review-note">
                  <Textarea
                    id="review-note"
                    value={state.linkForm.explanation}
                    onChange={(event) =>
                      state.setLinkForm((current) => ({
                        ...current,
                        explanation: event.target.value,
                      }))
                    }
                  />
                </Field>
                <div className="grid gap-2 sm:grid-cols-2">
                  <Button
                    type="button"
                    onClick={() => void state.reviewSelectedLink("reviewed")}
                    disabled={
                      state.reviewLink.isPending || state.attachLink.isPending
                    }
                  >
                    <Check className="mr-2 size-4" aria-hidden="true" />
                    {t("linkReview.accept")}</Button>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => void state.reviewSelectedLink("rejected")}
                    disabled={state.reviewLink.isPending}
                  >
                    <X className="mr-2 size-4" aria-hidden="true" />
                    {t("linkReview.reject")}</Button>
                </div>
              </>
            )}
          </CardContent>
        </Card>
      </div>
      <div className="grid gap-4 2xl:grid-cols-2">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <Plus className="size-4" aria-hidden="true" />
              {t("sourceOrGap.title")}</CardTitle>
            <CardDescription>
              {t("sourceOrGap.description")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <SelectField
                id="source-type"
                label={t("sourceOrGap.sourceType")}
                value={state.sourceForm.source_type}
                options={SOURCE_TYPES}
                onChange={(value) =>
                  state.setSourceForm((current) => ({
                    ...current,
                    source_type: value,
                    link_type:
                      value === "missing_history"
                        ? "missing_history"
                        : current.link_type === "missing_history"
                          ? "manual_source"
                          : current.link_type,
                  }))
                }
              />
              <SelectField
                id="source-link-type"
                label={t("sourceOrGap.linkType")}
                value={state.sourceForm.link_type}
                options={LINK_TYPES}
                onChange={(value) =>
                  state.setSourceForm((current) => ({
                    ...current,
                    link_type: value,
                  }))
                }
              />
              <Field label={t("sourceOrGap.label")} htmlFor="source-label">
                <Input
                  id="source-label"
                  value={state.sourceForm.label}
                  onChange={(event) =>
                    state.setSourceForm((current) => ({
                      ...current,
                      label: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label={t("sourceOrGap.amount")} htmlFor="source-amount">
                <Input
                  id="source-amount"
                  value={state.sourceForm.amount}
                  onChange={(event) =>
                    state.setSourceForm((current) => ({
                      ...current,
                      amount: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label={t("sourceOrGap.asset")} htmlFor="source-asset">
                <Input
                  id="source-asset"
                  value={state.sourceForm.asset}
                  onChange={(event) =>
                    state.setSourceForm((current) => ({
                      ...current,
                      asset: event.target.value,
                    }))
                  }
                />
              </Field>
              <TransactionSelect
                id="source-to"
                label={t("sourceOrGap.appliesTo")}
                rows={state.rows}
                value={state.sourceForm.to_transaction}
                defaultLabel={t("case.useTarget")}
                onChange={(value) =>
                  state.setSourceForm((current) => ({
                    ...current,
                    to_transaction: value,
                  }))
                }
              />
            </div>
            <EvidenceSelect
              id="source-evidence"
              value={state.sourceForm.attachment_id}
              evidence={state.evidence}
              onChange={(value) =>
                state.setSourceForm((current) => ({
                  ...current,
                  attachment_id: value,
                }))
              }
            />
            <Field label={t("sourceOrGap.evidenceNote")} htmlFor="source-description">
              <Textarea
                id="source-description"
                value={state.sourceForm.description}
                onChange={(event) =>
                  state.setSourceForm((current) => ({
                    ...current,
                    description: event.target.value,
                  }))
                }
              />
            </Field>
            <Button
              type="button"
              className="w-full"
              onClick={() => void state.createSourceLink()}
              disabled={
                state.createSource.isPending ||
                state.createLink.isPending ||
                !state.sourceForm.label.trim() ||
                !state.sourceForm.amount.trim()
              }
            >
              <Plus className="mr-2 size-4" aria-hidden="true" />
              {t("sourceOrGap.create")}</Button>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <Link2 className="size-4" aria-hidden="true" />
              {t("manualLink.title")}</CardTitle>
            <CardDescription>
              {t("manualLink.description")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <TransactionSelect
                id="manual-from"
                label={t("manualLink.from")}
                rows={state.rows}
                value={state.manualLinkForm.from_transaction}
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    from_transaction: value,
                  }))
                }
              />
              <TransactionSelect
                id="manual-to"
                label="To"
                rows={state.rows}
                value={state.manualLinkForm.to_transaction}
                defaultLabel={t("case.useTarget")}
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    to_transaction: value,
                  }))
                }
              />
              <SelectField
                id="manual-type"
                label={t("manualLink.type")}
                value={state.manualLinkForm.link_type}
                options={LINK_TYPES}
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    link_type: value,
                  }))
                }
              />
              <SelectField
                id="manual-confidence"
                label={t("manualLink.confidence")}
                value={state.manualLinkForm.confidence}
                options={CONFIDENCE_LEVELS}
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    confidence: value,
                  }))
                }
              />
              <Field label={t("manualLink.allocation")} htmlFor="manual-allocation">
                <Input
                  id="manual-allocation"
                  value={state.manualLinkForm.allocation_amount}
                  onChange={(event) =>
                    state.setManualLinkForm((current) => ({
                      ...current,
                      allocation_amount: event.target.value,
                    }))
                  }
                />
              </Field>
              <Field label={t("manualLink.fromAmount")} htmlFor="manual-from-amount">
                <Input
                  id="manual-from-amount"
                  value={state.manualLinkForm.from_allocation_amount}
                  onChange={(event) =>
                    state.setManualLinkForm((current) => ({
                      ...current,
                      from_allocation_amount: event.target.value,
                    }))
                  }
                />
              </Field>
            </div>
            <EvidenceSelect
              id="manual-evidence"
              value={state.manualLinkForm.attachment_id}
              evidence={state.evidence}
              onChange={(value) =>
                state.setManualLinkForm((current) => ({
                  ...current,
                  attachment_id: value,
                }))
              }
            />
            <Field label={t("manualLink.reviewNote")} htmlFor="manual-note">
              <Textarea
                id="manual-note"
                value={state.manualLinkForm.explanation}
                onChange={(event) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    explanation: event.target.value,
                  }))
                }
              />
            </Field>
            <Button
              type="button"
              className="w-full"
              onClick={() => void state.createManualLink()}
              disabled={
                state.createLink.isPending ||
                !state.manualLinkForm.from_transaction ||
                !state.manualLinkForm.allocation_amount.trim()
              }
            >
              <Plus className="mr-2 size-4" aria-hidden="true" />
              {t("manualLink.add")}</Button>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 3 — Disclose                                                  */
/* ------------------------------------------------------------------ */

export function DiscloseStage({ state }: { state: SourceFundsCaseState }) {
  const { t } = useTranslation("sourceFunds");
  return (
    <div className="space-y-5">
      <StageHeader
        title={t("caseStages.disclose.hint")}
        lede={t("workstation.everythingBelowIsExactlyWhat")}
      />

      <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_340px]">
        <div className="space-y-4">
          <DisclosureNarrative report={state.report} />
          <DisclosureTxidList report={state.report} />
          <DisclosureNodeOverrides
            report={state.report}
            overrides={state.revealOverrides}
            onChange={(id, decision) =>
              state.setRevealOverrides((current) => {
                const next = { ...current };
                if (decision) {
                  next[id] = decision;
                } else {
                  delete next[id];
                }
                return next;
              })
            }
          />
          <DisclosureList
            label={t("evidence.label")}
            values={(state.report?.disclosure_preview.attachments ?? []).map(
              (item) => item.label,
            )}
          />
          <DisclosureList
            label={t("disclosure.excluded")}
            values={state.report?.disclosure_preview.excluded ?? []}
          />
          {state.report?.disclosure_preview.privacy_note && (
            <p className="rounded-md border px-3 py-2 text-xs text-muted-foreground">
              {state.report.disclosure_preview.privacy_note}
            </p>
          )}
          <FlowLevelDetailPreview
            report={state.report}
            omitted={state.omitSections.includes("transaction_details")}
          />
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">{t("recipient.ariaLabel")}</CardTitle>
              <CardDescription>
                {t("workstation.whoReceivesThisDossierTheir")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3 p-4">
              <RecipientPicker
                recipients={
                  state.recipientsQuery.data?.data?.recipients ?? []
                }
                selectedRecipientId={state.selectedRecipientId}
                onSelectRecipient={(recipient) => {
                  state.setSelectedRecipientId(recipient?.id ?? "");
                }}
              />
              <RecipientPreferenceAdvisory
                recipient={state.selectedRecipient}
                currentRevealMode={state.revealMode}
                onApply={(mode) => state.setRevealMode(mode)}
              />
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="border-b">
              <CardTitle className="text-base">{t("workstation.reportOptions")}</CardTitle>
              <CardDescription>
                {t("workstation.frozenIntoTheCaseAt")}</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3 p-4 text-sm">
              <Field label={t("disclosure.revealMode")} htmlFor="disclose-reveal">
                <Select
                  value={state.revealMode}
                  onValueChange={state.setRevealMode}
                >
                  <SelectTrigger id="disclose-reveal" className="h-9 w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {["labels_only", "minimal", "standard", "full"].map(
                      (mode) => (
                        <SelectItem key={mode} value={mode}>
                          {t(`reveal.${mode}`, { defaultValue: pretty(mode) })}
                        </SelectItem>
                      ),
                    )}
                  </SelectContent>
                </Select>
              </Field>
              <Field label={t("workstation.diagramDetail")} htmlFor="disclose-diagram">
                <Select
                  value={state.diagramDetail}
                  onValueChange={(value) =>
                    state.setDiagramDetail(value as "summary" | "detailed")
                  }
                >
                  <SelectTrigger id="disclose-diagram" className="h-9 w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="summary">{t("workstation.summary")}</SelectItem>
                    <SelectItem value="detailed">{t("workstation.detailed")}</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label={t("workstation.amounts")} htmlFor="disclose-precision">
                <Select
                  value={state.amountPrecision}
                  onValueChange={(value) =>
                    state.setAmountPrecision(value as "btc" | "sats")
                  }
                >
                  <SelectTrigger id="disclose-precision" className="h-9 w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="btc">BTC</SelectItem>
                    <SelectItem value="sats">sats</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <label className="flex items-center gap-2 text-sm text-muted-foreground">
                <Checkbox
                  checked={state.maskRecipient}
                  onCheckedChange={(checked) =>
                    state.setMaskRecipient(checked === true)
                  }
                />
                {t("workstation.maskRecipientNameOnThe")}</label>
              <div className="space-y-1.5 border-t pt-3">
                <div className="text-xs font-medium text-muted-foreground">
                  {t("workstation.omitVerboseSections")}</div>
                {(
                  [
                    ["flow_levels", t("case.omitFlow")],
                    ["transaction_details", t("case.omitTransactions")],
                    ["flow_links", t("case.omitLinks")],
                    ["graph_nodes", t("case.omitNodes")],
                  ] as const
                ).map(([key, label]) => (
                  <label
                    key={key}
                    className="flex items-center gap-2 text-sm text-muted-foreground"
                  >
                    <Checkbox
                      checked={state.omitSections.includes(key)}
                      onCheckedChange={(checked) =>
                        state.setOmitSections(
                          checked === true
                            ? [...state.omitSections, key]
                            : state.omitSections.filter(
                                (section) => section !== key,
                              ),
                        )
                      }
                    />
                    {label}
                  </label>
                ))}
              </div>
            </CardContent>
          </Card>

          {state.report?.diagrams?.flow_svg && (
            <Card>
              <CardHeader className="border-b">
                <CardTitle className="text-base">{t("workstation.reportVisuals")}</CardTitle>
                <CardDescription>
                  {t("workstation.renderedOnThisDeviceIdentical")}</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4 p-4">
                <ReportDiagram
                  svg={state.report.diagrams.flow_svg}
                  label={t("flowPath.title")}
                />
                <ReportDiagram
                  svg={state.report.diagrams.source_mix_ring_svg}
                  label={t("workstation.sourceMix")}
                />
                <ReportDiagram
                  svg={state.report.diagrams.data_source_ring_svg}
                  label={t("workstation.dataSources")}
                />
              </CardContent>
            </Card>
          )}
        </div>
      </div>

    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 4 — Export                                                    */
/* ------------------------------------------------------------------ */

export function ExportStage({ state }: { state: SourceFundsCaseState }) {
  const { t } = useTranslation("sourceFunds");
  const exportable = Boolean(state.report?.explain_gates.exportable) && !state.preview.isFetching;
  return (
    <div className="space-y-5">
      <StageHeader
        title={t("exportStage.title")}
        lede={t("exportStage.lede")}
      />

      {!exportable && (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2.5 text-sm text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100">
          <div className="flex items-center gap-2 font-medium">
            <ShieldAlert className="size-4" aria-hidden="true" />
            {t("case.exportBlocked")}
          </div>
          <button
            type="button"
            className="mt-1 text-xs font-medium underline-offset-2 hover:underline"
            onClick={() => document.getElementById("source-funds-findings")?.scrollIntoView({ block: "center" })}
          >
            {t("workstation.goToTheTraceWork")}</button>
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="text-base">{t("workstation.sourceoffundsPdf")}</CardTitle>
            <CardDescription>
              {t("workstation.savesTheCaseSnapshotFirst")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <Button
              className="w-full"
              disabled={
                !exportable ||
                state.casesSave.isPending ||
                state.exportPdf.isPending
              }
              onClick={() => {
                void state.handleExportPdf();
              }}
            >
              <FileDown className="mr-2 size-4" aria-hidden="true" />
              {state.casesSave.isPending
                ? t("export.savingCase")
                : state.exportPdf.isPending
                  ? t("case.rendering")
                  : t("export.saveAndExport")}
            </Button>
            {state.savedCase && (
              <dl className="space-y-1 rounded-md border px-3 py-2 text-xs">
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">{t("workstation.case")}</dt>
                  <dd className="font-mono">{state.savedCase.id}</dd>
                </div>
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">{t("workstation.status")}</dt>
                  <dd>{state.savedCase.status}</dd>
                </div>
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">{t("workstation.snapshot")}</dt>
                  <dd className="truncate font-mono">
                    {state.savedCase.snapshot_hash}
                  </dd>
                </div>
              </dl>
            )}
            {state.exportedPdf?.filename && (
              <p className="text-xs text-muted-foreground">
                {t("case.exported", { filename: state.exportedPdf.filename })}
              </p>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="text-base">{t("workstation.evidenceBundle")}</CardTitle>
            <CardDescription>
              {t("workstation.reportPdfPlusTheOriginal")}</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4 text-sm text-muted-foreground">
            <p>
              {t("workstation.saveAFrozenCaseAnd")}</p>
            <Button
              type="button"
              variant="outline"
              disabled={
                !exportable ||
                state.casesSave.isPending ||
                state.exportBundle.isPending
              }
              onClick={() => {
                void state.handleExportBundle();
              }}
            >
              <FileDown className="mr-2 size-4" aria-hidden="true" />
              {state.casesSave.isPending
                ? t("export.savingCase")
                : state.exportBundle.isPending
                  ? t("case.buildingBundle")
                  : t("case.exportBundle")}
            </Button>
            {state.exportedBundle?.filename && (
              <p className="text-xs text-muted-foreground">
                {t("case.exported", { filename: state.exportedBundle.filename })}
              </p>
            )}
            <code className="block rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs">
              kassiber reports export-source-funds-bundle --case{" "}
              {state.savedCase ? state.savedCase.id : "<case-id>"} --file
              bundle.zip
            </code>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
