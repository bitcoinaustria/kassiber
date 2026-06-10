// The four work surfaces of the case dossier. Each stage is purely
// presentational over the shared case state; the rail owns navigation.

import {
  AlertTriangle,
  ArrowRight,
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
  coverageSummary,
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

function StageFooter({
  label,
  onContinue,
}: {
  label: string;
  onContinue: () => void;
}) {
  return (
    <div className="flex justify-end border-t pt-4">
      <Button type="button" onClick={onContinue}>
        {label}
        <ArrowRight className="ml-2 size-4" aria-hidden="true" />
      </Button>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 1 — Target                                                    */
/* ------------------------------------------------------------------ */

export function TargetStage({ state }: { state: SourceFundsCaseState }) {
  const planned = state.reportPurpose === "planned_exchange_sale";
  const targetLabel = planned
    ? "Bitcoin you're about to sell"
    : "Completed transaction";
  const amountLabel = planned ? "Planned sale amount" : "Report amount";

  return (
    <div className="space-y-5">
      <StageHeader
        title="What needs explaining?"
        lede="Every dossier is anchored to one transaction. Kassiber traces history backwards from it and disclosure stays scoped to the amount you enter — nothing else."
      />

      <div className="grid gap-3 md:grid-cols-2">
        <PurposeButton
          active={planned}
          title="Planned exchange sale"
          body="Prepare a bank or exchange disclosure before the deposit or sale happens."
          onClick={() => state.setReportPurpose("planned_exchange_sale")}
        />
        <PurposeButton
          active={!planned}
          title="Already happened"
          body="Explain a completed sale, exchange deposit, withdrawal, or transfer."
          onClick={() => state.setReportPurpose("existing_transaction")}
        />
      </div>

      {planned && (
        <div className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2.5 text-sm text-muted-foreground">
          <Info className="mt-0.5 size-4 shrink-0 opacity-70" aria-hidden="true" />
          <p>
            Planned reports prove the reviewed history of the bitcoin you intend
            to sell. If those sats were originally bought on an exchange, attach
            fiat-funds proof to that purchase source as a separate evidence item.
          </p>
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
          <Field label="Exchange or broker" htmlFor="planned-destination">
            <Input
              id="planned-destination"
              value={state.plannedDestination}
              onChange={(event) =>
                state.setPlannedDestination(event.target.value)
              }
              placeholder="Kraken, Bitpanda, OTC desk..."
            />
          </Field>
          <Field label="Bank disclosure note" htmlFor="planned-note">
            <Input
              id="planned-note"
              value={state.plannedNote}
              onChange={(event) => state.setPlannedNote(event.target.value)}
              placeholder="Expected EUR proceeds, bank contact, or internal case note"
            />
          </Field>
        </div>
      )}

      <div className="rounded-md border">
        <div className="flex flex-col gap-3 border-b p-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <div className="text-sm font-medium">{targetLabel}</div>
            <div className="text-xs text-muted-foreground">
              {state.filteredTargetRows.length} of {state.rows.length}{" "}
              transactions
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
                  placeholder="Search txid, wallet, note..."
                  className="h-9 pl-9"
                />
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-9"
                onClick={() =>
                  state.setShowAdvancedTargetFilters(
                    !state.showAdvancedTargetFilters,
                  )
                }
                aria-expanded={state.showAdvancedTargetFilters}
              >
                <SlidersHorizontal className="mr-2 size-4" aria-hidden="true" />
                Filters
              </Button>
              {state.targetFiltersActive && (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-9"
                  onClick={state.clearTargetFilters}
                >
                  <X className="mr-2 size-4" aria-hidden="true" />
                  Clear
                </Button>
              )}
            </div>
            <Collapsible
              open={state.showAdvancedTargetFilters}
              onOpenChange={state.setShowAdvancedTargetFilters}
            >
              <CollapsibleContent className="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
                <Select
                  value={state.targetDirectionFilter}
                  onValueChange={state.setTargetDirectionFilter}
                >
                  <SelectTrigger className="h-9 w-full" aria-label="Filter by direction">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All flows</SelectItem>
                    <SelectItem value="incoming">Incoming</SelectItem>
                    <SelectItem value="outgoing">Outgoing</SelectItem>
                    <SelectItem value="transfer">Transfer</SelectItem>
                    <SelectItem value="swap">Swap</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetDateFilter}
                  onValueChange={state.setTargetDateFilter}
                >
                  <SelectTrigger className="h-9 w-full" aria-label="Filter by date">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All dates</SelectItem>
                    <SelectItem value="today">Today</SelectItem>
                    <SelectItem value="yesterday">Yesterday</SelectItem>
                    <SelectItem value="7days">Last 7 days</SelectItem>
                    <SelectItem value="30days">Last 30 days</SelectItem>
                    <SelectItem value="older">Older</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetStatusFilter}
                  onValueChange={state.setTargetStatusFilter}
                >
                  <SelectTrigger className="h-9 w-full" aria-label="Filter by status">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All statuses</SelectItem>
                    <SelectItem value="confirmed">Confirmed</SelectItem>
                    <SelectItem value="pending">Pending</SelectItem>
                    <SelectItem value="review">Needs review</SelectItem>
                  </SelectContent>
                </Select>
                <Select
                  value={state.targetNetworkFilter}
                  onValueChange={state.setTargetNetworkFilter}
                >
                  <SelectTrigger className="h-9 w-full" aria-label="Filter by network">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All networks</SelectItem>
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
                  <SelectTrigger className="h-9 w-full" aria-label="Filter by asset">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All assets</SelectItem>
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
                    className="h-9 w-full xl:col-span-2"
                    aria-label="Filter by wallet"
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All wallets</SelectItem>
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
        <TransactionTargetHeader />
        <div className="max-h-[430px] overflow-y-auto p-2">
          {state.filteredTargetRows.length === 0 ? (
            <EmptyState text="No transactions match these filters." />
          ) : (
            <div className="space-y-2">
              {state.filteredTargetRows.map((row) => (
                <TransactionTargetRow
                  key={txRef(row)}
                  row={row}
                  active={txRef(row) === state.selectedTarget}
                  onSelect={() => state.setTarget(txRef(row))}
                  onOpenDetails={() => {
                    state.setTarget(txRef(row));
                    state.openTxDetailById(txRef(row));
                  }}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      <TracedCoverageHero coverage={state.coverageQuery.data?.data} />
      <OptionalSection
        open={state.showCoverage}
        onOpenChange={state.setShowCoverage}
        icon={<GitBranch className="size-4" aria-hidden="true" />}
        title="Historical inbound coverage"
        summary={coverageSummary(state.coverageQuery.data?.data)}
      >
        <CoveragePanel
          coverage={state.coverageQuery.data?.data}
          loading={state.coverageQuery.isLoading}
        />
      </OptionalSection>

      <StageFooter
        label="Trace this history"
        onContinue={() => state.goToStage("trace")}
      />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 2 — Trace                                                     */
/* ------------------------------------------------------------------ */

export function TraceStage({ state }: { state: SourceFundsCaseState }) {
  const assembled = state.assembleLinks.data?.data;
  const gaps: SourceFundsFinding[] = [...state.blockers, ...state.warnings];

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
        title="Assemble the history"
        lede="Kassiber proves every hop it can from local evidence: transaction inputs/outputs of synced wallets (Bitcoin and Liquid), Lightning payment hashes, platform ids, and reviewed pairs. You document only what remains."
      >
        <Button
          type="button"
          onClick={() => void state.runAssembly()}
          disabled={!state.selectedTarget || state.assembleLinks.isPending}
        >
          <GitBranch className="mr-2 size-4" aria-hidden="true" />
          {state.assembleLinks.isPending ? "Assembling…" : "Assemble History"}
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
          Mark Gap
        </Button>
      </StageHeader>

      {assembled && (
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 rounded-md border bg-muted/30 px-3 py-2 text-sm">
          <span className="font-semibold">
            {assembled.auto_reviewed} hop
            {assembled.auto_reviewed === 1 ? "" : "s"} proven
          </span>
          <span className="text-xs text-muted-foreground">
            {Object.entries(assembled.methods)
              .map(([method, count]) => `${pretty(method)} ×${count}`)
              .join(" · ") || "no new evidence this run"}
          </span>
          {assembled.awaiting_manual_review > 0 && (
            <span className="text-xs text-muted-foreground">
              · {assembled.awaiting_manual_review} suggestion
              {assembled.awaiting_manual_review === 1 ? "" : "s"} need manual
              review
            </span>
          )}
        </div>
      )}

      <CaseBrief
        report={state.report}
        bulkReviewable={state.bulkReviewableSuggestions.length}
        manualReview={state.manualSuggestionCount}
        onOpenTransaction={state.openTxDetailById}
      />

      <section aria-label="Work ledger" className="space-y-2">
        <div className="flex items-baseline justify-between">
          <h2 className="text-sm font-semibold uppercase tracking-[0.1em] text-muted-foreground">
            Work ledger
          </h2>
          <span className="text-xs text-muted-foreground">
            {state.blockers.length} blocker
            {state.blockers.length === 1 ? "" : "s"} · {state.warnings.length}{" "}
            warning{state.warnings.length === 1 ? "" : "s"}
          </span>
        </div>
        {state.preview.isLoading && (
          <EmptyState text="Building reviewed flow..." />
        )}
        {state.preview.isError && (
          <GateRow
            finding={{
              code: "preview_unavailable",
              message:
                "No source-funds report can be built for this target yet.",
            }}
          />
        )}
        {!state.preview.isLoading && !state.preview.isError && gaps.length === 0 && state.report && (
          <div className="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-200">
            Nothing left to document — every hop on this path is reviewed.
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
        title="Advanced review editor"
        summary={`${state.reviewQueueLinks.length} links, ${state.sources.length} sources, ${state.evidence.length} evidence items`}
      >
        <AdvancedReviewEditor state={state} />
      </OptionalSection>

      <StageFooter
        label="Choose what to disclose"
        onContinue={() => state.goToStage("disclose")}
      />
    </div>
  );
}

function AdvancedReviewEditor({ state }: { state: SourceFundsCaseState }) {
  return (
    <div className="space-y-4">
      <div className="grid gap-4 2xl:grid-cols-[minmax(0,1fr)_420px]">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <Link2 className="size-4" aria-hidden="true" />
              Review Queue
            </CardTitle>
            <CardDescription>
              Matched links for the selected target, plus suggested upstream
              hops that can extend the path.
            </CardDescription>
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
                Find Links
              </Button>
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
                Review Deterministic Hops
              </Button>
            </div>
            {state.reviewQueueLinks.length === 0 ? (
              <EmptyState text="No matched links yet. Assemble History looks for transaction input/output structure, payment hashes, same-id transfers, reviewed pairs, and provider ids." />
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
                        ? "Path"
                        : link.to_transaction_id === state.selectedTxId
                          ? "Target"
                          : "Suggested"}
                    </span>
                    <span className="font-medium">{pretty(link.link_type)}</span>
                    <span className="text-muted-foreground">
                      {pretty(link.method)}
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
                      {pretty(link.confidence)}
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
              Link Review
            </CardTitle>
            <CardDescription>
              Accept, reject, allocate, and attach evidence.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            {!state.selectedLink ? (
              <EmptyState text="Select a link to review." />
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
                    label="Type"
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
                    label="Confidence"
                    value={state.linkForm.confidence}
                    options={CONFIDENCE_LEVELS}
                    onChange={(value) =>
                      state.setLinkForm((current) => ({
                        ...current,
                        confidence: value,
                      }))
                    }
                  />
                  <Field label="Allocation" htmlFor="review-allocation">
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
                  <Field label="From amount" htmlFor="review-from-allocation">
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
                <Field label="Review note" htmlFor="review-note">
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
                    Accept
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => void state.reviewSelectedLink("rejected")}
                    disabled={state.reviewLink.isPending}
                  >
                    <X className="mr-2 size-4" aria-hidden="true" />
                    Reject
                  </Button>
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
              Source Or Gap
            </CardTitle>
            <CardDescription>
              Add a reviewed root source or explicit missing-history stop.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <SelectField
                id="source-type"
                label="Source type"
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
                label="Link type"
                value={state.sourceForm.link_type}
                options={LINK_TYPES}
                onChange={(value) =>
                  state.setSourceForm((current) => ({
                    ...current,
                    link_type: value,
                  }))
                }
              />
              <Field label="Label" htmlFor="source-label">
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
              <Field label="Amount" htmlFor="source-amount">
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
              <Field label="Asset" htmlFor="source-asset">
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
                label="Applies to"
                rows={state.rows}
                value={state.sourceForm.to_transaction || state.selectedTarget}
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
            <Field label="Evidence note" htmlFor="source-description">
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
              Create Source Link
            </Button>
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2 text-base">
              <Link2 className="size-4" aria-hidden="true" />
              Manual Link
            </CardTitle>
            <CardDescription>
              Connect two known transactions with explicit allocation.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <TransactionSelect
                id="manual-from"
                label="From"
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
                value={
                  state.manualLinkForm.to_transaction || state.selectedTarget
                }
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    to_transaction: value,
                  }))
                }
              />
              <SelectField
                id="manual-type"
                label="Type"
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
                label="Confidence"
                value={state.manualLinkForm.confidence}
                options={CONFIDENCE_LEVELS}
                onChange={(value) =>
                  state.setManualLinkForm((current) => ({
                    ...current,
                    confidence: value,
                  }))
                }
              />
              <Field label="Allocation" htmlFor="manual-allocation">
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
              <Field label="From amount" htmlFor="manual-from-amount">
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
            <Field label="Review note" htmlFor="manual-note">
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
              Add Reviewed Link
            </Button>
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
  return (
    <div className="space-y-5">
      <StageHeader
        title="Through their eyes"
        lede="Everything below is exactly what the recipient receives — no more. Tune the reveal mode, hide individual transactions, and trim report sections until the disclosure matches what this recipient needs."
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
            label="Evidence"
            values={(state.report?.disclosure_preview.attachments ?? []).map(
              (item) => item.label,
            )}
          />
          <DisclosureList
            label="Excluded"
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
              <CardTitle className="text-base">Recipient</CardTitle>
              <CardDescription>
                Who receives this dossier — their default reveal mode is a
                suggestion, your choice wins.
              </CardDescription>
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
              <CardTitle className="text-base">Report options</CardTitle>
              <CardDescription>
                Frozen into the case at export time.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3 p-4 text-sm">
              <Field label="Reveal mode" htmlFor="disclose-reveal">
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
                          {pretty(mode)}
                        </SelectItem>
                      ),
                    )}
                  </SelectContent>
                </Select>
              </Field>
              <Field label="Diagram detail" htmlFor="disclose-diagram">
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
                    <SelectItem value="summary">Summary</SelectItem>
                    <SelectItem value="detailed">Detailed</SelectItem>
                  </SelectContent>
                </Select>
              </Field>
              <Field label="Amounts" htmlFor="disclose-precision">
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
                Mask recipient name on the cover
              </label>
              <div className="space-y-1.5 border-t pt-3">
                <div className="text-xs font-medium text-muted-foreground">
                  Omit verbose sections
                </div>
                {(
                  [
                    ["flow_levels", "Flow diagram data"],
                    ["transaction_details", "Transaction details"],
                    ["flow_links", "Reviewed flow links"],
                    ["graph_nodes", "Disclosure graph nodes"],
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
                <CardTitle className="text-base">Report visuals</CardTitle>
                <CardDescription>
                  Rendered on this device — identical to the exported PDF.
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4 p-4">
                <ReportDiagram
                  svg={state.report.diagrams.flow_svg}
                  label="Simplified flow path"
                />
                <ReportDiagram
                  svg={state.report.diagrams.source_mix_ring_svg}
                  label="Source mix"
                />
                <ReportDiagram
                  svg={state.report.diagrams.data_source_ring_svg}
                  label="Data sources"
                />
              </CardContent>
            </Card>
          )}
        </div>
      </div>

      <StageFooter
        label="Freeze and export"
        onContinue={() => state.goToStage("export")}
      />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Stage 4 — Export                                                    */
/* ------------------------------------------------------------------ */

export function ExportStage({ state }: { state: SourceFundsCaseState }) {
  const exportable = Boolean(state.report?.explain_gates.exportable);
  return (
    <div className="space-y-5">
      <StageHeader
        title="Freeze and hand over"
        lede="Export renders a saved immutable case snapshot — the report can be re-rendered byte-identically later, and live edits never change a handed-over dossier."
      />

      {!exportable && (
        <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2.5 text-sm text-amber-950 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-100">
          <div className="flex items-center gap-2 font-medium">
            <ShieldAlert className="size-4" aria-hidden="true" />
            Export is blocked by {state.blockers.length} open gap
            {state.blockers.length === 1 ? "" : "s"}.
          </div>
          <button
            type="button"
            className="mt-1 text-xs font-medium underline-offset-2 hover:underline"
            onClick={() => state.goToStage("trace")}
          >
            Go to the trace work ledger to resolve them →
          </button>
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="text-base">Source-of-funds PDF</CardTitle>
            <CardDescription>
              Saves the case snapshot first, then renders the PDF from it.
            </CardDescription>
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
                ? "Saving case…"
                : state.exportPdf.isPending
                  ? "Rendering…"
                  : "Save case & export PDF"}
            </Button>
            {state.savedCase && (
              <dl className="space-y-1 rounded-md border px-3 py-2 text-xs">
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">Case</dt>
                  <dd className="font-mono">{state.savedCase.id}</dd>
                </div>
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">Status</dt>
                  <dd>{state.savedCase.status}</dd>
                </div>
                <div className="flex justify-between gap-3">
                  <dt className="text-muted-foreground">Snapshot</dt>
                  <dd className="truncate font-mono">
                    {state.savedCase.snapshot_hash}
                  </dd>
                </div>
              </dl>
            )}
            {state.exportedPdf?.filename && (
              <p className="text-xs text-muted-foreground">
                Exported: {state.exportedPdf.filename}
              </p>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="text-base">Evidence bundle</CardTitle>
            <CardDescription>
              Report PDF plus the original evidence files and a SHA-256
              manifest, reveal-mode scoped.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-2 p-4 text-sm text-muted-foreground">
            <p>
              After saving a case here, build the bundle from{" "}
              <span className="font-medium text-foreground">
                Reports → Audit package
              </span>{" "}
              (scoped to this case), or via the CLI:
            </p>
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
