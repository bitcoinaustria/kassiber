// State layer of the source-of-funds workstation. One hook owns every
// query, mutation, draft field, and derived selector of the case so the
// stage components stay purely presentational.

import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { type Transaction } from "@/components/transactions";
import { toDashboardTransaction } from "@/components/transactions/dashboard/model";
import { DaemonScopeContext, useDaemon, useDaemonInfinite, useDaemonMutation } from "@/daemon/client";
import { useCurrency } from "@/lib/currency";
import { type Tx } from "@/mocks/seed";
import { exportCurrentCase } from "./caseExport";
import { useUiStore } from "@/store/ui";
import { targetQueryArgs, type SourceFundsReviewContext } from "./caseScope";

import {
  NO_ATTACHMENT,
  isBulkReviewableLink,
  pretty,
  shortId,
  transactionRows,
  txLabel,
  txWallet,
  uniqueSorted,
  type EvidenceAttachment,
  type SourceFundsCoverage,
  type SourceFundsLink,
  type SourceFundsPreview,
  type SourceFundsRecipient,
  type SourceFundsSource,
  type TransactionRow,
} from "./model";

export const CASE_STAGES = [
  { id: "target" },
  { id: "trace" },
  { id: "disclose" },
  { id: "export" },
] as const;

export type CaseStage = (typeof CASE_STAGES)[number]["id"];
export type CaseStageEntry = {
  id: CaseStage;
  label: string;
  hint: string;
};

function migrateStage(value: string | undefined): CaseStage {
  // Older persisted drafts carry the retired wizard step names.
  if (value === "setup") return "target";
  if (value === "review") return "trace";
  if (value === "trace" || value === "disclose" || value === "export") {
    return value;
  }
  if (value === "target") return "target";
  return "target";
}

export function useSourceFundsCase(profileKey: string, initialTarget = "") {
  const { t } = useTranslation("sourceFunds");
  const scope = useContext(DaemonScopeContext);
  const addNotification = useUiStore((state) => state.addNotification);
  const storedDraft = useUiStore(
    (state) => state.sourceFundsDrafts[profileKey] ?? null,
  );
  // A route opened for another transaction must not inherit the previous case amount/options.
  const persistedDraft = initialTarget && initialTarget !== storedDraft?.target ? null : storedDraft;
  const setSourceFundsDraft = useUiStore((state) => state.setSourceFundsDraft);
  const currency = useCurrency();
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const explorerSettings = useUiStore((state) => state.explorerSettings);

  const [stage, setStage] = useState<CaseStage>(
    migrateStage(persistedDraft?.currentStep),
  );
  const [reportPurpose, setReportPurpose] = useState<
    "planned_exchange_sale" | "existing_transaction"
  >(persistedDraft?.reportPurpose ?? "planned_exchange_sale");
  const [target, setTarget] = useState(initialTarget || persistedDraft?.target || "");
  const [targetAmount, setTargetAmount] = useState(
    persistedDraft?.targetAmount ?? "",
  );
  const [plannedDestination, setPlannedDestination] = useState(
    persistedDraft?.plannedDestination ?? "",
  );
  const [plannedNote, setPlannedNote] = useState(
    persistedDraft?.plannedNote ?? "",
  );
  const [revealMode, setRevealMode] = useState(
    persistedDraft?.revealMode ?? "standard",
  );
  const [diagramDetail, setDiagramDetail] = useState<"summary" | "detailed">(
    persistedDraft?.diagramDetail ?? "summary",
  );
  const [amountPrecision, setAmountPrecision] = useState<"btc" | "sats">("btc");
  const [maskRecipient, setMaskRecipient] = useState(false);
  const [omitSections, setOmitSections] = useState<string[]>([]);
  const [revealOverrides, setRevealOverrides] = useState<
    Record<string, "show" | "hide">
  >({});
  const [selectedRecipientId, setSelectedRecipientId] = useState<string>(
    persistedDraft?.selectedRecipientId ?? "",
  );
  const [detailTransaction, setDetailTransaction] =
    useState<Transaction | null>(null);
  const caseStages = useMemo<CaseStageEntry[]>(
    () =>
      CASE_STAGES.map((entry) => ({
        id: entry.id,
        label: t(`caseStages.${entry.id}.label`),
        hint: t(`caseStages.${entry.id}.hint`),
      })),
    [t],
  );

  // Target picker filters.
  const [targetSearch, setTargetSearch] = useState("");
  const [targetDirectionFilter, setTargetDirectionFilter] = useState("all");
  const [targetDateFilter, setTargetDateFilter] = useState("all");
  const [targetStatusFilter, setTargetStatusFilter] = useState("all");
  const [targetNetworkFilter, setTargetNetworkFilter] = useState("all");
  const [targetAssetFilter, setTargetAssetFilter] = useState("all");
  const [targetWalletFilter, setTargetWalletFilter] = useState("all");
  const [showAdvancedTargetFilters, setShowAdvancedTargetFilters] =
    useState(false);

  // Advanced editor working state.
  const [showAdvancedReview, setShowAdvancedReview] = useState(false);
  const [showCoverage, setShowCoverage] = useState(false);
  const [showDisclosure, setShowDisclosure] = useState(false);
  const [selectedLinkId, setSelectedLinkId] = useState("");
  const [linkFormSourceId, setLinkFormSourceId] = useState("");
  const [linkForm, setLinkForm] = useState({
    link_type: "self_transfer",
    confidence: "strong",
    allocation_amount: "",
    from_allocation_amount: "",
    explanation: "",
    attachment_id: NO_ATTACHMENT,
  });
  const [sourceForm, setSourceForm] = useState({
    source_type: "fiat_purchase",
    label: "",
    asset: "BTC",
    amount: "",
    description: "",
    attachment_id: NO_ATTACHMENT,
    to_transaction: "",
    link_type: "manual_source",
  });
  const [manualLinkForm, setManualLinkForm] = useState({
    from_transaction: "",
    to_transaction: "",
    link_type: "self_transfer",
    allocation_amount: "",
    from_allocation_amount: "",
    confidence: "strong",
    explanation: "",
    attachment_id: NO_ATTACHMENT,
  });

  const transactionArgs = useMemo(() => targetQueryArgs({
    query: targetSearch, flow: targetDirectionFilter, date: targetDateFilter,
    status: targetStatusFilter, network: targetNetworkFilter,
    asset: targetAssetFilter, wallet: targetWalletFilter,
  }), [targetSearch, targetDirectionFilter, targetDateFilter, targetStatusFilter,
    targetNetworkFilter, targetAssetFilter, targetWalletFilter]);
  const transactions = useDaemonInfinite<{ txs: TransactionRow[]; nextCursor?: string | null }>(
    "ui.transactions.list", transactionArgs, (page) => page.data?.nextCursor ?? undefined,
  );
  const rows = useMemo(() => transactions.data?.pages.flatMap((page) => transactionRows(page.data)) ?? [], [transactions.data]);
  const walletsQuery = useDaemon<{ wallets: { label: string }[] }>("ui.wallets.list");
  const targetAssetOptions = ["BTC", "LBTC"];
  const targetWalletOptions = useMemo(() => uniqueSorted(walletsQuery.data?.data?.wallets?.map((wallet) => wallet.label) ?? rows.map(txWallet)), [walletsQuery.data, rows]);
  const targetNetworkOptions = ["on-chain", "lightning", "liquid", "exchange"];
  const filteredTargetRows = rows;
  const clearTargetFilters = () => {
    setTargetSearch("");
    setTargetDirectionFilter("all");
    setTargetDateFilter("all");
    setTargetStatusFilter("all");
    setTargetNetworkFilter("all");
    setTargetAssetFilter("all");
    setTargetWalletFilter("all");
  };
  const targetFiltersActive =
    Boolean(targetSearch) ||
    targetDirectionFilter !== "all" ||
    targetDateFilter !== "all" ||
    targetStatusFilter !== "all" ||
    targetNetworkFilter !== "all" ||
    targetAssetFilter !== "all" ||
    targetWalletFilter !== "all";

  const selectedTarget = target.trim();
  const previewArgs = {
    target_transaction: selectedTarget,
    target_amount: targetAmount || undefined,
    report_purpose: reportPurpose,
    planned_destination:
      reportPurpose === "planned_exchange_sale"
        ? plannedDestination || undefined
        : undefined,
    planned_note:
      reportPurpose === "planned_exchange_sale"
        ? plannedNote || undefined
        : undefined,
    reveal_mode: revealMode,
    recipient: selectedRecipientId || undefined,
    report_options: {
      diagram_detail: diagramDetail,
      amount_precision: amountPrecision,
      mask_recipient: maskRecipient,
      omit_sections: omitSections,
      reveal_overrides: revealOverrides,
    },
  };
  const preview = useDaemon<SourceFundsReviewContext>(
    "ui.source_funds.review_context",
    previewArgs,
    { enabled: Boolean(selectedTarget), retry: false },
  );
  const resolvedTarget = useDaemon<{ transaction?: TransactionRow }>(
    "ui.transactions.resolve", { query: preview.data?.data?.target.transaction_id }, { enabled: Boolean(preview.data?.data?.target.transaction_id), retry: false },
  );
  const selectedTx = resolvedTarget.data?.data?.transaction;
  const selectedTxId = preview.data?.data?.target.transaction_id ?? "";
  const selectedTargetAmount = targetAmount;
  const txById = useMemo(() => {
    const mapping = new Map<string, TransactionRow>();
    [...rows, ...(selectedTx ? [selectedTx] : [])].forEach((row) => {
      if (row.id) mapping.set(row.id, row);
      if (row.transaction_id) mapping.set(row.transaction_id, row);
    });
    return mapping;
  }, [rows, selectedTx]);
  const resolveDetail = useDaemonMutation<{ transaction?: TransactionRow }>("ui.transactions.resolve", { invalidateQueries: false });
  const openTxDetailById = async (txId: string) => {
    if (!txId) return;
    try {
      const envelope = await resolveDetail.mutateAsync({ query: txId });
      if (scope?.isCurrent?.() === false) return;
      if (envelope.data?.transaction) setDetailTransaction(toDashboardTransaction(envelope.data.transaction as unknown as Tx, 0));
    } catch {
      if (scope?.isCurrent?.() !== false) addNotification({ title: t("header.title"), body: t("case.targetUnavailable"), tone: "info" });
    }
  };

  const sourcesQuery = useDaemon<{ sources: SourceFundsSource[] }>(
    "ui.source_funds.sources.list", undefined, { enabled: showAdvancedReview },
  );
  const evidenceQuery = useDaemon<{ attachments: EvidenceAttachment[] }>(
    "ui.source_funds.evidence.list", undefined, { enabled: showAdvancedReview },
  );
  const coverageQuery = useDaemon<SourceFundsCoverage>(
    "ui.source_funds.coverage", undefined, { enabled: showCoverage },
  );
  const recipientsQuery = useDaemon<{ recipients: SourceFundsRecipient[] }>(
    "ui.source_funds.recipients.list",
    { include_inactive: true },
  );
  const selectedRecipient = useMemo<SourceFundsRecipient | null>(() => {
    const all = recipientsQuery.data?.data?.recipients ?? [];
    return all.find((item) => item.id === selectedRecipientId) ?? null;
  }, [recipientsQuery.data, selectedRecipientId]);

  const suggestLinks = useDaemonMutation<{ inserted: number }>(
    "ui.source_funds.suggest",
  );
  const assembleLinks = useDaemonMutation<{
    passes: number;
    inserted: number;
    auto_reviewed: number;
    awaiting_manual_review: number;
    methods: Record<string, number>;
  }>("ui.source_funds.assemble");
  const bulkReviewLinks = useDaemonMutation<{
    reviewed: number;
    skipped: number;
  }>("ui.source_funds.links.bulk_review");
  const reviewLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.review",
  );
  const attachLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.attach",
  );
  const createLink = useDaemonMutation<SourceFundsLink>(
    "ui.source_funds.links.create",
  );
  const createSource = useDaemonMutation<SourceFundsSource>(
    "ui.source_funds.sources.create",
  );
  const casesSave = useDaemonMutation<SourceFundsPreview>(
    "ui.source_funds.cases.save",
  );
  const exportPdf = useDaemonMutation("ui.source_funds.export_pdf");
  const exportBundle = useDaemonMutation("ui.source_funds.export_bundle");

  // Expensive printable SVGs are only requested while disclosure is expanded.
  const diagramQuery = useDaemon<SourceFundsPreview>("ui.source_funds.preview", previewArgs,
    { enabled: showDisclosure && Boolean(selectedTxId) });
  const canonicalReport = preview.data?.data?.report;
  const report = canonicalReport ? { ...canonicalReport,
    diagrams: showDisclosure && !diagramQuery.isFetching ? diagramQuery.data?.data?.diagrams : undefined,
  } : undefined;
  const recipeKey = JSON.stringify([previewArgs, preview.data?.data?.review_fingerprint]);
  const currentRecipe = useRef(recipeKey);
  currentRecipe.current = recipeKey;
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const [completedExport, setCompletedExport] = useState<{
    recipeKey: string; savedCase: NonNullable<SourceFundsPreview["case"]>;
    pdf?: { filename?: string }; bundle?: { filename?: string };
  } | null>(null);
  const currentExport = completedExport?.recipeKey === recipeKey ? completedExport : null;
  const savedCase = currentExport?.savedCase ?? null;
  const exportedPdf = currentExport?.pdf;
  const exportedBundle = currentExport?.bundle;
  const [exportFailure, setExportFailure] = useState<string | null>(null);
  const handleExport = async (kind: "pdf" | "bundle") => {
    if (!report?.explain_gates.exportable || preview.isFetching) return;
    if (casesSave.isPending || exportPdf.isPending || exportBundle.isPending) return;
    const capturedRecipe = recipeKey;
    setExportFailure(null);
    try {
      const result = await exportCurrentCase({
        save: async () => (await casesSave.mutateAsync({ ...previewArgs, expected_review_fingerprint: preview.data?.data?.review_fingerprint })).data,
        render: async (args) => (await (kind === "pdf" ? exportPdf : exportBundle).mutateAsync(args)).data as { filename?: string } | undefined,
        isCurrent: () => mounted.current && currentRecipe.current === capturedRecipe && scope?.isCurrent?.() !== false && (!scope || useUiStore.getState().daemonSession === scope.daemonSession),
      });
      if (result) setCompletedExport({ recipeKey: capturedRecipe, savedCase: result.savedCase, [kind]: result.output });
    } catch {
      if (mounted.current && currentRecipe.current === capturedRecipe) setExportFailure(capturedRecipe);
    }
  };
  const handleExportPdf = () => handleExport("pdf");
  const handleExportBundle = () => handleExport("bundle");
  const links = preview.data?.data?.links ?? [];
  const sources = showAdvancedReview ? sourcesQuery.data?.data?.sources ?? preview.data?.data?.sources ?? [] : preview.data?.data?.sources ?? [];
  const evidence = showAdvancedReview ? evidenceQuery.data?.data?.attachments ?? preview.data?.data?.evidence ?? [] : preview.data?.data?.evidence ?? [];
  const blockers = report?.explain_gates.blockers ?? [];
  const warnings = report?.explain_gates.warnings ?? [];

  // The canonical investigation owns graph reachability and evidence scope.
  const reachableLinkIds = new Set(links.map((link) => link.id));
  const reviewQueueLinks = links;
  const selectedLink =
    reviewQueueLinks.find((link) => link.id === selectedLinkId) ??
    reviewQueueLinks.find((link) => link.state === "suggested") ??
    reviewQueueLinks[0];
  const selectedSource = sources.find(
    (source) => source.id === selectedLink?.from_source_id,
  );
  const bulkReviewableSuggestions = links.filter(
    (link) => reachableLinkIds.has(link.id) && isBulkReviewableLink(link),
  );
  const manualSuggestionCount = links.filter(
    (link) =>
      reachableLinkIds.has(link.id) &&
      link.state === "suggested" &&
      !isBulkReviewableLink(link),
  ).length;

  // Persist only under the canonical database/workspace/profile key supplied by the scope wrapper.
  useEffect(() => {
    setSourceFundsDraft(profileKey, {
      target,
      targetAmount,
      reportPurpose,
      plannedDestination,
      plannedNote,
      revealMode,
      diagramDetail,
      selectedRecipientId,
      currentStep: stage,
    });
  }, [
    profileKey,
    setSourceFundsDraft,
    target,
    targetAmount,
    reportPurpose,
    plannedDestination,
    plannedNote,
    revealMode,
    diagramDetail,
    selectedRecipientId,
    stage,
  ]);

  useEffect(() => {
    if (!selectedTarget) return;
    // The assembled-hops summary belongs to one target; drop it on switch.
    assembleLinks.reset();
    setSourceForm((current) =>
      current.to_transaction === selectedTarget
        ? current
        : { ...current, to_transaction: selectedTarget },
    );
    setManualLinkForm((current) =>
      current.to_transaction === selectedTarget
        ? current
        : { ...current, to_transaction: selectedTarget },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps -- mutation object identity changes per render; reset only on target switch.
  }, [selectedTarget]);

  useEffect(() => {
    if (!selectedLink) {
      if (linkFormSourceId) {
        setLinkFormSourceId("");
      }
      return;
    }
    if (selectedLink.id === linkFormSourceId) {
      return;
    }
    setSelectedLinkId(selectedLink.id);
    setLinkFormSourceId(selectedLink.id);
    setLinkForm({
      link_type: selectedLink.link_type,
      confidence: selectedLink.confidence,
      allocation_amount:
        typeof selectedLink.allocation_amount === "number"
          ? selectedLink.allocation_amount.toFixed(8)
          : "",
      from_allocation_amount:
        typeof selectedLink.from_allocation_amount === "number"
          ? selectedLink.from_allocation_amount.toFixed(8)
          : "",
      explanation: selectedLink.explanation ?? "",
      attachment_id: NO_ATTACHMENT,
    });
  }, [selectedLink, linkFormSourceId]);

  const txName = (id?: string | null) => {
    const row = id ? txById.get(id) : undefined;
    return row ? txLabel(row) : shortId(id);
  };
  const sourceName = (id?: string | null) =>
    sources.find((source) => source.id === id)?.label ?? shortId(id);

  async function runSuggestions(showNotification = true) {
    if (!selectedTarget) return;
    const envelope = await suggestLinks.mutateAsync({
      target_transaction: selectedTarget,
    });
    const inserted = envelope.data?.inserted ?? 0;
    if (showNotification || inserted > 0) {
      addNotification({
        title: t(showNotification ? "toast.suggestionsUpdated" : "toast.evidenceMatched"),
        body: t("toast.linksFound", { count: inserted }),
        tone: inserted > 0 ? "success" : "info",
      });
    }
  }

  async function runAssembly(showNotification = true) {
    if (!selectedTarget) return;
    const envelope = await assembleLinks.mutateAsync({
      target_transaction: selectedTarget,
    });
    const summary = envelope.data;
    const reviewed = summary?.auto_reviewed ?? 0;
    const manual = summary?.awaiting_manual_review ?? 0;
    if (showNotification || reviewed > 0) {
      addNotification({
        title: t(reviewed > 0 ? "case.historyAssembled" : "actionsBar.assembleResultEmpty"),
        body: t("toast.deterministicBody", { reviewed, skipped: manual }),
        tone: reviewed > 0 ? "success" : "info",
      });
    }
  }

  const bulkReviewDeterministicLinks = async () => {
    if (!selectedTarget) return;
    const envelope = await bulkReviewLinks.mutateAsync({
      target_transaction: selectedTarget,
    });
    const reviewed = envelope.data?.reviewed ?? 0;
    const skipped = envelope.data?.skipped ?? 0;
    addNotification({
      title: t("toast.deterministicReviewed"),
      body: t("toast.deterministicBody", { reviewed, skipped }),
      tone: reviewed > 0 ? "success" : "info",
    });
  };

  const reviewSelectedLink = async (state: "reviewed" | "rejected") => {
    if (!selectedLink) return;
    await reviewLink.mutateAsync({
      link: selectedLink.id,
      state,
      link_type: linkForm.link_type,
      confidence: linkForm.confidence,
      allocation_amount: linkForm.allocation_amount || undefined,
      from_allocation_amount: linkForm.from_allocation_amount || undefined,
      allocation_policy: state === "reviewed" ? "explicit" : undefined,
      explanation: linkForm.explanation,
    });
    if (state === "reviewed" && linkForm.attachment_id !== NO_ATTACHMENT) {
      await attachLink.mutateAsync({
        link: selectedLink.id,
        attachment_id: linkForm.attachment_id,
      });
    }
    addNotification({
      title: t(state === "reviewed" ? "toast.linkAccepted" : "toast.linkRejected"),
      body: t(state === "reviewed" ? "toast.linkReviewedBody" : "toast.linkRejectedBody", { type: t(`linkType.${linkForm.link_type}`, { defaultValue: pretty(linkForm.link_type) }) }),
      tone: state === "reviewed" ? "success" : "info",
    });
  };

  const createManualLink = async () => {
    await createLink.mutateAsync({
      from_transaction: manualLinkForm.from_transaction,
      to_transaction: manualLinkForm.to_transaction || selectedTarget,
      link_type: manualLinkForm.link_type,
      state: "reviewed",
      confidence: manualLinkForm.confidence,
      method: "manual",
      allocation_amount: manualLinkForm.allocation_amount,
      from_allocation_amount:
        manualLinkForm.from_allocation_amount || undefined,
      allocation_policy: "explicit",
      explanation: manualLinkForm.explanation,
      attachment_id:
        manualLinkForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : manualLinkForm.attachment_id,
    });
    setManualLinkForm((current) => ({
      ...current,
      allocation_amount: "",
      from_allocation_amount: "",
      explanation: "",
      attachment_id: NO_ATTACHMENT,
    }));
    addNotification({
      title: t("toast.manualLinkAdded"),
      body: t("toast.manualLinkBody"),
      tone: "success",
    });
  };

  const createSourceLink = async () => {
    const sourceEnvelope = await createSource.mutateAsync({
      source_type: sourceForm.source_type,
      label: sourceForm.label,
      asset: sourceForm.asset,
      amount: sourceForm.amount,
      description: sourceForm.description,
      attachment_id:
        sourceForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : sourceForm.attachment_id,
    });
    if (!sourceEnvelope.data?.id) return;
    await createLink.mutateAsync({
      from_source: sourceEnvelope.data.id,
      to_transaction: sourceForm.to_transaction || selectedTarget,
      link_type: sourceForm.link_type,
      state: "reviewed",
      confidence:
        sourceForm.source_type === "missing_history" ? "unknown" : "strong",
      method: "manual",
      allocation_amount: sourceForm.amount,
      allocation_policy: "explicit",
      explanation: sourceForm.description,
      attachment_id:
        sourceForm.attachment_id === NO_ATTACHMENT
          ? undefined
          : sourceForm.attachment_id,
    });
    setSourceForm((current) => ({
      ...current,
      label: "",
      amount: "",
      description: "",
      attachment_id: NO_ATTACHMENT,
    }));
    addNotification({
      title:
        sourceForm.source_type === "missing_history"
          ? t("toast.gapMarked")
          : t("toast.sourceLinked"),
      body: t("toast.sourcePathBody"),
      tone: "success",
    });
  };

  /** Prefill the gap form for a quantified missing-history finding. */
  const prefillGapForm = (gap?: {
    amount?: number | null;
    asset?: string;
    ref?: string;
  }) => {
    setSourceForm((current) => ({
      ...current,
      source_type: "missing_history",
      link_type: "missing_history",
      label: current.label || t("gapDefaults.label"),
      asset: gap?.asset || current.asset,
      amount:
        typeof gap?.amount === "number"
          ? gap.amount.toFixed(8)
          : current.amount || selectedTargetAmount,
      to_transaction:
        gap?.ref && txById.has(gap.ref) ? gap.ref : current.to_transaction,
      description:
        current.description ||
        t("gapDefaults.description"),
    }));
  };

  const goToStage = (next: CaseStage) => { setStage(next); };

  return {
    // identity / app context
    addNotification,
    currency,
    hideSensitive,
    explorerSettings,
    // stage
    caseStages,
    stage,
    setStage,
    goToStage,
    // dossier fields
    reportPurpose,
    setReportPurpose,
    target,
    setTarget,
    targetAmount,
    setTargetAmount,
    plannedDestination,
    setPlannedDestination,
    plannedNote,
    setPlannedNote,
    revealMode,
    setRevealMode,
    diagramDetail,
    setDiagramDetail,
    amountPrecision,
    setAmountPrecision,
    maskRecipient,
    setMaskRecipient,
    omitSections,
    setOmitSections,
    revealOverrides,
    setRevealOverrides,
    selectedRecipientId,
    setSelectedRecipientId,
    selectedRecipient,
    detailTransaction,
    setDetailTransaction,
    // target picker
    rows,
    transactions,
    resolvedTarget,
    previewArgs,
    filteredTargetRows,
    targetSearch,
    setTargetSearch,
    targetDirectionFilter,
    setTargetDirectionFilter,
    targetDateFilter,
    setTargetDateFilter,
    targetStatusFilter,
    setTargetStatusFilter,
    targetNetworkFilter,
    setTargetNetworkFilter,
    targetAssetFilter,
    setTargetAssetFilter,
    targetWalletFilter,
    setTargetWalletFilter,
    targetAssetOptions,
    targetWalletOptions,
    targetNetworkOptions,
    targetFiltersActive,
    clearTargetFilters,
    showAdvancedTargetFilters,
    setShowAdvancedTargetFilters,
    // selection
    selectedTarget,
    selectedTx,
    selectedTxId,
    selectedTargetAmount,
    txById,
    txName,
    sourceName,
    openTxDetailById,
    // queries
    preview,
    report,
    coverageQuery,
    recipientsQuery,
    // collections
    links,
    sources,
    evidence,
    blockers,
    warnings,
    reachableLinkIds,
    reviewQueueLinks,
    selectedLink,
    selectedLinkId,
    setSelectedLinkId,
    selectedSource,
    bulkReviewableSuggestions,
    manualSuggestionCount,
    // mutations + actions
    suggestLinks,
    assembleLinks,
    bulkReviewLinks,
    reviewLink,
    attachLink,
    createLink,
    createSource,
    casesSave,
    exportPdf,
    exportBundle,
    savedCase,
    exportedPdf,
    exportedBundle,
    exportError: exportFailure === recipeKey,
    showDisclosure,
    setShowDisclosure,
    handleExportPdf,
    handleExportBundle,
    runSuggestions,
    runAssembly,
    bulkReviewDeterministicLinks,
    reviewSelectedLink,
    createManualLink,
    createSourceLink,
    prefillGapForm,
    // advanced editor
    showAdvancedReview,
    setShowAdvancedReview,
    showCoverage,
    setShowCoverage,
    linkForm,
    setLinkForm,
    sourceForm,
    setSourceForm,
    manualLinkForm,
    setManualLinkForm,
  };
}

export type SourceFundsCaseState = ReturnType<typeof useSourceFundsCase>;
