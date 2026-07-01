import {
  AlertTriangle,
  ArrowDown,
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUp,
  ArrowUpRight,
  ArrowUpDown,
  Coins,
  Copy,
  Eye,
  Filter,
  Link2Off,
  MoreHorizontal,
  Pencil,
  X,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import {
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  openAttachmentFile,
  openExternalUrl,
  type DaemonEnvelope,
} from "@/daemon/transport";
import { cn } from "@/lib/utils";
import { accountLegs, accountMatchesLabel } from "@/lib/connectionTransactions";
import { type Currency } from "@/lib/currency";
import { type ExplorerSettings } from "@/lib/explorer";
import { useUiStore } from "@/store/ui";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
} from "@/lib/transactionHistory";
import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  TransactionEvidenceReuseDialog,
  allPaymentMethods,
  allTransactionFlows,
  allTransactionStatuses,
  austrianTaxClassificationFor,
  blurClass,
  classificationOptionLabelKeys,
  copyText,
  currencyFormatter,
  draftForTransaction,
  explorerForTransaction,
  formatCounterDisplayMoney,
  formatDisplayMoney,
  formatShortTxid,
  formatSignedDisplayMoney,
  parseManualDecimal,
  pricingCacheSummary,
  pricingSelectionValue,
  pricingSourceLabel,
  pricingSourceStyles,
  transactionBtc,
  transactionFlow,
  transactionFlowLabels,
  transactionFlowStyles,
  transactionStatusIcons,
  transactionStatusLabels,
  transactionStatusStyles,
  type CommercialContextData,
  type LoanMark,
  type LoanMarkTarget,
  type Transaction,
  type TransactionEditDraft,
  type TransactionFlow,
  type TransactionStatus,
} from "@/components/transactions";
import {
  attachmentRecordToItem,
  breakdownSelectionLabel,
  filterChipClassName,
  flowChartSelectionLabel,
  isAttachmentListQueryKeyForTransaction,
  isRedundantTransactionLabel,
  matchesFlowChartSelection,
  matchesTransactionDeepLink,
  pairRailLabel,
  quickFilterLabel,
  readTransactionDetailParams,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  updateTransactionDetailParams,
  upsertAttachmentRecords,
  type AttachmentOpenData,
  type AttachmentRecord,
  type AttachmentsCopyData,
  type AttachmentsListData,
  type BreakdownSelection,
  type FeeFilter,
  type FlowChartSelection,
  type JournalEventsData,
  type TableQuickFilter,
} from "./model";

type LoansList = {
  marks: LoanMark[];
};

function loanRoleBadgeLabelKey(role: string): string {
  if (role === "collateral_release") {
    return "table.row.collateral.returnedBadge";
  }
  if (role === "loan_principal_received") {
    return "table.row.collateral.principalReceivedBadge";
  }
  if (role === "loan_principal_repaid") {
    return "table.row.collateral.principalRepaidBadge";
  }
  return "table.row.collateral.collateralBadge";
}

function loanRoleAccountingLabelKey(role: string): string {
  if (role === "collateral_release") {
    return "table.row.collateral.returnedAccounting";
  }
  if (role === "loan_principal_received") {
    return "table.row.collateral.principalReceivedAccounting";
  }
  if (role === "loan_principal_repaid") {
    return "table.row.collateral.principalRepaidAccounting";
  }
  return "table.row.collateral.collateralAccounting";
}

function loanRoleBadgeClassName(role: string): string {
  if (role === "loan_principal_received" || role === "loan_principal_repaid") {
    return "border-sky-500/40 text-sky-700 dark:text-sky-400";
  }
  if (role === "collateral_release") {
    return "border-emerald-500/40 text-emerald-700 dark:text-emerald-400";
  }
  return "border-amber-500/40 text-amber-700 dark:text-amber-400";
}

type TableSortKey = "date" | "amount";
type TableSortDirection = "asc" | "desc";
type TableSortState = {
  key: TableSortKey;
  direction: TableSortDirection;
} | null;

function sortableTransactionDateValue(label: string) {
  const normalized = label.trim().toLowerCase();
  if (normalized === "today") return Date.now();
  if (normalized === "yesterday" || normalized === "1 day ago") {
    return Date.now() - 24 * 60 * 60 * 1000;
  }
  const relativeDays = normalized.match(/^(\d+)\s+days?\s+ago$/);
  if (relativeDays) {
    return Date.now() - Number(relativeDays[1]) * 24 * 60 * 60 * 1000;
  }
  const parsed = Date.parse(label);
  return Number.isNaN(parsed) ? null : parsed;
}

const TransactionsTable = ({
  records,
  hideSensitive,
  currency,
  nowRate,
  explorerSettings,
  swapCandidateIds = new Set<string>(),
  chartSelection,
  quickFilter,
  breakdownSelection,
  onChartSelectionChange,
  onQuickFilterChange,
  onBreakdownSelectionChange,
  resetTableFiltersToken,
  isRefreshing,
  hasMoreRecords = false,
  isLoadingMoreRecords = false,
  onLoadMoreRecords,
  deepLinkedTransactionId,
  deepLinkedTransactionTab = "details",
}: {
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  nowRate: number | null;
  explorerSettings: ExplorerSettings;
  swapCandidateIds?: Set<string>;
  chartSelection: FlowChartSelection | null;
  quickFilter: TableQuickFilter | null;
  breakdownSelection: BreakdownSelection | null;
  onChartSelectionChange: (selection: FlowChartSelection | null) => void;
  onQuickFilterChange: (filter: TableQuickFilter | null) => void;
  onBreakdownSelectionChange: (selection: BreakdownSelection | null) => void;
  resetTableFiltersToken: number;
  isRefreshing?: boolean;
  hasMoreRecords?: boolean;
  isLoadingMoreRecords?: boolean;
  onLoadMoreRecords?: () => void;
  deepLinkedTransactionId?: string | null;
  deepLinkedTransactionTab?: string;
}) => {
  const { t } = useTranslation("transactions");
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [statusFilter, setStatusFilter] = React.useState<string>("all");
  const [flowFilter, setFlowFilter] = React.useState<string>("all");
  const [paymentMethodFilter, setPaymentMethodFilter] =
    React.useState<string>("all");
  const [feeFilter, setFeeFilter] = React.useState<FeeFilter>("all");
  const [tableSort, setTableSort] = React.useState<TableSortState>(null);
  const [isHydrated, setIsHydrated] = React.useState(false);
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [detailTransaction, setDetailTransaction] =
    React.useState<Transaction | null>(null);
  const [detailInitialTab, setDetailInitialTab] = React.useState("details");
  const [attachmentListOverride, setAttachmentListOverride] = React.useState<{
    transactionId: string;
    attachments: AttachmentRecord[];
  } | null>(null);
  const [reuseDialogOpen, setReuseDialogOpen] = React.useState(false);
  const [reuseSourceTransactionId, setReuseSourceTransactionId] =
    React.useState("");
  const pendingDetailLinkRef = React.useRef(readTransactionDetailParams());
  const tableRef = React.useRef<HTMLDivElement>(null);
  const tableScrollRef = React.useRef<HTMLDivElement>(null);
  const lastAutoLoadRowCountRef = React.useRef<number | null>(null);
  const [drafts, setDrafts] = React.useState<Record<string, TransactionEditDraft>>(
    {},
  );
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const attachmentAdd = useDaemonMutation<AttachmentRecord>("ui.attachments.add");
  const attachmentCopy = useDaemonMutation<AttachmentsCopyData>(
    "ui.attachments.copy",
  );
  const attachmentRename =
    useDaemonMutation<AttachmentRecord>("ui.attachments.rename");
  const attachmentRemove = useDaemonMutation<AttachmentRecord>(
    "ui.attachments.remove",
  );
  const attachmentOpen =
    useDaemonMutation<AttachmentOpenData>("ui.attachments.open");
  const unpairTransfer = useDaemonMutation("ui.transfers.unpair");
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  // Per-transaction loan marks (replaces the old facility "Loans" screen).
  // `ui.loans.list` returns the marked transactions; `mark`/`unmark` flip a tx
  // between normal tax treatment and a loan non-event role. Mutations fall
  // through to the default broad daemon-query invalidation, so the transactions
  // list and this loans query both refresh after a change.
  const loansQuery = useDaemon<LoansList>("ui.loans.list");
  const markCollateral = useDaemonMutation("ui.loans.mark");
  const unmarkCollateral = useDaemonMutation("ui.loans.unmark");
  const linkLoanMarks = useDaemonMutation("ui.loans.link");
  const loanMarkByTransaction = React.useMemo(() => {
    const map = new Map<string, LoanMark>();
    for (const mark of loansQuery.data?.data?.marks ?? []) {
      map.set(mark.transaction_id, mark);
    }
    return map;
  }, [loansQuery.data?.data?.marks]);
  const collateralRoleByTransaction = React.useMemo(() => {
    const map = new Map<string, string>();
    for (const [transactionId, mark] of loanMarkByTransaction) {
      map.set(transactionId, mark.role);
    }
    return map;
  }, [loanMarkByTransaction]);
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({
      notifyStart: true,
      notifyAlreadyRunning: true,
    });
  const attachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: detailTransaction?.id ?? "" },
    { enabled: Boolean(detailTransaction) },
  );
  const historyQuery = useDaemon<TransactionHistoryList>(
    "ui.transactions.history",
    { transaction: detailTransaction?.id ?? "", limit: 25 },
    { enabled: Boolean(detailTransaction) },
  );
  const reuseSourceAttachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: reuseSourceTransactionId },
    { enabled: reuseDialogOpen && Boolean(reuseSourceTransactionId) },
  );
  const journalEventsQuery = useDaemon<JournalEventsData>(
    "ui.journals.events.list",
    { transaction: detailTransaction?.id ?? "", limit: 20 },
    { enabled: Boolean(detailTransaction) },
  );
  const commercialContextQuery = useDaemon<CommercialContextData>(
    "ui.transactions.commercial_context",
    { transaction: detailTransaction?.id ?? "" },
    { enabled: Boolean(detailTransaction) },
  );
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const displayFlow = React.useCallback(
    (txn: Transaction): TransactionFlow =>
      swapCandidateIds.has(txn.id) ? "swap" : transactionFlow(txn),
    [swapCandidateIds],
  );
  // The Wallet dropdown owns leg-mode wallet selections only — it shares
  // `breakdownSelection` with the breakdown chart and the "Show all" deep link
  // behind one removable chip, but a chart-driven (exact) wallet selection is
  // not a leg filter, so the dropdown does not claim it as active/checked.
  const walletFilterActive =
    breakdownSelection?.dimension === "wallet" &&
    breakdownSelection.match === "leg";
  const selectedWalletKey = walletFilterActive ? breakdownSelection.key : null;
  // Options are the distinct single-wallet legs present in the data — so a
  // transfer row "Cold Storage → Vault" contributes "Cold Storage" and "Vault",
  // never the combined string. The active key is always offered even if the
  // current period filtered all its rows out.
  const walletOptions = React.useMemo(() => {
    const legs = new Set<string>();
    for (const txn of records) {
      for (const leg of accountLegs(txn.wallet)) legs.add(leg);
    }
    if (selectedWalletKey) legs.add(selectedWalletKey);
    return Array.from(legs).sort((a, b) => a.localeCompare(b));
  }, [records, selectedWalletKey]);
  // Selecting/clearing a leg-mode wallet goes through breakdownSelection; the
  // dashboard derives the parent's server-side wallet scope from it (so every
  // clear path stays in sync, not just this dropdown).
  const selectWalletScope = React.useCallback(
    (key: string) =>
      onBreakdownSelectionChange({ dimension: "wallet", key, match: "leg" }),
    [onBreakdownSelectionChange],
  );
  const clearWalletScope = React.useCallback(
    () => onBreakdownSelectionChange(null),
    [onBreakdownSelectionChange],
  );
  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
  );
  const saveTransactionDraft = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      setSaveError(null);
      const sourceTransaction = records.find((txn) => txn.id === transactionId);
      const baseline = sourceTransaction
        ? drafts[transactionId] ?? draftForTransaction(sourceTransaction)
        : null;
      const persistedTagCodes = new Set(
        (sourceTransaction?.tags ?? []).map((tag) => tag.toLowerCase()),
      );
      const shouldPersistLabel =
        draft.label &&
        draft.label !== "Unlabeled" &&
        (persistedTagCodes.has(draft.label.toLowerCase()) ||
          draft.label !== baseline?.label);
      const tags = [
        shouldPersistLabel ? draft.label : "",
        ...draft.tags,
      ].filter(Boolean);
      const pricingDirty = baseline
        ? draft.pricingSourceKind !== baseline.pricingSourceKind ||
          draft.pricingQuality !== baseline.pricingQuality ||
          draft.manualCurrency !== baseline.manualCurrency ||
          draft.manualPrice !== baseline.manualPrice ||
          draft.manualValue !== baseline.manualValue ||
          draft.manualSource !== baseline.manualSource
        : false;
      const reviewTaxDirty = baseline
        ? draft.reviewStatus !== baseline.reviewStatus ||
          draft.taxable !== baseline.taxable ||
          draft.atRegime !== baseline.atRegime ||
          draft.atCategory !== baseline.atCategory
        : false;
      const manualPrice = parseManualDecimal(draft.manualPrice);
      const manualValue = parseManualDecimal(draft.manualValue);
      await metadataUpdate.mutateAsync({
        transaction: transactionId,
        note: draft.note.trim() ? draft.note : null,
        tags: Array.from(new Set(tags)),
        excluded: draft.excluded,
        ...(reviewTaxDirty
          ? {
              review_status: draft.reviewStatus,
              taxable: draft.taxable,
              at_regime: draft.atRegime,
              at_category: draft.atCategory,
            }
          : {}),
        ...(pricingDirty
          ? {
              pricing_source_kind: draft.pricingSourceKind,
              pricing_quality: draft.pricingQuality,
              fiat_currency: draft.manualCurrency.trim().toUpperCase(),
              fiat_rate: manualPrice === null ? null : draft.manualPrice,
              fiat_value: manualValue === null ? null : draft.manualValue,
              pricing_external_ref: draft.manualSource.trim() || null,
            }
          : {}),
      });
      setDrafts((current) => ({
        ...current,
        [transactionId]: draft,
      }));
    },
    [drafts, metadataUpdate, records],
  );

  const openTransactionDetail = React.useCallback(
    (txn: Transaction, tab = "details") => {
      setSaveError(null);
      setDetailInitialTab(tab);
      setDetailTransaction(txn);
      updateTransactionDetailParams(txn.id, tab);
    },
    [],
  );
  React.useEffect(() => {
    if (!deepLinkedTransactionId) return;
    if (
      detailTransaction &&
      matchesTransactionDeepLink(detailTransaction, deepLinkedTransactionId)
    ) {
      return;
    }
    const transaction = records.find((txn) =>
      matchesTransactionDeepLink(txn, deepLinkedTransactionId),
    );
    if (transaction) {
      pendingDetailLinkRef.current = { transactionId: null, tab: "details" };
      openTransactionDetail(transaction, deepLinkedTransactionTab);
      return;
    }
    pendingDetailLinkRef.current = {
      transactionId: deepLinkedTransactionId,
      tab: deepLinkedTransactionTab,
    };
  }, [
    deepLinkedTransactionId,
    deepLinkedTransactionTab,
    detailTransaction,
    openTransactionDetail,
    records,
  ]);
  const detailAttachmentRecords = React.useMemo(
    () => {
      if (
        attachmentListOverride &&
        attachmentListOverride.transactionId === detailTransaction?.id
      ) {
        return attachmentListOverride.attachments;
      }
      return attachmentsQuery.data?.data?.attachments ?? [];
    },
    [
      attachmentListOverride,
      attachmentsQuery.data?.data?.attachments,
      detailTransaction?.id,
    ],
  );
  const attachmentItems = React.useMemo(
    () =>
      detailAttachmentRecords.map((record) =>
        // loose translator
        attachmentRecordToItem(record, t as (key: string) => string),
      ),
    [detailAttachmentRecords, t],
  );
  React.useEffect(() => {
    setAttachmentListOverride(null);
  }, [detailTransaction?.id]);
  const updateDetailAttachmentRecords = React.useCallback(
    (updater: (attachments: AttachmentRecord[]) => AttachmentRecord[]) => {
      if (!detailTransaction) return;
      setAttachmentListOverride((current) => {
        const currentAttachments =
          current?.transactionId === detailTransaction.id
            ? current.attachments
            : attachmentsQuery.data?.data?.attachments ?? [];
        return {
          transactionId: detailTransaction.id,
          attachments: updater(currentAttachments),
        };
      });
    },
    [attachmentsQuery.data?.data?.attachments, detailTransaction],
  );
  const updateAttachmentListQueryCache = React.useCallback(
    (
      transactionId: string,
      updater: (attachments: AttachmentRecord[]) => AttachmentRecord[],
    ) => {
      queryClient.setQueriesData<DaemonEnvelope<AttachmentsListData>>(
        {
          queryKey: ["daemon"],
          predicate: (query) =>
            isAttachmentListQueryKeyForTransaction(
              query.queryKey,
              transactionId,
            ),
        },
        (current) =>
          current?.data
            ? {
                ...current,
                data: {
                  ...current.data,
                  attachments: updater(current.data.attachments),
                },
              }
            : current,
      );
    },
    [queryClient],
  );
  const evidenceSourceTransactions = React.useMemo(
    () =>
      detailTransaction
        ? records.filter((txn) => txn.id !== detailTransaction.id)
        : [],
    [detailTransaction, records],
  );
  React.useEffect(() => {
    if (!reuseDialogOpen) return;
    if (
      reuseSourceTransactionId &&
      evidenceSourceTransactions.some(
        (transaction) => transaction.id === reuseSourceTransactionId,
      )
    ) {
      return;
    }
    setReuseSourceTransactionId(evidenceSourceTransactions[0]?.id ?? "");
  }, [evidenceSourceTransactions, reuseDialogOpen, reuseSourceTransactionId]);
  const reuseSourceAttachmentItems = React.useMemo(
    () =>
      (reuseSourceAttachmentsQuery.data?.data?.attachments ?? []).map((record) =>
        // loose translator
        attachmentRecordToItem(record, t as (key: string) => string),
      ),
    [reuseSourceAttachmentsQuery.data, t],
  );
  const journalEvents = journalEventsQuery.data?.data?.events ?? [];
  const commercialContext = commercialContextQuery.data?.data;
  const historyData = historyQuery.data?.data;
  const revertHistoryTarget = React.useCallback(
    async (target: HistoryRevertTarget) => {
      if (!detailTransaction) return;
      await revertHistory.mutateAsync({
        transaction: detailTransaction.id,
        event: target.event.id,
        ...(target.field ? { field: target.field.field } : {}),
        reason: target.field
          ? t("history.revertReasonField", { label: target.field.label })
          : t("history.revertReasonEvent"),
      });
      useUiStore.getState().addNotification({
        title: t("notification.editReverted.title"),
        body: t("notification.editReverted.body"),
        tone: "success",
        dedupeKey: `history-revert-${target.event.id}-${target.field?.field ?? "event"}`,
      });
    },
    [detailTransaction, revertHistory, t],
  );

  const markTransactionCollateral = React.useCallback(
    async (txn: Transaction, as: LoanMarkTarget) => {
      try {
        await markCollateral.mutateAsync({ txid: txn.id, as });
        useUiStore.getState().addNotification({
          title: t("notification.loanMarked.title"),
          body: t("notification.loanMarked.body"),
          tone: "success",
          dedupeKey: `loan-mark-${txn.id}`,
        });
      } catch (error) {
        useUiStore.getState().addNotification({
          title: t("notification.loanMarkFailed.title"),
          body:
            error instanceof Error
              ? error.message
              : t("notification.loanMarkFailed.body"),
          tone: "error",
          dedupeKey: `loan-mark-failed-${txn.id}`,
        });
      }
    },
    [markCollateral, t],
  );
  const unmarkTransactionCollateral = React.useCallback(
    async (txn: Transaction) => {
      try {
        await unmarkCollateral.mutateAsync({ txid: txn.id });
        useUiStore.getState().addNotification({
          title: t("notification.loanUnmarked.title"),
          body: t("notification.loanUnmarked.body"),
          tone: "success",
          dedupeKey: `loan-unmark-${txn.id}`,
        });
      } catch (error) {
        useUiStore.getState().addNotification({
          title: t("notification.loanMarkFailed.title"),
          body:
            error instanceof Error
              ? error.message
              : t("notification.loanMarkFailed.body"),
          tone: "error",
          dedupeKey: `loan-unmark-failed-${txn.id}`,
        });
      }
    },
    [unmarkCollateral, t],
  );
  const linkTransactionLoan = React.useCallback(
    async (txn: Transaction, targetTransactionId: string) => {
      try {
        await linkLoanMarks.mutateAsync({ txids: [txn.id, targetTransactionId] });
        useUiStore.getState().addNotification({
          title: t("notification.loanLinked.title"),
          body: t("notification.loanLinked.body"),
          tone: "success",
          dedupeKey: `loan-link-${txn.id}-${targetTransactionId}`,
        });
      } catch (error) {
        useUiStore.getState().addNotification({
          title: t("notification.loanLinkFailed.title"),
          body:
            error instanceof Error
              ? error.message
              : t("notification.loanLinkFailed.body"),
          tone: "error",
          dedupeKey: `loan-link-failed-${txn.id}-${targetTransactionId}`,
        });
      }
    },
    [linkLoanMarks, t],
  );

  const hasActiveFilters =
    chartSelection !== null ||
    quickFilter !== null ||
    breakdownSelection !== null ||
    statusFilter !== "all" ||
    flowFilter !== "all" ||
    paymentMethodFilter !== "all" ||
    feeFilter !== "all";

  const clearFilters = () => {
    onChartSelectionChange(null);
    onQuickFilterChange(null);
    onBreakdownSelectionChange(null);
    setStatusFilter("all");
    setFlowFilter("all");
    setPaymentMethodFilter("all");
    setFeeFilter("all");
  };

  React.useEffect(() => {
    if (resetTableFiltersToken === 0) return;
    setStatusFilter("all");
    setFlowFilter("all");
    setPaymentMethodFilter("all");
    setFeeFilter("all");
  }, [resetTableFiltersToken]);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);

    const nextStatus = params.get("status");
    if (
      nextStatus &&
      (nextStatus === "all" ||
        allTransactionStatuses.includes(nextStatus as TransactionStatus))
    ) {
      setStatusFilter(nextStatus);
    }

    const nextFlow = params.get("flow");
    if (
      nextFlow &&
      (nextFlow === "all" ||
        allTransactionFlows.includes(nextFlow as TransactionFlow))
    ) {
      setFlowFilter(nextFlow);
    }

    const nextPayment = params.get("payment");
    if (
      nextPayment &&
      (nextPayment === "all" ||
        allPaymentMethods.includes(
          nextPayment as (typeof allPaymentMethods)[number],
        ))
    ) {
      setPaymentMethodFilter(nextPayment);
    }

    const nextFees = params.get("fees");
    if (nextFees === "with-fees" || nextFees === "true" || nextFees === "1") {
      setFeeFilter("with-fees");
    } else if (nextFees === "all") {
      setFeeFilter("all");
    }

    if (params.get("sort") === "amount" || params.get("sort") === "date") {
      const sortKey = params.get("sort") === "date" ? "date" : "amount";
      const nextOrder = params.get("order");
      if (nextOrder === "asc" || nextOrder === "desc") {
        setTableSort({ key: sortKey, direction: nextOrder });
      }
    }

    setIsHydrated(true);
  }, []);

  React.useEffect(() => {
    const pending = pendingDetailLinkRef.current;
    if (!pending.transactionId) return;
    const transaction = records.find((txn) =>
      matchesTransactionDeepLink(txn, pending.transactionId ?? ""),
    );
    if (!transaction) return;
    pendingDetailLinkRef.current = { transactionId: null, tab: "details" };
    openTransactionDetail(transaction, pending.tab);
  }, [records, openTransactionDetail]);

  const dateSortDirection =
    tableSort?.key === "date" ? tableSort.direction : null;
  const amountSortDirection =
    tableSort?.key === "amount" ? tableSort.direction : null;
  const dateSortButtonLabel =
    dateSortDirection === "desc"
      ? t("table.sort.dateDescending")
      : dateSortDirection === "asc"
        ? t("table.sort.dateAscending")
        : t("table.sort.dateInactive");
  const DateSortIcon =
    dateSortDirection === "desc"
      ? ArrowDown
      : dateSortDirection === "asc"
        ? ArrowUp
        : ArrowUpDown;
  const amountSortButtonLabel =
    amountSortDirection === "desc"
      ? t("table.sort.amountDescending")
      : amountSortDirection === "asc"
        ? t("table.sort.amountAscending")
        : t("table.sort.amountInactive");
  const AmountSortIcon =
    amountSortDirection === "desc"
      ? ArrowDown
      : amountSortDirection === "asc"
        ? ArrowUp
        : ArrowUpDown;
  const toggleDateSort = React.useCallback(() => {
    setTableSort((current) => {
      if (current?.key !== "date") return { key: "date", direction: "asc" };
      if (current.direction === "asc") return { key: "date", direction: "desc" };
      return { key: "date", direction: "asc" };
    });
  }, []);
  const toggleAmountSort = React.useCallback(() => {
    setTableSort((current) => {
      if (current?.key !== "amount") return { key: "amount", direction: "desc" };
      if (current.direction === "desc") {
        return { key: "amount", direction: "asc" };
      }
      return null;
    });
  }, []);
  const scrollTableIntoView = React.useCallback(() => {
    if (typeof window === "undefined") return;
    const table = tableRef.current;
    if (!table) return;
    const periodNav = document.getElementById("transactions-period-nav");
    const periodNavBottom = periodNav?.getBoundingClientRect().bottom ?? 88;
    const viewportTopOffset = Math.max(104, periodNavBottom + 12);
    let scrollParent = table.parentElement;
    while (scrollParent) {
      const style = window.getComputedStyle(scrollParent);
      const canScroll =
        /(auto|scroll|overlay)/.test(style.overflowY) &&
        scrollParent.scrollHeight > scrollParent.clientHeight;
      if (canScroll) break;
      scrollParent = scrollParent.parentElement;
    }

    if (scrollParent) {
      const nextTop =
        scrollParent.scrollTop +
        table.getBoundingClientRect().top -
        viewportTopOffset;
      scrollParent.scrollTo({
        top: Math.max(0, nextTop),
        behavior: "smooth",
      });
      return;
    }

    const top = table.getBoundingClientRect().top + window.scrollY - viewportTopOffset;
    window.scrollTo({
      top: Math.max(0, top),
      behavior: "smooth",
    });
  }, []);
  const handleTableToolbarClick = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target;
      if (!(target instanceof HTMLElement)) return;
      if (
        target.closest(
          'button, a, input, select, textarea, [role="button"], [role="menuitem"], [data-radix-collection-item]',
        )
      ) {
        return;
      }
      scrollTableIntoView();
    },
    [scrollTableIntoView],
  );

  const filteredTransactions = React.useMemo(() => {
    const filtered = records.filter((txn) => {
      const draft = getDraft(txn);
      const matchesStatus =
        statusFilter === "all" || draft.reviewStatus === statusFilter;

      const matchesFlow =
        flowFilter === "all" || displayFlow(txn) === flowFilter;

      const matchesPaymentMethod =
        paymentMethodFilter === "all" ||
        txn.paymentMethod === paymentMethodFilter;

      const matchesFees =
        feeFilter === "all" || (txn.feeBtc ?? 0) > 0 || (txn.feeEur ?? 0) > 0;

      const matchesChartSelection =
        !chartSelection ||
        matchesFlowChartSelection(txn, chartSelection, displayFlow);

      const matchesQuickFilter =
        quickFilter === null ||
        (quickFilter === "external_flow" &&
          ["incoming", "outgoing"].includes(displayFlow(txn))) ||
        (quickFilter === "review_queue" &&
          draft.reviewStatus !== "completed") ||
        (quickFilter === "no_explorer_id" && !txn.explorerId) ||
        (quickFilter === "missing_price" && !txn.rate) ||
        (quickFilter === "failed_import" && draft.reviewStatus === "failed");

      const matchesBreakdownSelection =
        !breakdownSelection ||
        (breakdownSelection.dimension === "network" &&
          txn.paymentMethod === breakdownSelection.key) ||
        // Wallet matching depends on how the selection was made. Leg-aware
        // ("Cold Storage" also surfaces "Cold Storage → Vault") for the Wallet
        // dropdown and the "Show all" deep link — matching the Wallet Detail
        // recent-transactions list. Exact (full account string) for chart-bar
        // clicks, so the table count stays equal to the clicked bar's count.
        (breakdownSelection.dimension === "wallet" &&
          (breakdownSelection.match === "leg"
            ? accountMatchesLabel(txn.wallet, breakdownSelection.key)
            : (txn.wallet ?? "Unassigned") === breakdownSelection.key));

      return (
        matchesChartSelection &&
        matchesQuickFilter &&
        matchesBreakdownSelection &&
        matchesStatus &&
        matchesFlow &&
        matchesPaymentMethod &&
        matchesFees
      );
    });
    const originalIndex = new Map(records.map((txn, index) => [txn.id, index]));
    if (tableSort === null) return filtered;

    if (tableSort.key === "date") {
      return [...filtered].sort((left, right) => {
        const leftValue = sortableTransactionDateValue(left.date);
        const rightValue = sortableTransactionDateValue(right.date);
        if (leftValue === null && rightValue === null) {
          return (originalIndex.get(left.id) ?? 0) - (originalIndex.get(right.id) ?? 0);
        }
        if (leftValue === null) return 1;
        if (rightValue === null) return -1;
        const delta =
          tableSort.direction === "asc"
            ? leftValue - rightValue
            : rightValue - leftValue;
        return delta === 0
          ? (originalIndex.get(left.id) ?? 0) - (originalIndex.get(right.id) ?? 0)
          : delta;
      });
    }

    const amountValue = (txn: Transaction) => {
      const flow = displayFlow(txn);
      const sign = flow === "outgoing" ? -1 : 1;
      if (currency === "btc") return sign * transactionBtc(txn);
      return txn.amount === null ? null : sign * txn.amount;
    };

    return [...filtered].sort((left, right) => {
      const leftValue = amountValue(left);
      const rightValue = amountValue(right);
      if (leftValue === null && rightValue === null) {
        return (originalIndex.get(left.id) ?? 0) - (originalIndex.get(right.id) ?? 0);
      }
      if (leftValue === null) return 1;
      if (rightValue === null) return -1;
      const delta =
        tableSort.direction === "asc"
          ? leftValue - rightValue
          : rightValue - leftValue;
      return delta === 0
        ? (originalIndex.get(left.id) ?? 0) - (originalIndex.get(right.id) ?? 0)
        : delta;
    });
  }, [
    records,
    getDraft,
    chartSelection,
    quickFilter,
    breakdownSelection,
    statusFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    displayFlow,
    tableSort,
    currency,
  ]);

  React.useEffect(() => {
    if (
      (!chartSelection && !quickFilter && !breakdownSelection) ||
      typeof window === "undefined"
    ) {
      return;
    }
    window.requestAnimationFrame(() => {
      scrollTableIntoView();
    });
  }, [chartSelection, quickFilter, breakdownSelection, scrollTableIntoView]);

  const virtualRowCount = isRefreshing
    ? 10
    : filteredTransactions.length + (hasMoreRecords ? 1 : 0);
  const rowVirtualizer = useVirtualizer({
    count: virtualRowCount,
    getScrollElement: () => tableScrollRef.current,
    estimateSize: () => 76,
    overscan: 8,
    getItemKey: (index) =>
      isRefreshing
        ? `refresh-${index}`
        : (filteredTransactions[index]?.id ?? `loader-${index}`),
  });
  const virtualRows = rowVirtualizer.getVirtualItems();
  const firstVirtualRow = virtualRows[0];
  const lastVirtualRow = virtualRows[virtualRows.length - 1];
  const lastVirtualIndex = lastVirtualRow?.index ?? -1;
  const paddingTop = firstVirtualRow?.start ?? 0;
  const paddingBottom =
    lastVirtualRow !== undefined
      ? Math.max(0, rowVirtualizer.getTotalSize() - lastVirtualRow.end)
      : 0;
  const loadedRecordCount = filteredTransactions.length;
  const isCompleteRecordSet = !hasMoreRecords;

  React.useEffect(() => {
    tableScrollRef.current?.scrollTo({ top: 0 });
  }, [
    chartSelection,
    quickFilter,
    breakdownSelection,
    statusFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    tableSort,
  ]);

  React.useEffect(() => {
    if (isRefreshing) return;
    if (!hasMoreRecords || isLoadingMoreRecords || !onLoadMoreRecords) {
      lastAutoLoadRowCountRef.current = null;
      return;
    }
    if (lastVirtualIndex < 0) return;
    const prefetchFromIndex = Math.max(0, filteredTransactions.length - 8);
    if (lastVirtualIndex < prefetchFromIndex) return;
    if (lastAutoLoadRowCountRef.current === filteredTransactions.length) return;
    lastAutoLoadRowCountRef.current = filteredTransactions.length;
    onLoadMoreRecords();
  }, [
    filteredTransactions.length,
    hasMoreRecords,
    isLoadingMoreRecords,
    isRefreshing,
    lastVirtualIndex,
    onLoadMoreRecords,
  ]);

  React.useEffect(() => {
    if (isLoadingMoreRecords) return;
    if (lastAutoLoadRowCountRef.current !== filteredTransactions.length) {
      lastAutoLoadRowCountRef.current = null;
    }
  }, [filteredTransactions.length, isLoadingMoreRecords]);

  React.useEffect(() => {
    if (!isHydrated || typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    params.delete("q");
    params.delete("page");
    params.delete("pageSize");
    params.delete("date");

    if (statusFilter !== "all") {
      params.set("status", statusFilter);
    } else {
      params.delete("status");
    }

    if (flowFilter !== "all") {
      params.set("flow", flowFilter);
    } else {
      params.delete("flow");
    }

    if (paymentMethodFilter !== "all") {
      params.set("payment", paymentMethodFilter);
    } else {
      params.delete("payment");
    }

    if (feeFilter === "with-fees") {
      params.set("fees", feeFilter);
    } else {
      params.delete("fees");
    }

    if (tableSort) {
      params.set("sort", tableSort.key);
      params.set("order", tableSort.direction);
    } else {
      params.delete("sort");
      params.delete("order");
    }

    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [
    statusFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    tableSort,
    isHydrated,
  ]);

  const activeFilterCount = [
    chartSelection,
    quickFilter,
    breakdownSelection,
    statusFilter !== "all",
    flowFilter !== "all",
    paymentMethodFilter !== "all",
    feeFilter !== "all",
  ].filter(Boolean).length;

  const headerFilterButtonClassName =
    "inline-flex h-7 items-center gap-1 rounded-md px-1.5 transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

  const renderStatusFilterItems = () => (
    <>
      <DropdownMenuLabel>{t("table.filter.statusLabel")}</DropdownMenuLabel>
      <DropdownMenuCheckboxItem
        checked={statusFilter === "all"}
        onCheckedChange={() => setStatusFilter("all")}
      >
        {t("table.filter.allStatuses")}
      </DropdownMenuCheckboxItem>
      {allTransactionStatuses.map((status) => (
        <DropdownMenuCheckboxItem
          key={status}
          checked={statusFilter === status}
          onCheckedChange={() => setStatusFilter(status)}
        >
          {/* loose translator */}
          {(t as (key: string) => string)(transactionStatusLabels[status])}
        </DropdownMenuCheckboxItem>
      ))}
    </>
  );

  const renderFlowFilterItems = () => (
    <>
      <DropdownMenuLabel>{t("table.filter.flowLabel")}</DropdownMenuLabel>
      <DropdownMenuCheckboxItem
        checked={flowFilter === "all"}
        onCheckedChange={() => setFlowFilter("all")}
      >
        {t("table.filter.allFlows")}
      </DropdownMenuCheckboxItem>
      {allTransactionFlows.map((flow) => (
        <DropdownMenuCheckboxItem
          key={flow}
          checked={flowFilter === flow}
          onCheckedChange={() => setFlowFilter(flow)}
        >
          {/* loose translator */}
          {(t as (key: string) => string)(transactionFlowLabels[flow])}
        </DropdownMenuCheckboxItem>
      ))}
    </>
  );

  const renderNetworkFilterItems = () => (
    <>
      <DropdownMenuLabel>{t("table.filter.networkLabel")}</DropdownMenuLabel>
      <DropdownMenuCheckboxItem
        checked={paymentMethodFilter === "all"}
        onCheckedChange={() => setPaymentMethodFilter("all")}
      >
        {t("table.filter.allNetworks")}
      </DropdownMenuCheckboxItem>
      {allPaymentMethods.map((method) => (
        <DropdownMenuCheckboxItem
          key={method}
          checked={paymentMethodFilter === method}
          onCheckedChange={() => setPaymentMethodFilter(method)}
        >
          {method}
        </DropdownMenuCheckboxItem>
      ))}
    </>
  );

  const renderWalletFilterItems = () => (
    <>
      <DropdownMenuLabel>{t("table.filter.walletLabel")}</DropdownMenuLabel>
      <DropdownMenuCheckboxItem
        checked={!walletFilterActive}
        onCheckedChange={() => {
          // Only clear when a wallet is selected — don't clobber an active
          // network breakdown (both share breakdownSelection).
          if (walletFilterActive) clearWalletScope();
        }}
      >
        {t("table.filter.allWallets")}
      </DropdownMenuCheckboxItem>
      {walletOptions.map((wallet) => (
        <DropdownMenuCheckboxItem
          key={wallet}
          checked={selectedWalletKey === wallet}
          onCheckedChange={() => selectWalletScope(wallet)}
        >
          {wallet}
        </DropdownMenuCheckboxItem>
      ))}
    </>
  );

  const renderFeeFilterItems = () => (
    <>
      <DropdownMenuLabel>{t("table.filter.feesLabel")}</DropdownMenuLabel>
      <DropdownMenuCheckboxItem
        checked={feeFilter === "all"}
        onCheckedChange={() => setFeeFilter("all")}
      >
        {t("table.filter.allFees")}
      </DropdownMenuCheckboxItem>
      <DropdownMenuCheckboxItem
        checked={feeFilter === "with-fees"}
        onCheckedChange={() => setFeeFilter("with-fees")}
      >
        {t("table.withFees")}
      </DropdownMenuCheckboxItem>
    </>
  );

  const allLoanMarks = loansQuery.data?.data?.marks ?? [];
  const detailLoanMark = detailTransaction
    ? loanMarkByTransaction.get(detailTransaction.id) ?? null
    : null;
  const detailLinkedLoanMarks =
    detailLoanMark?.loan_id
      ? allLoanMarks.filter(
          (mark) =>
            mark.loan_id === detailLoanMark.loan_id &&
            mark.transaction_id !== detailLoanMark.transaction_id,
        )
      : [];
  const detailLoanLinkCandidates = detailLoanMark
    ? allLoanMarks.filter(
        (mark) =>
          mark.transaction_id !== detailLoanMark.transaction_id &&
          (!detailLoanMark.loan_id || mark.loan_id !== detailLoanMark.loan_id),
      )
    : [];

  return (
    <>
      <div
        ref={tableRef}
        className="scroll-mt-24 rounded-xl border bg-card"
        role={isRefreshing ? "status" : undefined}
        aria-live={isRefreshing ? "polite" : undefined}
      >
      <div
        className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5"
        onClick={handleTableToolbarClick}
      >
        <div className="flex flex-1 items-center gap-2">
          <span className="text-sm font-medium sm:text-base">{t("table.title")}</span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  activeFilterCount > 0 && "border-primary",
                )}
                aria-label={t("table.filter.menuAria")}
              >
                <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                <span>{t("table.filter.menuTrigger")}</span>
                {activeFilterCount > 0 ? (
                  <span className="grid min-w-4 place-items-center rounded-full bg-primary px-1 text-[10px] font-semibold leading-4 text-primary-foreground">
                    {activeFilterCount}
                  </span>
                ) : null}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              className="w-[220px]"
            >
              <DropdownMenuLabel>{t("table.filter.menuLabel")}</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("table.filter.statusTrigger")}</span>
                  {statusFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[180px]">
                  {renderStatusFilterItems()}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("table.filter.flowTrigger")}</span>
                  {flowFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[190px]">
                  {renderFlowFilterItems()}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("table.filter.networkTrigger")}</span>
                  {paymentMethodFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[200px]">
                  {renderNetworkFilterItems()}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger disabled={walletOptions.length === 0}>
                  <span>{t("table.filter.walletTrigger")}</span>
                  {walletFilterActive ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="max-h-[320px] w-[220px] overflow-y-auto">
                  {renderWalletFilterItems()}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <span>{t("table.filter.feesTrigger")}</span>
                  {feeFilter !== "all" ? (
                    <span className="ml-1 size-1.5 rounded-full bg-primary" />
                  ) : null}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="w-[180px]">
                  {renderFeeFilterItems()}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {hasActiveFilters && (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            {t("table.filters")}
          </span>
          {chartSelection && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onChartSelectionChange(null)}
              aria-label={t("table.chip.clearChart", {
                // loose translator
                label: flowChartSelectionLabel(
                  chartSelection,
                  t as (key: string, opts?: Record<string, unknown>) => string,
                ),
              })}
            >
              {t("table.chip.chartPrefix", {
                // loose translator
                label: flowChartSelectionLabel(
                  chartSelection,
                  t as (key: string, opts?: Record<string, unknown>) => string,
                ),
              })}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {quickFilter && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onQuickFilterChange(null)}
              aria-label={t("table.chip.clearQuick", {
                // loose translator
                label: (t as (key: string) => string)(
                  quickFilterLabel(quickFilter),
                ),
              })}
            >
              {/* loose translator */}
              {(t as (key: string) => string)(quickFilterLabel(quickFilter))}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {breakdownSelection && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onBreakdownSelectionChange(null)}
              aria-label={t("table.chip.clearBreakdown", {
                // loose translator
                label: breakdownSelectionLabel(
                  breakdownSelection,
                  t as (key: string, opts?: Record<string, unknown>) => string,
                ),
              })}
            >
              {/* loose translator */}
              {breakdownSelectionLabel(
                breakdownSelection,
                t as (key: string, opts?: Record<string, unknown>) => string,
              )}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {statusFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setStatusFilter("all")}
              aria-label={t("table.chip.clearStatus", {
                // loose translator
                label: (t as (key: string) => string)(
                  transactionStatusLabels[statusFilter as TransactionStatus],
                ),
              })}
            >
              {/* loose translator */}
              {(t as (key: string) => string)(
                transactionStatusLabels[statusFilter as TransactionStatus],
              )}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {flowFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setFlowFilter("all")}
              aria-label={t("table.chip.clearFlow", {
                // loose translator
                label: (t as (key: string) => string)(
                  transactionFlowLabels[flowFilter as TransactionFlow],
                ),
              })}
            >
              {/* loose translator */}
              {(t as (key: string) => string)(
                transactionFlowLabels[flowFilter as TransactionFlow],
              )}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {paymentMethodFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setPaymentMethodFilter("all")}
              aria-label={t("table.chip.clearPayment", {
                label: paymentMethodFilter,
              })}
            >
              {paymentMethodFilter}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {feeFilter === "with-fees" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setFeeFilter("all")}
              aria-label={t("table.chip.clearWithFees")}
            >
              {t("table.withFees")}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          <button
            onClick={clearFilters}
            className="text-[10px] text-destructive hover:underline sm:text-xs"
          >
            {t("table.clearAll")}
          </button>
        </div>
      )}

      <div
        ref={tableScrollRef}
        className="overflow-auto px-3 pb-3 sm:px-6 sm:pb-4"
        role="region"
        aria-label={t("table.virtual.scrollRegion")}
        tabIndex={0}
        style={{ maxHeight: "clamp(560px, calc(100vh - 18rem), 1120px)" }}
      >
        <table
          data-slot="table"
          className="w-full min-w-[1080px] table-fixed caption-bottom text-sm"
        >
          <colgroup>
            <col className="w-[34%]" />
            <col className="w-[150px]" />
            <col className="hidden w-[190px] md:table-column" />
            <col className="hidden w-[180px] lg:table-column" />
            <col className="hidden w-[150px] xl:table-column" />
            <col className="w-[130px]" />
            <col className="w-[48px]" />
          </colgroup>
          <TableHeader className="sticky top-0 z-20 bg-card">
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead
                className="w-[34%] text-xs font-medium text-muted-foreground sm:text-sm"
                aria-sort={
                  dateSortDirection === "desc"
                    ? "descending"
                    : dateSortDirection === "asc"
                      ? "ascending"
                      : "none"
                }
              >
                <button
                  type="button"
                  className={cn(
                    "inline-flex h-7 items-center gap-1 rounded-md px-1.5 transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    dateSortDirection && "text-foreground",
                  )}
                  aria-label={dateSortButtonLabel}
                  title={dateSortButtonLabel}
                  onClick={toggleDateSort}
                >
                  <span>{t("table.column.transaction")}</span>
                  <DateSortIcon className="size-3.5" aria-hidden="true" />
                </button>
              </TableHead>
              <TableHead
                className="w-[150px] text-right text-xs font-medium text-muted-foreground sm:text-sm"
                aria-sort={
                  amountSortDirection === "desc"
                    ? "descending"
                    : amountSortDirection === "asc"
                      ? "ascending"
                      : "none"
                }
              >
                <button
                  type="button"
                  className={cn(
                    "ml-auto inline-flex h-7 items-center justify-end gap-1 rounded-md px-1.5 text-right transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    amountSortDirection && "text-foreground",
                  )}
                  aria-label={amountSortButtonLabel}
                  title={amountSortButtonLabel}
                  onClick={toggleAmountSort}
                >
                  <span>{t("table.column.amount")}</span>
                  <AmountSortIcon className="size-3.5" aria-hidden="true" />
                </button>
              </TableHead>
              <TableHead className="hidden w-[190px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                {t("table.column.accounting")}
              </TableHead>
              <TableHead className="hidden w-[180px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                {t("table.column.pricing")}
              </TableHead>
              <TableHead className="hidden w-[150px] text-xs font-medium text-muted-foreground sm:text-sm xl:table-cell">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button
                      type="button"
                      className={cn(
                        headerFilterButtonClassName,
                        paymentMethodFilter !== "all" && "text-foreground",
                      )}
                      aria-label={t("table.filter.paymentAria")}
                      title={t("table.filter.paymentAria")}
                    >
                      <span>{t("table.column.network")}</span>
                      <Filter className="size-3.5" aria-hidden="true" />
                      {paymentMethodFilter !== "all" ? (
                        <span className="size-1.5 rounded-full bg-primary" />
                      ) : null}
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-[200px]">
                    {renderNetworkFilterItems()}
                  </DropdownMenuContent>
                </DropdownMenu>
              </TableHead>
              <TableHead className="w-[130px] text-xs font-medium text-muted-foreground sm:text-sm">
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button
                      type="button"
                      className={cn(
                        headerFilterButtonClassName,
                        statusFilter !== "all" && "text-foreground",
                      )}
                      aria-label={t("table.filter.statusAria")}
                      title={t("table.filter.statusAria")}
                    >
                      <span>{t("table.column.status")}</span>
                      <Filter className="size-3.5" aria-hidden="true" />
                      {statusFilter !== "all" ? (
                        <span className="size-1.5 rounded-full bg-primary" />
                      ) : null}
                    </button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-[180px]">
                    {renderStatusFilterItems()}
                  </DropdownMenuContent>
                </DropdownMenu>
              </TableHead>
              <TableHead className="w-[48px]"></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {!isRefreshing && virtualRowCount === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={7}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  {t("table.empty")}
                </TableCell>
              </TableRow>
            ) : (
              <>
                {paddingTop > 0 ? (
                  <TableRow aria-hidden="true">
                    <TableCell
                      colSpan={7}
                      style={{ height: paddingTop, padding: 0 }}
                    />
                  </TableRow>
                ) : null}
                {virtualRows.map((virtualRow) => {
                  if (isRefreshing) {
                    return (
                      <tr
                        key={virtualRow.key}
                        ref={rowVirtualizer.measureElement}
                        data-index={virtualRow.index}
                        className="border-b transition-colors"
                      >
                        <TableCell>
                          <div className="space-y-2">
                            <Skeleton className="h-4 w-48 max-w-full" />
                            <Skeleton className="h-3 w-72 max-w-full" />
                          </div>
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="space-y-2">
                            <Skeleton className="ml-auto h-4 w-24" />
                            <Skeleton className="ml-auto h-3 w-16" />
                          </div>
                        </TableCell>
                        <TableCell className="hidden md:table-cell">
                          <Skeleton className="h-5 w-28" />
                        </TableCell>
                        <TableCell className="hidden lg:table-cell">
                          <Skeleton className="h-5 w-24" />
                        </TableCell>
                        <TableCell className="hidden xl:table-cell">
                          <Skeleton className="h-5 w-20" />
                        </TableCell>
                        <TableCell>
                          <Skeleton className="h-6 w-24" />
                        </TableCell>
                        <TableCell>
                          <Skeleton className="size-8 rounded-md" />
                        </TableCell>
                      </tr>
                    );
                  }
                  if (virtualRow.index >= filteredTransactions.length) {
                    return (
                      <tr
                        key={virtualRow.key}
                        ref={rowVirtualizer.measureElement}
                        data-index={virtualRow.index}
                        className="border-b transition-colors"
                      >
                        <TableCell
                          colSpan={7}
                          className="h-16 text-center text-sm text-muted-foreground"
                        >
                          {isLoadingMoreRecords
                            ? t("table.virtual.loadingMore")
                            : onLoadMoreRecords
                              ? (
                                  <Button
                                    variant="outline"
                                    size="sm"
                                    onClick={onLoadMoreRecords}
                                  >
                                    {t("table.virtual.loadMore")}
                                  </Button>
                                )
                              : t("table.virtual.moreAvailable")}
                        </TableCell>
                      </tr>
                    );
                  }
                  const txn = filteredTransactions[virtualRow.index];
                  if (!txn) return null;
                const draft = getDraft(txn);
                const rowTaxClassification = austrianTaxClassificationFor(
                  draft.atRegime,
                  draft.atCategory,
                );
                const rowPricingValue = pricingSelectionValue(
                  draft.pricingSourceKind,
                  draft.pricingQuality,
                );
                const rowPricingSummary =
                  draft.pricingSourceKind === "manual_override"
                    ? null
                    : // loose translator
                      pricingCacheSummary(txn, t as (key: string) => string);
                const StatusIcon = transactionStatusIcons[draft.reviewStatus];
                const flow = displayFlow(txn);
                const collateralRole = collateralRoleByTransaction.get(txn.id);
                const showPrimaryLabel = !isRedundantTransactionLabel(
                  draft.label,
                  flow,
                );
                const tagPreview = draft.tags;
                const networkLabel =
                  flow === "swap" || flow === "layer-transition"
                    ? // loose translator
                      pairRailLabel(
                        txn,
                        t as (key: string, opts?: Record<string, unknown>) => string,
                      )
                    : txn.paymentMethod;
                const amountBtc = transactionBtc(txn);
                const signedAmountBtc =
                  flow === "outgoing" ? -amountBtc : amountBtc;
                const signedAmountEur =
                  txn.amount === null
                    ? null
                    : flow === "outgoing"
                      ? -txn.amount
                      : txn.amount;
                const primaryAmount =
                  flow === "incoming" || flow === "outgoing"
                    ? formatSignedDisplayMoney(
                        signedAmountEur,
                        signedAmountBtc,
                        currency,
                      )
                    : formatDisplayMoney(txn.amount, amountBtc, currency);
                const FlowIcon =
                  flow === "incoming"
                    ? ArrowDownRight
                    : flow === "outgoing"
                      ? ArrowUpRight
                      : ArrowLeftRight;
                const amountTone =
                  flow === "incoming"
                    ? "text-emerald-700 dark:text-emerald-300"
                    : flow === "outgoing"
                      ? "text-red-700 dark:text-red-300"
                      : "text-muted-foreground";
                return (
                  <tr
                    key={txn.id}
                    ref={rowVirtualizer.measureElement}
                    data-index={virtualRow.index}
                    className="cursor-pointer border-b align-top transition-colors hover:bg-muted/35"
                    onClick={() => openTransactionDetail(txn)}
                  >
                    <TableCell className="overflow-hidden whitespace-normal">
                      <div className="flex min-w-0 items-start gap-3">
                        <span
                          className={cn(
                            "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
                            transactionFlowStyles[flow],
                          )}
                          aria-hidden="true"
                        >
                          <FlowIcon className="size-4" />
                        </span>
                        <div className="min-w-0 flex-1 overflow-hidden">
                          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                            <span
                              className={cn(
                                "truncate text-sm font-medium text-foreground",
                                blurClass(hideSensitive),
                              )}
                            >
                              {txn.counterparty}
                            </span>
                            {showPrimaryLabel ? (
                              <Badge variant="secondary" className="rounded-md">
                                {classificationOptionLabelKeys[draft.label]
                                  ? // loose translator
                                    (t as (key: string) => string)(
                                      classificationOptionLabelKeys[draft.label],
                                    )
                                  : draft.label}
                              </Badge>
                            ) : null}
                            {collateralRole ? (
                              <Badge
                                variant="outline"
                                className={cn(
                                  "gap-1 rounded-md",
                                  loanRoleBadgeClassName(collateralRole),
                                )}
                              >
                                <Coins className="size-3" aria-hidden="true" />
                                {(t as (key: string) => string)(
                                  loanRoleBadgeLabelKey(collateralRole),
                                )}
                              </Badge>
                            ) : null}
                          </div>
                          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
                            <span
                              className={cn("truncate", blurClass(hideSensitive))}
                            >
                              {txn.wallet || txn.paymentMethod}
                            </span>
                            <span aria-hidden="true">·</span>
                            <span>{txn.date}</span>
                            <span aria-hidden="true">·</span>
                            <span
                              className={cn(
                                "truncate font-mono",
                                blurClass(hideSensitive),
                              )}
                            >
                              {formatShortTxid(txn.txnId)}
                            </span>
                          </div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="overflow-hidden text-right">
                      <CurrencyToggleText
                        className={cn(
                          "text-sm font-semibold tabular-nums",
                          amountTone,
                          blurClass(hideSensitive),
                        )}
                      >
                        {primaryAmount}
                      </CurrencyToggleText>
                      <div
                        className={cn(
                          "mt-1 text-[10px] text-muted-foreground tabular-nums sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {formatCounterDisplayMoney(
                          txn.amount,
                          amountBtc,
                          currency,
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="hidden overflow-hidden whitespace-normal md:table-cell">
                      <div className="flex min-w-0 max-w-full flex-wrap gap-1 overflow-hidden">
                        {tagPreview.slice(0, 2).map((tag) => (
                          <Badge
                            key={tag}
                            variant="outline"
                            className={cn(
                              "max-w-full rounded-md truncate",
                              blurClass(hideSensitive),
                            )}
                          >
                            {tag}
                          </Badge>
                        ))}
                        {tagPreview.length > 2 && (
                          <Badge variant="outline" className="rounded-md">
                            +{tagPreview.length - 2}
                          </Badge>
                        )}
                      </div>
                      <p className="mt-1 truncate text-[10px] text-muted-foreground sm:text-xs">
                        {collateralRole
                          ? (t as (key: string) => string)(
                              loanRoleAccountingLabelKey(collateralRole),
                            )
                          : // loose translator
                            (t as (key: string) => string)(
                              rowTaxClassification.shortLabel,
                            )}
                      </p>
                    </TableCell>
                    <TableCell className="hidden overflow-hidden whitespace-normal lg:table-cell">
                      <div className="flex min-w-0 items-center gap-1.5 overflow-hidden">
                        <span
                          className={cn(
                            "inline-flex max-w-full items-center truncate rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                            pricingSourceStyles[rowPricingValue],
                          )}
                        >
                          {/* loose translator */}
                          {(t as (key: string) => string)(
                            pricingSourceLabel(
                              draft.pricingSourceKind,
                              draft.pricingQuality,
                            ),
                          )}
                        </span>
                        {draft.pricingQuality === "coarse_fallback" ? (
                          <span
                            className="inline-flex max-w-full items-center gap-1 truncate rounded-md bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-400"
                            title={t("table.coarse.title")}
                          >
                            <AlertTriangle className="size-3" aria-hidden="true" />
                            {t("table.coarse.badge")}
                          </span>
                        ) : null}
                      </div>
                      <p
                        className={cn(
                          "mt-1 truncate text-[10px] text-muted-foreground sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {draft.pricingSourceKind === "manual_override"
                          ? `${draft.manualCurrency} ${draft.manualValue || t("table.value.valuePending")}`
                          : txn.rate
                            ? t("table.value.perBtc", {
                                value: currencyFormatter.format(txn.rate),
                              })
                            : t("table.value.awaitingPrice")}
                      </p>
                      {rowPricingSummary ? (
                        <p
                          className="truncate text-[10px] text-muted-foreground/80"
                          title={rowPricingSummary}
                        >
                          {rowPricingSummary}
                        </p>
                      ) : null}
                    </TableCell>
                    <TableCell className="hidden overflow-hidden whitespace-normal xl:table-cell">
                      <div className="flex min-w-0 flex-wrap gap-1 overflow-hidden">
                        <span className="inline-flex max-w-full items-center truncate rounded-md border px-2 py-0.5 text-[10px] font-normal text-muted-foreground sm:text-xs">
                          {networkLabel}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="overflow-hidden">
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          transactionStatusStyles[draft.reviewStatus],
                        )}
                      >
                        <StatusIcon className="size-3" aria-hidden="true" />
                        {/* loose translator */}
                        {(t as (key: string) => string)(
                          transactionStatusLabels[draft.reviewStatus],
                        )}
                      </span>
                      <p className="mt-1 hidden text-[10px] text-muted-foreground sm:block sm:text-xs">
                        {collateralRole
                          ? t("table.row.loanNonEvent")
                          : draft.excluded
                          ? t("table.row.excluded")
                          : draft.taxable
                            ? t("table.row.taxable")
                            : t("table.row.notTaxable")}
                      </p>
                    </TableCell>
                    <TableCell className="overflow-hidden">
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-7 text-muted-foreground hover:text-foreground sm:size-8"
                            aria-label={t("table.row.actionsAria", {
                              txid: txn.txnId,
                            })}
                            onClick={(event) => event.stopPropagation()}
                          >
                            <MoreHorizontal className="size-3.5 sm:size-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onSelect={() => openTransactionDetail(txn)}>
                            <Eye className="mr-2 size-4" aria-hidden="true" />
                            {t("table.row.viewDetails")}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => openTransactionDetail(txn, "classify")}
                          >
                            <Pencil
                              className="mr-2 size-4"
                              aria-hidden="true"
                            />
                            {t("table.row.classify")}
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => copyText(txn.explorerId ?? txn.txnId)}
                          >
                            <Copy className="mr-2 size-4" aria-hidden="true" />
                            {t("table.row.copyId")}
                          </DropdownMenuItem>
                          {collateralRole ? (
                            <>
                              <DropdownMenuSeparator />
                              {flow === "outgoing" &&
                              collateralRole !== "loan_principal_repaid" ? (
                                <DropdownMenuItem
                                  onSelect={() => {
                                    void markTransactionCollateral(
                                      txn,
                                      "principal-repaid",
                                    );
                                  }}
                                >
                                  <Coins
                                    className="mr-2 size-4"
                                    aria-hidden="true"
                                  />
                                  {t(
                                    "table.row.collateral.changePrincipalRepaid",
                                  )}
                                </DropdownMenuItem>
                              ) : null}
                              {flow === "incoming" &&
                              collateralRole !== "loan_principal_received" ? (
                                <DropdownMenuItem
                                  onSelect={() => {
                                    void markTransactionCollateral(
                                      txn,
                                      "principal-received",
                                    );
                                  }}
                                >
                                  <Coins
                                    className="mr-2 size-4"
                                    aria-hidden="true"
                                  />
                                  {t(
                                    "table.row.collateral.changePrincipalReceived",
                                  )}
                                </DropdownMenuItem>
                              ) : null}
                              <DropdownMenuItem
                                onSelect={() => {
                                  void unmarkTransactionCollateral(txn);
                                }}
                              >
                                <Link2Off
                                  className="mr-2 size-4"
                                  aria-hidden="true"
                                />
                                {t("table.row.collateral.unmark")}
                              </DropdownMenuItem>
                            </>
                          ) : flow === "outgoing" ? (
                            <>
                              <DropdownMenuSeparator />
                              <DropdownMenuItem
                                onSelect={() => {
                                  void markTransactionCollateral(
                                    txn,
                                    "principal-repaid",
                                  );
                                }}
                              >
                                <Coins
                                  className="mr-2 size-4"
                                  aria-hidden="true"
                                />
                                {t("table.row.collateral.markPrincipalRepaid")}
                              </DropdownMenuItem>
                              <DropdownMenuItem
                                onSelect={() => {
                                  void markTransactionCollateral(
                                    txn,
                                    "collateral",
                                  );
                                }}
                              >
                                <Coins
                                  className="mr-2 size-4"
                                  aria-hidden="true"
                                />
                                {t("table.row.collateral.markCollateral")}
                              </DropdownMenuItem>
                            </>
                          ) : flow === "incoming" ? (
                            <>
                              <DropdownMenuSeparator />
                              <DropdownMenuItem
                                onSelect={() => {
                                  void markTransactionCollateral(
                                    txn,
                                    "principal-received",
                                  );
                                }}
                              >
                                <Coins
                                  className="mr-2 size-4"
                                  aria-hidden="true"
                                />
                                {t("table.row.collateral.markPrincipalReceived")}
                              </DropdownMenuItem>
                              <DropdownMenuItem
                                onSelect={() => {
                                  void markTransactionCollateral(txn, "returned");
                                }}
                              >
                                <Coins
                                  className="mr-2 size-4"
                                  aria-hidden="true"
                                />
                                {t("table.row.collateral.markReturned")}
                              </DropdownMenuItem>
                            </>
                          ) : null}
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive"
                            onSelect={(event: Event) => {
                              event.preventDefault();
                              if (typeof window === "undefined") return;
                              window.confirm(t("table.row.voidConfirm"));
                            }}
                          >
                            <X className="mr-2 size-4" aria-hidden="true" />
                            {t("table.row.exclude")}
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </tr>
                );
                })}
                {paddingBottom > 0 ? (
                  <TableRow aria-hidden="true">
                    <TableCell
                      colSpan={7}
                      style={{ height: paddingBottom, padding: 0 }}
                    />
                  </TableRow>
                ) : null}
              </>
            )}
          </TableBody>
        </table>
      </div>

      <div className="grid items-center gap-3 border-t px-3 py-3 text-xs text-muted-foreground sm:grid-cols-[1fr_auto_1fr] sm:px-6 sm:text-sm">
        <div className="flex flex-col gap-1">
          <span>
            {isRefreshing
              ? t("table.virtual.refreshing")
              : isCompleteRecordSet
                ? t("table.virtual.complete", { count: loadedRecordCount })
                : t("table.virtual.loaded", { count: loadedRecordCount })}
          </span>
          {!isRefreshing && !isCompleteRecordSet ? (
            <span className="text-[10px] sm:text-xs">
              {isLoadingMoreRecords
                ? t("table.virtual.loadingMore")
                : t("table.virtual.moreHint")}
            </span>
          ) : null}
        </div>

        <div className="flex justify-center">
          {hasMoreRecords && onLoadMoreRecords ? (
            <Button
              variant="outline"
              size="sm"
              className="h-8"
              onClick={onLoadMoreRecords}
              disabled={isLoadingMoreRecords}
            >
              {isLoadingMoreRecords
                ? t("table.virtual.loadingMore")
                : t("table.virtual.loadMore")}
            </Button>
          ) : null}
        </div>
        <div aria-hidden="true" />
      </div>
      </div>
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailSheet
        transaction={detailTransaction}
        draft={detailTransaction ? getDraft(detailTransaction) : null}
        initialTab={detailInitialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        isSaving={metadataUpdate.isPending}
        saveError={saveError}
        nowRate={nowRate}
        attachments={detailTransaction ? attachmentItems : undefined}
        journalEvents={journalEvents}
        commercialContext={commercialContext}
        commercialContextLoading={commercialContextQuery.isLoading}
        historyEvents={historyData?.events}
        historyStale={historyData?.stale}
        historyLoading={historyQuery.isLoading}
        isRevertingHistory={revertHistory.isPending}
        onRevertHistory={revertHistoryTarget}
        onProcessJournals={runJournalProcessing}
        isProcessingJournals={isProcessingJournals}
        loanRole={
          detailTransaction
            ? collateralRoleByTransaction.get(detailTransaction.id) ?? null
            : null
        }
        loanMark={detailLoanMark}
        linkedLoanMarks={detailLinkedLoanMarks}
        loanLinkCandidates={detailLoanLinkCandidates}
        isLoanMarking={markCollateral.isPending || unmarkCollateral.isPending}
        isLoanLinking={linkLoanMarks.isPending}
        onMarkLoan={markTransactionCollateral}
        onUnmarkLoan={unmarkTransactionCollateral}
        onLinkLoan={linkTransactionLoan}
        onAddAttachmentFiles={async (paths) => {
          if (!detailTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const path of paths) {
            const result = await attachmentAdd.mutateAsync({
              transaction: detailTransaction.id,
              file_path: path,
            });
            if (result.data) {
              added.push(result.data);
            }
          }
          if (added.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, added),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: t("notification.filesAttached.title"),
            body: t("notification.filesAttached.body", { count: paths.length }),
            tone: "success",
            dedupeKey: `attachments-files-${detailTransaction.id}`,
          });
        }}
        onAddAttachmentLinks={async (urls) => {
          if (!detailTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const url of urls) {
            const result = await attachmentAdd.mutateAsync({
              transaction: detailTransaction.id,
              url,
            });
            if (result.data) {
              added.push(result.data);
            }
          }
          if (added.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, added),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: t("notification.linksAttached.title"),
            body: t("notification.linksAttached.body", { count: urls.length }),
            tone: "success",
            dedupeKey: `attachments-links-${detailTransaction.id}`,
          });
        }}
        onReuseEvidence={
          evidenceSourceTransactions.length
            ? () => {
                setReuseDialogOpen(true);
              }
            : undefined
        }
        onOpenAttachment={async (item) => {
          const result = await attachmentOpen.mutateAsync({
            attachment: item.id,
          });
          const data = result.data;
          if (!data) return;
          if (data.target_type === "url" && data.url) {
            await openExternalUrl(data.url);
            return;
          }
          if (data.target_type === "file" && data.path) {
            await openAttachmentFile(data.path);
          }
        }}
        onRenameAttachment={async (item, label) => {
          if (!detailTransaction) return;
          const result = await attachmentRename.mutateAsync({
            attachment: item.id,
            label,
          });
          const updated = result.data;
          if (updated) {
            updateDetailAttachmentRecords((attachments) =>
              replaceAttachmentRecord(attachments, updated),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => replaceAttachmentRecord(attachments, updated),
            );
          }
          useUiStore.getState().addNotification({
            title: t("notification.linkTextUpdated.title"),
            body: t("notification.linkTextUpdated.body"),
            tone: "success",
          });
        }}
        onRemoveAttachment={async (item) => {
          if (!detailTransaction) return;
          await attachmentRemove.mutateAsync({ attachment: item.id });
          updateDetailAttachmentRecords((attachments) =>
            removeAttachmentRecord(attachments, item.id),
          );
          updateAttachmentListQueryCache(
            detailTransaction.id,
            (attachments) => removeAttachmentRecord(attachments, item.id),
          );
          useUiStore.getState().addNotification({
            title: t("notification.attachmentRemoved.title"),
            body:
              item.kind === "file"
                ? t("notification.attachmentRemoved.fileBody")
                : t("notification.attachmentRemoved.linkBody"),
            tone: "success",
            dedupeKey: `attachment-remove-${item.id}`,
          });
        }}
        onUnpair={async (pairId) => {
          await unpairTransfer.mutateAsync({ pair_id: pairId });
          setDetailTransaction((current) =>
            current?.pair?.id === pairId ? { ...current, pair: undefined } : current,
          );
          useUiStore.getState().addNotification({
            title: t("notification.pairRemoved.title"),
            body: t("notification.pairRemoved.body"),
            tone: "success",
            dedupeKey: `transfer-unpair-${pairId}`,
          });
        }}
        isUnpairing={unpairTransfer.isPending}
        onOpenPairingReview={() => {
          setDetailTransaction(null);
          updateTransactionDetailParams(null);
          void navigate({ to: "/swaps" });
        }}
        onOpenMarketDataSettings={() => {
          setDetailTransaction(null);
          updateTransactionDetailParams(null);
          void navigate({ to: "/settings", hash: "market" });
        }}
        hasNext={
          detailTransaction
            ? filteredTransactions.findIndex(
                (txn) => txn.id === detailTransaction.id,
              ) <
              filteredTransactions.length - 1
            : false
        }
        onOpenChange={(open) => {
          if (!open) {
            setDetailTransaction(null);
            setReuseDialogOpen(false);
            setSaveError(null);
            updateTransactionDetailParams(null);
          }
        }}
        onOpenExplorer={(transaction) => setExplorerTransaction(transaction)}
        onSave={async (transactionId, draft) => {
          try {
            await saveTransactionDraft(transactionId, draft);
          } catch (error) {
            setSaveError(
              error instanceof Error
                ? error.message
                : t("save.couldNotSaveMetadata"),
            );
            throw error;
          }
        }}
        onSaveAndNext={async (transactionId, draft) => {
          try {
            await saveTransactionDraft(transactionId, draft);
            const idx = filteredTransactions.findIndex(
              (txn) => txn.id === transactionId,
            );
            const next = filteredTransactions[idx + 1];
            if (next) {
              openTransactionDetail(next, detailInitialTab);
            } else {
              setDetailTransaction(null);
              updateTransactionDetailParams(null);
            }
          } catch (error) {
            setSaveError(
              error instanceof Error
                ? error.message
                : t("save.couldNotSaveMetadata"),
            );
            throw error;
          }
        }}
      />
      <TransactionEvidenceReuseDialog
        open={reuseDialogOpen}
        onOpenChange={setReuseDialogOpen}
        targetTransaction={detailTransaction}
        sourceTransactions={evidenceSourceTransactions}
        sourceTransactionId={reuseSourceTransactionId}
        onSourceTransactionIdChange={setReuseSourceTransactionId}
        sourceAttachments={reuseSourceAttachmentItems}
        isLoadingSourceAttachments={reuseSourceAttachmentsQuery.isLoading}
        isCopying={attachmentCopy.isPending}
        hideSensitive={hideSensitive}
        onCopy={async (attachmentIds) => {
          if (!detailTransaction || !reuseSourceTransactionId) return;
          const result = await attachmentCopy.mutateAsync({
            transaction: detailTransaction.id,
            source_transaction: reuseSourceTransactionId,
            attachments: attachmentIds,
          });
          const copied = result.data?.copied ?? attachmentIds.length;
          const copiedAttachments = result.data?.attachments ?? [];
          if (copiedAttachments.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, copiedAttachments),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) =>
                upsertAttachmentRecords(attachments, copiedAttachments),
            );
          }
          setReuseDialogOpen(false);
          useUiStore.getState().addNotification({
            title: t("notification.evidenceReused.title"),
            body: t("notification.evidenceReused.body", { count: copied }),
            tone: "success",
            dedupeKey: `attachments-copy-${detailTransaction.id}`,
          });
        }}
      />
    </>
  );
};

export { TransactionsTable };
