// State layer of the source-of-funds workstation. One hook owns every
// query, mutation, draft field, and derived selector of the case so the
// stage components stay purely presentational.

import { useEffect, useMemo, useState } from "react";

import { type Transaction } from "@/components/transactions";
import { toDashboardTransaction } from "@/components/transactions/dashboard/model";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useCurrency } from "@/lib/currency";
import { type Tx } from "@/mocks/seed";
import { sourceFundsExportArgs } from "@/lib/sourceFundsExport";
import { useUiStore } from "@/store/ui";

import {
  DATE_FILTER_BUCKETS,
  NO_ATTACHMENT,
  isBulkReviewableLink,
  pretty,
  shortId,
  transactionRows,
  txDateFilterValue,
  txFlow,
  txLabel,
  txNetwork,
  txRef,
  txSearchText,
  txStatus,
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
  { id: "target", label: "Target", hint: "What needs explaining" },
  { id: "trace", label: "Trace", hint: "Assemble the history" },
  { id: "disclose", label: "Disclose", hint: "Through their eyes" },
  { id: "export", label: "Export", hint: "Freeze and hand over" },
] as const;

export type CaseStage = (typeof CASE_STAGES)[number]["id"];

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

export function useSourceFundsCase() {
  const addNotification = useUiStore((state) => state.addNotification);
  const profileKey = useUiStore(
    (state) => state.identity?.profile ?? "default",
  );
  const persistedDraft = useUiStore(
    (state) => state.sourceFundsDrafts[profileKey] ?? null,
  );
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
  const [target, setTarget] = useState(persistedDraft?.target ?? "");
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

  const transactions = useDaemon<unknown>("ui.transactions.list", {
    limit: 500,
  });
  const rows = useMemo(
    () => transactionRows(transactions.data?.data),
    [transactions.data],
  );
  const targetAssetOptions = useMemo(
    () => uniqueSorted(rows.map((row) => row.asset || "BTC")),
    [rows],
  );
  const targetWalletOptions = useMemo(
    () => uniqueSorted(rows.map(txWallet)),
    [rows],
  );
  const targetNetworkOptions = useMemo(
    () => uniqueSorted(rows.map(txNetwork)),
    [rows],
  );
  const filteredTargetRows = useMemo(() => {
    const query = targetSearch.trim().toLowerCase();
    return rows.filter((row) => {
      const matchesSearch = !query || txSearchText(row).includes(query);
      const matchesDirection =
        targetDirectionFilter === "all" || txFlow(row) === targetDirectionFilter;
      const matchesDate =
        targetDateFilter === "all" ||
        (DATE_FILTER_BUCKETS[targetDateFilter]?.has(txDateFilterValue(row)) ??
          true);
      const matchesStatus =
        targetStatusFilter === "all" || txStatus(row) === targetStatusFilter;
      const matchesNetwork =
        targetNetworkFilter === "all" || txNetwork(row) === targetNetworkFilter;
      const matchesAsset =
        targetAssetFilter === "all" || (row.asset || "BTC") === targetAssetFilter;
      const matchesWallet =
        targetWalletFilter === "all" || txWallet(row) === targetWalletFilter;
      return (
        matchesSearch &&
        matchesDirection &&
        matchesDate &&
        matchesStatus &&
        matchesNetwork &&
        matchesAsset &&
        matchesWallet
      );
    });
  }, [
    rows,
    targetSearch,
    targetDirectionFilter,
    targetDateFilter,
    targetStatusFilter,
    targetNetworkFilter,
    targetAssetFilter,
    targetWalletFilter,
  ]);
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

  const selectedTarget = target || txRef(rows[0] ?? {});
  const selectedTx =
    rows.find((row) => txRef(row) === selectedTarget) ?? rows[0];
  const selectedTxId = selectedTx?.id || selectedTx?.transaction_id || "";
  const selectedTargetAmount =
    targetAmount ||
    (typeof selectedTx?.amount === "number"
      ? selectedTx.amount.toFixed(8)
      : "");
  const txById = useMemo(() => {
    const mapping = new Map<string, TransactionRow>();
    rows.forEach((row) => {
      if (row.id) mapping.set(row.id, row);
      if (row.transaction_id) mapping.set(row.transaction_id, row);
    });
    return mapping;
  }, [rows]);
  const openTxDetailById = (txId: string) => {
    if (!txId) return;
    const index = rows.findIndex(
      (row) =>
        row.id === txId || row.transaction_id === txId || txRef(row) === txId,
    );
    if (index >= 0) {
      setDetailTransaction(
        toDashboardTransaction(rows[index] as unknown as Tx, index),
      );
    }
  };

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
  const preview = useDaemon<SourceFundsPreview>(
    "ui.source_funds.preview",
    previewArgs,
    { enabled: Boolean(selectedTarget) },
  );
  const sourcesQuery = useDaemon<{ sources: SourceFundsSource[] }>(
    "ui.source_funds.sources.list",
  );
  const linksQuery = useDaemon<{ links: SourceFundsLink[] }>(
    "ui.source_funds.links.list",
  );
  const evidenceQuery = useDaemon<{ attachments: EvidenceAttachment[] }>(
    "ui.source_funds.evidence.list",
  );
  const coverageQuery = useDaemon<SourceFundsCoverage>(
    "ui.source_funds.coverage",
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

  const report = preview.data?.data;
  const savedCase = casesSave.data?.data?.case ?? null;
  const exportedPdf = exportPdf.data?.data as { filename?: string } | undefined;
  const exportedBundle = exportBundle.data?.data as
    | { filename?: string }
    | undefined;

  const handleExportPdf = async () => {
    if (!report?.explain_gates.exportable) return;
    if (casesSave.isPending || exportPdf.isPending) return;
    const saved = await casesSave.mutateAsync(previewArgs);
    const args = sourceFundsExportArgs(saved.data);
    if (!args) return;
    exportPdf.mutate(args);
  };

  const handleExportBundle = async () => {
    if (!report?.explain_gates.exportable) return;
    if (casesSave.isPending || exportBundle.isPending) return;
    const saved = await casesSave.mutateAsync(previewArgs);
    const args = sourceFundsExportArgs(saved.data);
    if (!args) return;
    exportBundle.mutate(args);
  };

  const links = useMemo(
    () => linksQuery.data?.data?.links ?? [],
    [linksQuery.data],
  );
  const sources = useMemo(
    () => sourcesQuery.data?.data?.sources ?? [],
    [sourcesQuery.data],
  );
  const evidence = useMemo(
    () => evidenceQuery.data?.data?.attachments ?? [],
    [evidenceQuery.data],
  );
  const blockers = report?.explain_gates.blockers ?? [];
  const warnings = report?.explain_gates.warnings ?? [];

  const reachableLinkIds = useMemo(() => {
    const found = new Set<string>();
    if (!selectedTxId) return found;
    const byTo = new Map<string, SourceFundsLink[]>();
    links.forEach((link) => {
      const rowsForTarget = byTo.get(link.to_transaction_id) ?? [];
      rowsForTarget.push(link);
      byTo.set(link.to_transaction_id, rowsForTarget);
    });
    const queue = [selectedTxId];
    const visited = new Set<string>();
    while (queue.length > 0) {
      const txId = queue.shift();
      if (!txId || visited.has(txId)) continue;
      visited.add(txId);
      for (const link of byTo.get(txId) ?? []) {
        if (link.state === "rejected") continue;
        found.add(link.id);
        if (link.from_transaction_id) queue.push(link.from_transaction_id);
      }
    }
    return found;
  }, [links, selectedTxId]);
  const reviewQueueLinks = useMemo(() => {
    const rowsForReview = links.filter(
      (link) =>
        reachableLinkIds.has(link.id) ||
        link.to_transaction_id === selectedTxId ||
        link.state === "suggested",
    );
    const queueRows = rowsForReview.length > 0 ? rowsForReview : links;
    return [...queueRows].sort((a, b) => {
      const score = (link: SourceFundsLink) => {
        if (reachableLinkIds.has(link.id)) return 0;
        if (link.to_transaction_id === selectedTxId) return 1;
        if (link.state === "suggested") return 2;
        if (link.state === "reviewed") return 3;
        return 4;
      };
      const scoreDelta = score(a) - score(b);
      if (scoreDelta !== 0) return scoreDelta;
      return Number(a.state === "rejected") - Number(b.state === "rejected");
    });
  }, [links, reachableLinkIds, selectedTxId]);
  const selectedLink =
    reviewQueueLinks.find((link) => link.id === selectedLinkId) ??
    reviewQueueLinks.find((link) => link.state === "suggested") ??
    reviewQueueLinks[0] ??
    links[0];
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

  // Persist the dossier draft per profile.
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
        title: showNotification ? "Suggestions updated" : "Evidence matched",
        body: `${inserted} new source-funds link${inserted === 1 ? "" : "s"}.`,
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
        title: reviewed > 0 ? "History assembled" : "Nothing new to assemble",
        body:
          `${reviewed} hop${reviewed === 1 ? "" : "s"} proven from local evidence` +
          (manual > 0
            ? `; ${manual} suggestion${manual === 1 ? "" : "s"} left for manual review.`
            : "."),
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
      title: "Deterministic hops reviewed",
      body: `${reviewed} reviewed, ${skipped} left for manual review.`,
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
      title: state === "reviewed" ? "Link accepted" : "Link rejected",
      body: `${pretty(linkForm.link_type)} ${state}.`,
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
      title: "Manual link added",
      body: "The reviewed flow has been updated.",
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
          ? "Gap marked reviewed"
          : "Source linked",
      body: "The source-funds path has been updated.",
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
      label: current.label || "Reviewed missing history",
      asset: gap?.asset || current.asset,
      amount:
        typeof gap?.amount === "number"
          ? gap.amount.toFixed(8)
          : current.amount || selectedTargetAmount,
      to_transaction:
        gap?.ref && txById.has(gap.ref) ? gap.ref : current.to_transaction,
      description:
        current.description ||
        "Prior history is missing and has been reviewed as a disclosure gap.",
    }));
  };

  const goToStage = (next: CaseStage) => {
    const wasTarget = stage === "target";
    setStage(next);
    if (wasTarget && next === "trace" && selectedTarget) {
      void runAssembly(false);
    }
  };

  return {
    // identity / app context
    addNotification,
    currency,
    hideSensitive,
    explorerSettings,
    // stage
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
