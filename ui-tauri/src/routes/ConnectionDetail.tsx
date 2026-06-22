/**
 * Connection detail page.
 *
 * The detail surface uses the shared shadcn component vocabulary from the
 * Connections and Overview screens.
 */

import { useEffect, useRef, useState, type FormEvent } from "react";
import { Trans, useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeft,
  ArrowLeftRight,
  ArrowRight,
  ArrowUpRight,
  AlertTriangle,
  CircleDollarSign,
  Database,
  ListChecks,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Scale,
  ShieldCheck,
  Trash2,
  Wallet,
} from "lucide-react";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { ConnectionStatusPill } from "@/components/kb/ConnectionStatusPill";
import { CountBadge } from "@/components/kb/CountBadge";
import { DetailRow } from "@/components/kb/DetailRow";
import { MetricCard } from "@/components/kb/MetricCard";
import {
  UtxosInventoryPanel,
  WalletBalanceHistoryCard,
  type WalletUtxosData,
} from "@/components/kb/wallets";
import { useOverviewTransactionDetail } from "@/components/overview-dashboard/useOverviewTransactionDetail";
import { NodeConnectionDetail } from "./NodeConnectionDetail";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  daemonMutationKey,
  useDaemon,
  useDaemonMutation,
  useDaemonStreamMutation,
  retryRetryableDaemonError,
} from "@/daemon/client";
import {
  connectionKindLabels,
  connectionKindTone,
} from "@/lib/connectionDisplay";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { formatShortDate } from "@/lib/date";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { editConfigKindForConnection } from "@/lib/connectionEditKind";
import { describeWalletSyncResult, type SyncResult } from "@/lib/syncResults";
import { transactionBelongsToConnection } from "@/lib/connectionTransactions";
import { buildBalanceReconciliation } from "@/lib/walletBalanceReconcile";
import { MISSING_FIAT_LABEL } from "@/lib/currency";
import {
  startingSyncProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import { detectWalletMaterial } from "@/lib/walletMaterialFormat";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "@/hooks/useSyncProgressNotice";
import { useConnectionRefreshState } from "@/hooks/useConnectionRefreshState";
import type {
  Connection,
  ConnectionKind,
  NodeSnapshot,
  OverviewSnapshot,
} from "@/mocks/seed";

// Lightning *node* kinds — sync against a daemon that exposes channels,
// peers, and routing snapshots. Phoenix is also categorised as "Lightning"
// in connectionDisplay, but it lives in Kassiber as a CSV-import wallet
// (`wallets import-phoenix`, setupKind: "file-wallet"), not a node-shaped
// sync target — so it keeps rendering with the wallet detail layout below.
const NODE_CONNECTION_KINDS: ReadonlySet<ConnectionKind> = new Set([
  "lnd",
  "core-ln",
  "nwc",
]);

const isNodeConnection = (kind: ConnectionKind) =>
  NODE_CONNECTION_KINDS.has(kind);

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");
const MAX_DESCRIPTOR_GAP_LIMIT = 5000;

const fmtBtc = (value: number) => `₿ ${value.toFixed(8)}`;
const fmtEur = (value: number | null) =>
  value === null
    ? MISSING_FIAT_LABEL
    : "€ " +
      value.toLocaleString("de-AT", {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });
const fmtSatSigned = (amountSat: number) =>
  `${amountSat >= 0 ? "+ " : "- "}${Math.abs(amountSat).toLocaleString(
    "en-US",
  )}`;
const fmtEurSigned = (amountEur: number | null) =>
  amountEur === null
    ? MISSING_FIAT_LABEL
    : `${amountEur >= 0 ? "+ " : "- "}${fmtEur(Math.abs(amountEur))}`;
const fmtShortTxid = (value?: string) =>
  !value ? "no id" : value.length <= 18 ? value : `${value.slice(0, 10)}…${value.slice(-6)}`;

const relatedViewLinkClass =
  "group flex items-center gap-3 px-4 py-3 transition-colors hover:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";
const relatedViewIconClass =
  "flex size-8 shrink-0 items-center justify-center rounded-md border bg-muted/40 text-muted-foreground transition-colors group-hover:border-foreground/20 group-hover:text-foreground";
const relatedViewArrowClass =
  "size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5";

const PLAINTEXT_CHANGE_ACK = "CHANGE LOCAL DATA";
const PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA";
const CLEAR_BACKEND_SELECTION = "__kassiber_clear_backend__";

interface UpdateWalletResult {
  wallet: {
    id: string;
    label: string;
  };
}

interface DeleteWalletResult {
  wallet: {
    id: string;
    label: string;
    deleted: boolean;
    cascaded_transactions: number;
  };
}

type WalletListItem = {
  id?: string;
  label: string;
  kind?: string;
  account?: {
    code?: string;
    label?: string;
  };
  backend?: {
    name?: string;
    source?: string;
    kind?: string;
  };
  chain?: string;
  network?: string;
  sync_mode?: string;
  sync_source?: string;
  transaction_count?: number;
  last_transaction_at?: string | null;
  last_synced_at?: string | null;
  sync_status?: string;
  created_at?: string;
  btcpay_provenance?: Array<{
    backend: string;
    store_id: string;
    payment_method_id: string;
  }>;
  samourai?: SamouraiWalletMetadata | null;
};

type BackendOption = {
  name: string;
  display_name?: string;
  kind: string;
  chain?: string;
  network?: string;
  is_default?: boolean;
};

interface SamouraiWalletMetadata {
  role?: string;
  group_id?: string;
  group_label?: string;
  source?: string;
  section?: string;
  script_type?: string;
  root_path?: string;
  gap_limit?: number;
  privacy_boundary?: boolean;
  whirlpool?: boolean;
  toxic_change?: boolean;
  minimum_mix_count?: number;
  mix_count?: number;
  mix_count_confidence?: string;
  target_mix_count?: number;
  pool_denomination_sat?: number;
  watch_only?: boolean;
  bip47?: string;
  paynym?: boolean;
  scanned_without_explicit_descriptor?: boolean;
  sections?: string[];
}

const syncModeLabelKeys: Record<string, string> = {
  backend_descriptor: "detail.syncMode.backendDescriptor",
  backend_addresses: "detail.syncMode.backendAddresses",
  file_import: "detail.syncMode.fileImport",
  btcpay: "detail.syncMode.btcpay",
  not_configured: "detail.syncMode.notConfigured",
};

function syncModeLabel(value: string, t: TFunction<"connections">) {
  const key = syncModeLabelKeys[value];
  // dynamic key
  return key ? t(key as never) : undefined;
}

function formatBackendDetail(
  backend: WalletListItem["backend"] | undefined,
  t: TFunction<"connections">,
) {
  if (!backend?.name) return t("detail.backendDetail.notConfigured");
  const kind = backend.kind ? ` · ${backend.kind}` : "";
  const source =
    backend.source && backend.source !== "none" ? ` (${backend.source})` : "";
  return `${backend.name}${kind}${source}`;
}

function samouraiSectionLabel(
  value: string | undefined,
  t: TFunction<"connections">,
) {
  const keys: Record<string, string> = {
    deposit: "detail.samourai.sectionLabel.deposit",
    badbank: "detail.samourai.sectionLabel.badbank",
    premix: "detail.samourai.sectionLabel.premix",
    postmix: "detail.samourai.sectionLabel.postmix",
    ricochet: "detail.samourai.sectionLabel.ricochet",
  };
  const key = keys[value ?? ""];
  // dynamic key
  return key ? t(key as never) : value ?? t("detail.samourai.groupFallback");
}

function samouraiSourceLabel(
  value: string | undefined,
  t: TFunction<"connections">,
) {
  if (value === "source_set") return t("detail.samourai.sourceLabel.sourceSet");
  return value ?? t("detail.samourai.sourceLabel.imported");
}

function backendOptionLabel(backend: { name: string; display_name?: string }) {
  const label = backend.display_name?.trim() || backend.name;
  return label === backend.name ? label : `${label} (${backend.name})`;
}

function backendOptionChain(backend: Pick<BackendOption, "name" | "chain">) {
  const chain = backend.chain?.trim().toLowerCase();
  if (chain) return chain;
  const name = backend.name.trim().toLowerCase();
  return name === "liquid" || name === "liquid-blockstream"
    ? "liquid"
    : "bitcoin";
}

function isWalletLiveBackendSource(
  connection: Pick<Connection, "kind" | "syncMode">,
  wallet?: Pick<WalletListItem, "sync_mode">,
) {
  const syncMode = wallet?.sync_mode || connection.syncMode || "";
  return (
    syncMode === "backend_descriptor" ||
    syncMode === "backend_addresses" ||
    connection.kind === "descriptor" ||
    connection.kind === "xpub" ||
    connection.kind === "address" ||
    connection.kind === "samourai"
  );
}

export function ConnectionDetail() {
  const { t } = useTranslation("connections");
  const { connectionId } = useParams({ from: "/_app/connections/$connectionId" });
  const { data, isLoading } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
  );
  const hideSensitive = useUiStore((state) => state.hideSensitive);

  if (isLoading || !data?.data) {
    return <ScreenSkeleton titleWidth="w-44" />;
  }

  const snapshot = data.data;
  const connection = snapshot.connections.find((item) => item.id === connectionId);

  if (!connection) {
    return (
      <div className="flex flex-1 flex-col items-center justify-center gap-4 p-12 text-muted-foreground">
        <p className="text-sm">
          {t("detail.notFound", { id: connectionId })}
        </p>
        <Button asChild variant="outline" size="sm">
          <Link to="/connections">
            <ArrowLeft className="size-4" aria-hidden="true" />
            {t("detail.backToWallets")}
          </Link>
        </Button>
      </div>
    );
  }

  if (isNodeConnection(connection.kind)) {
    // Node detail intentionally diverges from the wallet detail: it
    // does NOT expose the per-connection edit/remove dropdown menu.
    // The wallet edit dialog edits descriptor / source_file / btcpay
    // store / gap_limit fields that have no node analogue — a node's
    // alias, pubkey, and channel set are reported by the node itself,
    // not user-editable. Connection-level remove still flows through
    // the Connections list (where the bulk edit/remove path lives);
    // wiring a node-only Remove dialog here is tracked as follow-up.
    return (
      <NodeConnectionContainer
        connection={connection}
        priceEur={snapshot.priceEur}
        hideSensitive={hideSensitive}
      />
    );
  }

  return (
    <ConnectionDetailView
      connection={connection}
      snapshot={snapshot}
      priceEur={snapshot.priceEur}
      txs={snapshot.txs}
      hideSensitive={hideSensitive}
    />
  );
}

interface NodeConnectionContainerProps {
  connection: Connection;
  priceEur: number;
  hideSensitive: boolean;
}

function NodeConnectionContainer({
  connection,
  priceEur,
  hideSensitive,
}: NodeConnectionContainerProps) {
  const { t } = useTranslation("connections");
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const syncNoticeIdRef = useRef<string | null>(null);
  const progressValueRef = useRef(startingSyncProgress().value ?? 5);
  const nodeSnapshotQuery = useDaemon<NodeSnapshot>(
    "ui.connections.node.snapshot",
    { connection: connection.id },
    { retry: retryRetryableDaemonError },
  );
  const liveSnapshot = nodeSnapshotQuery.data?.data;
  const resolvedConnection = liveSnapshot
    ? { ...connection, node: liveSnapshot }
    : connection;
  const walletSyncMutationKey = daemonMutationKey(dataMode, "ui.wallets.sync");
  const connectionRefreshing = useConnectionRefreshState(connection);
  // TODO: switch to ui.connections.node.sync (or similar) once #154/#155 land
  // a real node-sync kind. ui.wallets.sync is a mock-only stop-gap — the
  // Python daemon won't execute it for lnd/core-ln/nwc kinds yet.
  const syncWallet = useDaemonStreamMutation<
    { results: SyncResult[] },
    WalletSyncProgress
  >("ui.wallets.sync", {
    onProgress: (record) => {
      if (!syncNoticeIdRef.current) return;
      const wallet = record.wallet ?? connection.label;
      const nextProgress = syncProgressNotification(
        { ...record, wallet },
        progressValueRef.current,
      );
      progressValueRef.current = nextProgress.value;
      updateNotification(syncNoticeIdRef.current, {
        body: nextProgress.body,
        progress: nextProgress.progress,
      });
    },
  });
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const nodeSyncDedupeKey = `node-sync-${connection.id}`;

  const isSyncRunning = syncWallet.isPending || connectionRefreshing;

  const onSync = () => {
    if (
      syncWallet.isPending ||
      queryClient.isMutating({ mutationKey: walletSyncMutationKey }) > 0
    ) {
      addNotification({
        title: t("node.sync.alreadyRunningTitle"),
        body: t("node.sync.alreadyRunningBody", { label: connection.label }),
        tone: "info",
        dedupeKey: nodeSyncDedupeKey,
      });
      return;
    }
    progressValueRef.current = startingSyncProgress().value ?? 5;
    syncNoticeIdRef.current = addNotification({
      title: t("node.sync.startedTitle"),
      body: t("node.sync.startedBody", { label: connection.label }),
      tone: "warning",
      dedupeKey: nodeSyncDedupeKey,
      progress: startingSyncProgress(),
    });
    startSyncNotice(
      t("node.sync.stillScanning", { label: connection.label }),
    );
    syncWallet.mutate(
      { wallet: connection.label },
      {
        onSuccess: (envelope) => {
          const result = envelope.data?.results?.find(
            (item) => item.wallet === connection.label,
          );
          const status = result?.status ?? "synced";
          const message = describeWalletSyncResult(result, connection.label);
          const notification = {
            title:
              status === "error"
                ? t("node.sync.failedTitle")
                : t("node.sync.finishedTitle"),
            body: message,
            tone: status === "error" ? "error" : "success",
            dedupeKey: nodeSyncDedupeKey,
            progress: undefined,
          } as const;
          if (syncNoticeIdRef.current) {
            updateNotification(syncNoticeIdRef.current, notification);
          } else {
            addNotification(notification);
          }
        },
        onError: (error) => {
          const message =
            error instanceof Error ? error.message : t("node.sync.failedFallback");
          const notification = {
            title: t("node.sync.failedTitle"),
            body: message,
            tone: "error",
            dedupeKey: nodeSyncDedupeKey,
            progress: undefined,
          } as const;
          if (syncNoticeIdRef.current) {
            updateNotification(syncNoticeIdRef.current, notification);
          } else {
            addNotification(notification);
          }
        },
        onSettled: () => {
          clearSyncNotice();
          syncNoticeIdRef.current = null;
        },
      },
    );
  };

  return (
    <NodeConnectionDetail
      connection={resolvedConnection}
      priceEur={priceEur}
      hideSensitive={hideSensitive}
      onSync={onSync}
      isSyncRunning={isSyncRunning}
    />
  );
}

interface ConnectionDetailViewProps {
  connection: Connection;
  snapshot: OverviewSnapshot;
  priceEur: number;
  txs: OverviewSnapshot["txs"];
  hideSensitive: boolean;
}

function ConnectionDetailView({
  connection,
  snapshot,
  priceEur,
  txs,
  hideSensitive,
}: ConnectionDetailViewProps) {
  const { t } = useTranslation(["connections", "common"]);
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const identity = useUiStore((state) => state.identity);
  const explorerSettings = useUiStore((state) => state.explorerSettings);
  const currency = useUiStore((state) => state.currency);
  const [pendingUtxoTransactionId, setPendingUtxoTransactionId] =
    useState<string | null>(null);
  const syncNoticeIdRef = useRef<string | null>(null);
  const walletSyncMutationKey = daemonMutationKey(dataMode, "ui.wallets.sync");
  const connectionRefreshing = useConnectionRefreshState(connection);
  const progressValueRef = useRef(startingSyncProgress().value ?? 5);
  const syncWallet = useDaemonStreamMutation<
    { results: SyncResult[] },
    WalletSyncProgress
  >("ui.wallets.sync", {
    onProgress: (record) => {
      const wallet = record.wallet ?? connection.label;
      if (syncNoticeIdRef.current) {
        const nextProgress = syncProgressNotification(
          { ...record, wallet },
          progressValueRef.current,
        );
        progressValueRef.current = nextProgress.value;
        updateNotification(syncNoticeIdRef.current, {
          body: nextProgress.body,
          progress: nextProgress.progress,
        });
      }
    },
  });
  const updateWallet =
    useDaemonMutation<UpdateWalletResult>("ui.wallets.update");
  const deleteWallet =
    useDaemonMutation<DeleteWalletResult>("ui.wallets.delete");
  const backendOptionsQuery = useDaemon<{
    backends: BackendOption[];
  }>("ui.backends.options");
  const walletsListQuery = useDaemon<{
    wallets: WalletListItem[];
  }>("ui.wallets.list");
  const walletDetail = walletsListQuery.data?.data?.wallets?.find(
    (wallet) =>
      (wallet.id && wallet.id === connection.id) ||
      wallet.label === connection.label,
  );
  const coinsInventoryQuery = useDaemon<WalletUtxosData>(
    "ui.wallets.utxos",
    { wallet: connection.id },
    { retry: retryRetryableDaemonError },
  );
  const utxoTransactionQuery = useDaemon<{
    transaction?: OverviewSnapshot["txs"][number] | null;
  }>(
    "ui.transactions.resolve",
    { query: pendingUtxoTransactionId ?? "" },
    { enabled: Boolean(pendingUtxoTransactionId), retry: retryRetryableDaemonError },
  );
  const resolvedUtxoTransaction =
    utxoTransactionQuery.data?.data?.transaction ?? null;
  const { detailSheet, openTransactionDetail } = useOverviewTransactionDetail({
    snapshot,
    extraTransactions: resolvedUtxoTransaction
      ? [resolvedUtxoTransaction]
      : [],
    hideSensitive,
    currency,
    explorerSettings,
  });
  const openUtxoTransaction = (transactionId: string) => {
    setPendingUtxoTransactionId(transactionId);
  };
  const walletProvenanceRoutes = walletDetail?.btcpay_provenance ?? [];
  const samouraiMetadata = walletDetail?.samourai ?? null;
  const inventory = coinsInventoryQuery.data?.data;
  const samouraiInventory = inventory;
  // Balance reconciliation: the "Balance" tile sums imported transactions,
  // while the UTXO inventory is the watch-only on-chain coin set. For a
  // fully-synced wallet they should match; a gap usually means a stale sync
  // or excluded/quarantined rows. Only sources that expose a UTXO inventory
  // can be reconciled (cashu and other unsupported sources cannot).
  const reconciliation = buildBalanceReconciliation(
    connection.balance,
    inventory,
  );
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const [syncErrorMessage, setSyncErrorMessage] = useState<string | null>(null);
  const [editOpen, setEditOpen] = useState(false);
  const [editLabel, setEditLabel] = useState(connection.label);
  const [editPassphrase, setEditPassphrase] = useState("");
  const [editPlaintextAck, setEditPlaintextAck] = useState("");
  const [editWalletMaterial, setEditWalletMaterial] = useState("");
  const [editGapLimit, setEditGapLimit] = useState("");
  const [editStoreId, setEditStoreId] = useState("");
  const [editPaymentMethodId, setEditPaymentMethodId] = useState("");
  const [editBackend, setEditBackend] = useState("");
  const [editSourceFile, setEditSourceFile] = useState("");
  const [editClearProvenance, setEditClearProvenance] = useState(false);
  const [editError, setEditError] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deletePassphrase, setDeletePassphrase] = useState("");
  const [deletePlaintextAck, setDeletePlaintextAck] = useState("");
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const sourceValue =
    walletDetail?.sync_source ||
    connection.syncSource ||
    connection.sourceFormat ||
    connectionKindLabels[connection.kind];
  const sourceDetail =
    connection.syncMode === "live"
      ? t("detail.source.live")
      : connection.syncMode === "file"
        ? t("detail.source.file")
        : t("detail.source.wallet");
  const hasGapMetric = connection.gap != null;
  const connectionTxs = txs.filter((tx) =>
    transactionBelongsToConnection(tx, connection),
  );
  const txsForConnection = connectionTxs.slice(0, 6);

  useEffect(() => {
    if (!pendingUtxoTransactionId || utxoTransactionQuery.isLoading) return;
    if (resolvedUtxoTransaction) {
      openTransactionDetail(resolvedUtxoTransaction.id);
      setPendingUtxoTransactionId(null);
      return;
    }
    if (utxoTransactionQuery.error) {
      addNotification({
        title: t("detail.utxoTransaction.notOpenedTitle"),
        body:
          utxoTransactionQuery.error instanceof Error
            ? utxoTransactionQuery.error.message
            : t("detail.utxoTransaction.couldNotResolve"),
        tone: "warning",
        dedupeKey: `utxo-transaction-open-${pendingUtxoTransactionId}`,
      });
      setPendingUtxoTransactionId(null);
      return;
    }
    if (utxoTransactionQuery.data) {
      addNotification({
        title: t("detail.utxoTransaction.notFoundTitle"),
        body: t("detail.utxoTransaction.notFoundBody"),
        tone: "warning",
        dedupeKey: `utxo-transaction-missing-${pendingUtxoTransactionId}`,
      });
      setPendingUtxoTransactionId(null);
    }
  }, [
    addNotification,
    openTransactionDetail,
    pendingUtxoTransactionId,
    resolvedUtxoTransaction,
    utxoTransactionQuery.data,
    utxoTransactionQuery.error,
    utxoTransactionQuery.isLoading,
    t,
  ]);
  const txCount = connection.transactionCount ?? connectionTxs.length;
  const isWalletSyncRunning = syncWallet.isPending || connectionRefreshing;
  const refreshButtonLabel = isWalletSyncRunning
    ? t("detail.refreshing")
    : t("detail.refresh");

  const onSync = (options?: { forceFull?: boolean }) => {
    if (
      syncWallet.isPending ||
      queryClient.isMutating({ mutationKey: walletSyncMutationKey }) > 0
    ) {
      addNotification({
        title: t("detail.sync.alreadyRunningTitle"),
        body: t("detail.sync.alreadyRunningBody", { label: connection.label }),
        tone: "info",
        dedupeKey: "wallet-sync",
      });
      return;
    }
    setSyncErrorMessage(null);
    progressValueRef.current = startingSyncProgress().value ?? 5;
    syncNoticeIdRef.current = addNotification({
      title: options?.forceFull
        ? t("detail.sync.rescanStartedTitle")
        : t("detail.sync.refreshStartedTitle"),
      body: options?.forceFull
        ? t("detail.sync.rescanStartedBody", { label: connection.label })
        : t("detail.sync.refreshStartedBody", { label: connection.label }),
      tone: "warning",
      dedupeKey: "wallet-sync",
      progress: startingSyncProgress(),
    });
    startSyncNotice(
      t("detail.sync.stillScanning", { label: connection.label }),
    );
    syncWallet.mutate(
      { wallet: connection.label, force_full: Boolean(options?.forceFull) },
      {
        onSuccess: (envelope) => {
          const result = envelope.data?.results?.find(
            (item) => item.wallet === connection.label,
          );
          const status = result?.status ?? "synced";
          const message = describeWalletSyncResult(result, connection.label);
          if (status === "error") {
            setSyncErrorMessage(message);
          }
          const notification = {
            title:
              status === "error"
                ? t("detail.sync.failedTitle")
                : t("detail.sync.finishedTitle"),
            body: message,
            tone: status === "error" ? "error" : "success",
            dedupeKey: "wallet-sync",
            progress: undefined,
          } as const;
          if (syncNoticeIdRef.current) {
            updateNotification(syncNoticeIdRef.current, notification);
          } else {
            addNotification(notification);
          }
        },
        onError: (error) => {
          const message =
            error instanceof Error ? error.message : t("detail.sync.failedFallback");
          setSyncErrorMessage(message);
          const notification = {
            title: t("detail.sync.failedTitle"),
            body: message,
            tone: "error",
            dedupeKey: "wallet-sync",
            progress: undefined,
          } as const;
          if (syncNoticeIdRef.current) {
            updateNotification(syncNoticeIdRef.current, notification);
          } else {
            addNotification(notification);
          }
        },
        onSettled: () => {
          clearSyncNotice();
          syncNoticeIdRef.current = null;
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  };

  const openEditDialog = () => {
    setEditLabel(connection.label);
    setEditPassphrase("");
    setEditPlaintextAck("");
    setEditWalletMaterial("");
    setEditGapLimit(connection.gap != null ? String(connection.gap) : "");
    setEditStoreId("");
    setEditPaymentMethodId("");
    setEditBackend("");
    setEditSourceFile("");
    setEditClearProvenance(false);
    setEditError(null);
    setEditOpen(true);
  };

  const allBackendOptions = backendOptionsQuery.data?.data?.backends ?? [];
  const btcpayBackendOptions = allBackendOptions.filter(
    (backend) => backend.kind === "btcpay",
  );
  const walletChain = (walletDetail?.chain || "bitcoin").toLowerCase();
  const canEditLiveBackend = isWalletLiveBackendSource(connection, walletDetail);
  const liveBackendOptions = allBackendOptions.filter((backend) => {
    const kind = backend.kind.trim().toLowerCase();
    if (kind === "btcpay" || kind === "coreln" || kind === "lnd") return false;
    return backendOptionChain(backend) === walletChain;
  });
  const canClearLiveBackend =
    canEditLiveBackend &&
    walletChain !== "liquid" &&
    walletDetail?.backend?.source === "explicit";

  const editConfigKind = editConfigKindForConnection(connection);

  const openDeleteDialog = () => {
    setDeletePassphrase("");
    setDeletePlaintextAck("");
    setDeleteConfirm("");
    setDeleteError(null);
    setDeleteOpen(true);
  };

  const verifyLocalPassphrase = async (
    passphrase: string,
    setError: (message: string) => void,
  ) => {
    if (!encryptedWorkspace) return true;
    if (!passphrase) {
      setError(t("detail.auth.enterPassphrase"));
      return false;
    }
    return true;
  };

  const onEditSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setEditError(null);
    const nextLabel = editLabel.trim();
    if (!nextLabel) {
      setEditError(t("detail.edit.errorEnterLabel"));
      return;
    }
    if (!(await verifyLocalPassphrase(editPassphrase, setEditError))) return;
    if (!encryptedWorkspace && editPlaintextAck.trim() !== PLAINTEXT_CHANGE_ACK) {
      setEditError(
        t("detail.edit.errorPlaintextAck", { ack: PLAINTEXT_CHANGE_ACK }),
      );
      return;
    }
    const labelChanged = nextLabel !== connection.label;
    const walletMaterial = editWalletMaterial.trim();
    const gapLimitText = editGapLimit.trim();
    const storeId = editStoreId.trim();
    const paymentMethodId = editPaymentMethodId.trim();
    const backend = editBackend.trim();
    const sourceFile = editSourceFile.trim();
    const configChanges: Record<string, unknown> = {};
    const clearFields: string[] = [];
    if (editConfigKind === "descriptor" && walletMaterial) {
      const detection = detectWalletMaterial(walletMaterial);
      if (detection.kind === "bare-xpub" || detection.kind === "unknown") {
        setEditError(detection.hint ?? detection.label);
        return;
      }
      configChanges.wallet_material = walletMaterial;
    }
    if (editConfigKind === "descriptor" && gapLimitText) {
      const gapLimit = Number.parseInt(gapLimitText, 10);
      if (!Number.isFinite(gapLimit) || gapLimit <= 0) {
        setEditError(t("detail.edit.errorGapPositive"));
        return;
      }
      if (gapLimit > MAX_DESCRIPTOR_GAP_LIMIT) {
        setEditError(
          t("detail.edit.errorGapMax", {
            max: MAX_DESCRIPTOR_GAP_LIMIT.toLocaleString(),
          }),
        );
        return;
      }
      if (connection.gap == null || gapLimit !== connection.gap) {
        configChanges.gap_limit = gapLimit;
      }
    }
    if (editConfigKind === "btcpay") {
      if (storeId) configChanges.store_id = storeId;
      if (paymentMethodId) configChanges.payment_method_id = paymentMethodId;
      if (backend) configChanges.backend = backend;
    }
    if (canEditLiveBackend && editConfigKind !== "btcpay") {
      if (backend === CLEAR_BACKEND_SELECTION) {
        clearFields.push("backend");
      } else if (
        backend &&
        (backend !== walletDetail?.backend?.name ||
          walletDetail?.backend?.source !== "explicit")
      ) {
        configChanges.backend = backend;
      }
    }
    if (editConfigKind === "file-wallet" && sourceFile) {
      configChanges.source_file = sourceFile;
    }
    if (editClearProvenance && walletProvenanceRoutes.length > 0) {
      clearFields.push("btcpay_provenance");
    }
    if (
      !labelChanged &&
      Object.keys(configChanges).length === 0 &&
      clearFields.length === 0
    ) {
      setEditError(t("detail.edit.errorChangeNothing"));
      return;
    }

    try {
      await updateWallet.mutateAsync({
        wallet: connection.id,
        ...(labelChanged ? { label: nextLabel } : {}),
        ...configChanges,
        ...(clearFields.length > 0 ? { clear: clearFields } : {}),
        auth_response: encryptedWorkspace
          ? { passphrase_secret: editPassphrase }
          : { plaintext_change_ack: PLAINTEXT_CHANGE_ACK },
      });
      const summary = labelChanged
        ? t("detail.edit.renamed", { from: connection.label, to: nextLabel })
        : t("detail.edit.updated", { label: connection.label });
      addNotification({
        title: t("detail.edit.changedTitle"),
        body: summary,
        tone: "success",
      });
      setEditOpen(false);
    } catch (error) {
      setEditError(
        error instanceof Error ? error.message : t("detail.edit.couldNotChange"),
      );
    }
  };

  const onDeleteSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setDeleteError(null);
    if (deleteConfirm.trim() !== connection.label) {
      setDeleteError(
        t("detail.delete.errorConfirmLabel", { label: connection.label }),
      );
      return;
    }
    if (!(await verifyLocalPassphrase(deletePassphrase, setDeleteError))) return;
    if (
      !encryptedWorkspace &&
      deletePlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setDeleteError(
        t("detail.delete.errorPlaintextAck", { ack: PLAINTEXT_DELETE_ACK }),
      );
      return;
    }

    try {
      await deleteWallet.mutateAsync({
        wallet: connection.id,
        confirm: "DELETE",
        confirm_wallet: connection.label,
        cascade: true,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: deletePassphrase }
          : { plaintext_delete_ack: PLAINTEXT_DELETE_ACK },
      });
      addNotification({
        title: t("detail.delete.removedTitle"),
        body: t("detail.delete.removedBody", { label: connection.label }),
        tone: "success",
      });
      setDeleteOpen(false);
      void navigate({ to: "/connections", replace: true });
    } catch (error) {
      setDeleteError(
        error instanceof Error ? error.message : t("detail.delete.couldNotRemove"),
      );
    }
  };

  return (
    <div className={screenShellClassName}>
      <Card className="rounded-xl py-3">
        <CardContent className="flex flex-col gap-3 px-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <Button asChild variant="outline" size="icon" className="shrink-0">
              <Link to="/connections" aria-label={t("detail.backToWallets")}>
                <ArrowLeft className="size-4" aria-hidden="true" />
              </Link>
            </Button>
            <span
              className={cn(
                "hidden size-9 shrink-0 items-center justify-center rounded-md border sm:flex",
                connectionKindTone(connection.kind),
              )}
              aria-hidden="true"
            >
              <Wallet className="size-4" />
            </span>
            <div className="min-w-0">
              <h1 className="truncate text-xl font-semibold tracking-tight sm:text-2xl">
                {connection.label}
              </h1>
              <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span className="truncate">
                  {connectionKindLabels[connection.kind]}
                </span>
                {connection.status !== "synced" ? (
                  <>
                    <span aria-hidden="true">·</span>
                    <ConnectionStatusPill status={connection.status} />
                  </>
                ) : null}
              </div>
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-2 self-start sm:self-center">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isWalletSyncRunning}
              aria-busy={isWalletSyncRunning}
              aria-label={t("detail.refreshAction", {
                action: refreshButtonLabel,
                label: connection.label,
              })}
              onClick={() => onSync()}
            >
              <RefreshCw
                className={cn("size-4", isWalletSyncRunning && "animate-spin")}
                aria-hidden="true"
              />
              {/*
                Reserve the width of the widest label so the button never
                resizes when it flips to the in-progress text. A mid-refresh
                width change strands a stale composited tile of the spinning
                icon in WKWebView (the macOS webview), leaving a frozen "ghost"
                copy of the button overlaid on the live, spinning one. The
                shift — and the ghost — only showed in locales where the two
                labels differ in width (e.g. de "Aktualisieren" →
                "Wird aktualisiert"); English clamped to the same width and
                stayed clean. Stacking both labels pins the width to the
                wider one in every locale.
              */}
              <span className="grid justify-items-center">
                <span
                  aria-hidden="true"
                  className="invisible col-start-1 row-start-1"
                >
                  {t("detail.refresh")}
                </span>
                <span
                  aria-hidden="true"
                  className="invisible col-start-1 row-start-1"
                >
                  {t("detail.refreshing")}
                </span>
                <span className="col-start-1 row-start-1">
                  {refreshButtonLabel}
                </span>
              </span>
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label={t("detail.moreActions")}
                >
                  <MoreHorizontal className="size-4" aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem onClick={openEditDialog}>
                  <Pencil className="size-4" aria-hidden="true" />
                  {t("common:actions.edit")}
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={isWalletSyncRunning}
                  onClick={() => onSync({ forceFull: true })}
                >
                  <RotateCcw className="size-4" aria-hidden="true" />
                  {t("detail.fullRescan")}
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={openDeleteDialog}
                >
                  <Trash2 className="size-4" aria-hidden="true" />
                  {t("common:actions.remove")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </CardContent>
        {syncErrorMessage && (
          <div className="px-4 pt-3">
            <div
              className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300"
              role="status"
            >
              {syncErrorMessage}
            </div>
          </div>
        )}
      </Card>

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          label={t("detail.metric.balance")}
          value={
            <span className={blurClass(hideSensitive)}>
              {fmtBtc(connection.balance)}
            </span>
          }
          detail={fmtEur(connection.balance * priceEur)}
          icon={<Wallet className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={t("detail.metric.transactions")}
          value={txCount.toLocaleString("en-US")}
          detail={t("detail.metric.transactionsDetail")}
          icon={<ArrowLeftRight className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={t("detail.metric.lastSync")}
          value={connection.last}
          detail={connection.status}
          icon={<RefreshCw className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={hasGapMetric ? t("detail.metric.gapLimit") : t("detail.metric.source")}
          value={
            hasGapMetric ? connection.gap?.toLocaleString("en-US") ?? "—" : sourceValue
          }
          detail={hasGapMetric ? t("detail.metric.gapLimitDetail") : sourceDetail}
          icon={<Database className="size-4" aria-hidden="true" />}
        />
      </div>

      {reconciliation.available && !reconciliation.reconciled ? (
        // Only surface reconciliation when it needs attention: the two figures
        // genuinely differ. When the books match the chain we stay quiet (the
        // Balance metric already reflects it) rather than adding a standing
        // "all good" confirmation. The comparison + the
        // signed delta is the actionable signal.
        <Card className="rounded-xl border-amber-300 py-3 dark:border-amber-900/60">
          <CardContent className="flex items-center justify-between gap-4 px-4">
            <div className="flex items-center gap-4">
              <span
                className="flex size-9 shrink-0 items-center justify-center rounded-md border bg-muted/40 text-muted-foreground"
                aria-hidden="true"
              >
                <Scale className="size-4" />
              </span>
              <div className="flex items-center gap-4">
                <div>
                  <div className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("detail.reconcile.onChainInventory")}
                  </div>
                  <div
                    className={cn(
                      "font-mono text-base font-semibold tabular-nums",
                      blurClass(hideSensitive),
                    )}
                  >
                    {fmtBtc(reconciliation.utxoSat / 1e8)}
                  </div>
                  <div className="text-[10px] text-muted-foreground">
                    {t("detail.reconcile.watchOnlyTruth")}
                  </div>
                </div>
                <span
                  className="self-start pt-4 font-mono text-base font-semibold text-amber-600 dark:text-amber-400"
                  aria-hidden="true"
                >
                  ≠
                </span>
                <div>
                  <div className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
                    {t("detail.reconcile.recordedInBooks")}
                  </div>
                  <div
                    className={cn(
                      "font-mono text-base font-semibold tabular-nums",
                      blurClass(hideSensitive),
                    )}
                  >
                    {fmtBtc(reconciliation.importedSat / 1e8)}
                  </div>
                  <div className="text-[10px] text-muted-foreground">
                    {t("detail.reconcile.feedsTaxReports")}
                  </div>
                </div>
              </div>
            </div>
            <div className="text-right">
              <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-1 text-xs font-medium text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:text-amber-300">
                <AlertTriangle className="size-3.5" aria-hidden="true" />
                {reconciliation.deltaSat < 0
                  ? t("detail.reconcile.onChainNotInBooks", {
                      amount: fmtBtc(Math.abs(reconciliation.deltaSat) / 1e8),
                    })
                  : t("detail.reconcile.inBooksNotOnChain", {
                      amount: fmtBtc(Math.abs(reconciliation.deltaSat) / 1e8),
                    })}
              </span>
              <div className="mt-1 text-xs text-muted-foreground">
                {reconciliation.deltaSat < 0
                  ? t("detail.reconcile.inboundExcluded")
                  : t("detail.reconcile.manualOrDuplicate")}
              </div>
            </div>
          </CardContent>
        </Card>
      ) : null}

      <UtxosInventoryPanel
        inventory={coinsInventoryQuery.data?.data}
        isLoading={coinsInventoryQuery.isLoading}
        errorMessage={
          coinsInventoryQuery.error instanceof Error
            ? coinsInventoryQuery.error.message
            : null
        }
        hideSensitive={hideSensitive}
        isRefreshing={isWalletSyncRunning}
        explorerSettings={explorerSettings}
        onRefresh={onSync}
        onOpenTransaction={openUtxoTransaction}
      />

      {samouraiMetadata ? (
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="flex flex-wrap items-center gap-2 text-sm sm:text-base">
              {t("detail.samourai.title")}
              <Badge variant="secondary">
                {samouraiMetadata.role === "parent"
                  ? t("detail.samourai.group")
                  : samouraiSectionLabel(samouraiMetadata.section, t)}
              </Badge>
              {samouraiMetadata.privacy_boundary ? (
                <Badge variant="outline">
                  {t("detail.samourai.privacyBoundary")}
                </Badge>
              ) : null}
              {samouraiMetadata.paynym ? (
                <Badge variant="outline">BIP47</Badge>
              ) : null}
            </CardTitle>
            <CardDescription>
              {t("detail.samourai.description")}
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-3 px-4 pt-4 md:grid-cols-2">
            <DetailRow
              label={t("detail.samourai.source")}
              value={samouraiSourceLabel(samouraiMetadata.source, t)}
            />
            <DetailRow
              label={t("detail.samourai.group")}
              value={samouraiMetadata.group_label || connection.label}
            />
            {samouraiMetadata.role !== "parent" ? (
              <>
                <DetailRow
                  label={t("detail.samourai.section")}
                  value={samouraiSectionLabel(samouraiMetadata.section, t)}
                />
                <DetailRow
                  label={t("detail.samourai.script")}
                  value={samouraiMetadata.script_type || "—"}
                />
                <DetailRow
                  label={t("detail.samourai.root")}
                  value={samouraiMetadata.root_path || "—"}
                  mono
                />
                <DetailRow
                  label={t("detail.samourai.gap")}
                  value={
                    samouraiMetadata.gap_limit?.toLocaleString("en-US") ??
                    connection.gap?.toLocaleString("en-US") ??
                    "—"
                  }
                />
              </>
            ) : (
              <DetailRow
                label={t("detail.samourai.sections")}
                value={(samouraiMetadata.sections ?? [])
                  .map((section) => samouraiSectionLabel(section, t))
                  .join(", ")}
              />
            )}
            {samouraiMetadata.minimum_mix_count ? (
              <DetailRow
                label={t("detail.samourai.minimumMixCount")}
                value={t("detail.samourai.minimumMixCountValue", {
                  value:
                    samouraiMetadata.minimum_mix_count.toLocaleString("en-US"),
                  confidence:
                    samouraiMetadata.mix_count_confidence ??
                    t("detail.samourai.minimumConfidence"),
                })}
              />
            ) : null}
            {samouraiMetadata.mix_count ? (
              <DetailRow
                label={t("detail.samourai.observedMixes")}
                value={t("detail.samourai.observedMixesValue", {
                  value: samouraiMetadata.mix_count.toLocaleString("en-US"),
                  confidence:
                    samouraiMetadata.mix_count_confidence ??
                    t("detail.samourai.importedConfidence"),
                })}
              />
            ) : null}
            {samouraiMetadata.target_mix_count ? (
              <DetailRow
                label={t("detail.samourai.targetMixes")}
                value={samouraiMetadata.target_mix_count.toLocaleString("en-US")}
              />
            ) : null}
            {samouraiMetadata.pool_denomination_sat ? (
              <DetailRow
                label={t("detail.samourai.pool")}
                value={t("detail.samourai.poolValue", {
                  value:
                    samouraiMetadata.pool_denomination_sat.toLocaleString(
                      "en-US",
                    ),
                })}
              />
            ) : null}
            {samouraiMetadata.role !== "parent" ? (
              <DetailRow
                label={t("detail.samourai.coins")}
                value={
                  samouraiInventory?.freshness.active_count?.toLocaleString(
                    "en-US",
                  ) ??
                  samouraiInventory?.summary?.count?.toLocaleString("en-US") ??
                  "—"
                }
              />
            ) : null}
          </CardContent>
          <div className="space-y-2 border-t px-4 py-3 text-sm">
            {samouraiMetadata.toxic_change ? (
              <div className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
                {t("detail.samourai.toxicChange")}
              </div>
            ) : null}
            {samouraiMetadata.section === "postmix" ? (
              <div className="rounded-md border bg-background px-3 py-2 text-muted-foreground">
                {t("detail.samourai.postmixNote")}
              </div>
            ) : null}
            {samouraiInventory?.freshness.stale ? (
              <div className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
                {t("detail.samourai.stale")}
              </div>
            ) : null}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isWalletSyncRunning}
              onClick={() => onSync()}
            >
              <RefreshCw
                className={cn("size-4", isWalletSyncRunning && "animate-spin")}
                aria-hidden="true"
              />
              {/* Reserve the widest label's width so the spinning icon can't
                  ghost on a mid-refresh resize — see the header Refresh button. */}
              <span className="grid justify-items-center">
                <span
                  aria-hidden="true"
                  className="invisible col-start-1 row-start-1"
                >
                  {t("detail.samourai.refreshSource")}
                </span>
                <span
                  aria-hidden="true"
                  className="invisible col-start-1 row-start-1"
                >
                  {t("detail.refreshing")}
                </span>
                <span className="col-start-1 row-start-1">
                  {isWalletSyncRunning
                    ? t("detail.refreshing")
                    : t("detail.samourai.refreshSource")}
                </span>
              </span>
            </Button>
          </div>
        </Card>
      ) : null}

      {walletProvenanceRoutes.length > 0 ? (
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="text-sm sm:text-base">
              {t("detail.btcpayProvenance.title")}
            </CardTitle>
            <CardDescription>
              {t("detail.btcpayProvenance.description")}
            </CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("detail.btcpayProvenance.instance")}</TableHead>
                  <TableHead>{t("detail.btcpayProvenance.store")}</TableHead>
                  <TableHead>
                    {t("detail.btcpayProvenance.paymentMethod")}
                  </TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {walletProvenanceRoutes.map((route, index) => (
                  <TableRow
                    key={`${route.backend}-${route.store_id}-${route.payment_method_id}-${index}`}
                  >
                    <TableCell>{route.backend}</TableCell>
                    <TableCell>{route.store_id}</TableCell>
                    <TableCell>{route.payment_method_id}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
          <div className="border-t px-4 py-2.5 text-xs text-muted-foreground">
            <Trans
              t={t}
              i18nKey="detail.btcpayProvenance.footer"
              components={{
                strong: <strong />,
                em: <em />,
                code: <code />,
              }}
            />
          </div>
        </Card>
      ) : null}

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.85fr)]">
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
                  {t("detail.recentTransactions.title")}
                  <CountBadge>{txCount.toLocaleString("en-US")}</CountBadge>
                </CardTitle>
                <CardDescription>
                  {connectionTxs.length > txsForConnection.length
                    ? t("detail.recentTransactions.showingRecent", {
                        count: txsForConnection.length,
                      })
                    : t("detail.recentTransactions.recent")}
                </CardDescription>
              </div>
              {connectionTxs.length > 0 ? (
                <Button
                  asChild
                  type="button"
                  variant="outline"
                  size="sm"
                  className="shrink-0"
                >
                  <Link
                    to="/transactions"
                    search={{ wallet: connection.label }}
                    hash="transactions-table"
                  >
                    {t("detail.recentTransactions.showAll")}
                    <ArrowRight className="size-4" aria-hidden="true" />
                  </Link>
                </Button>
              ) : null}
            </div>
          </CardHeader>
          <CardContent className="p-0">
            {txsForConnection.length ? (
              <div className="divide-y">
                {txsForConnection.map((tx) => (
                  <ConnectionTransactionRow
                    key={tx.id}
                    tx={tx}
                    hideSensitive={hideSensitive}
                  />
                ))}
              </div>
            ) : (
              <div className="flex flex-col items-start gap-3 px-5 py-8 text-sm text-muted-foreground">
                <p>{t("detail.recentTransactions.empty")}</p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={isWalletSyncRunning}
                    onClick={() => onSync()}
                  >
                    <RefreshCw
                      className={cn(
                        "size-4",
                        isWalletSyncRunning && "animate-spin",
                      )}
                      aria-hidden="true"
                    />
                    {/* Reserve the widest label's width so the spinning icon
                        can't ghost on a mid-refresh resize — see the header
                        Refresh button. */}
                    <span className="grid justify-items-center">
                      <span
                        aria-hidden="true"
                        className="invisible col-start-1 row-start-1"
                      >
                        {t("detail.recentTransactions.refreshNow")}
                      </span>
                      <span
                        aria-hidden="true"
                        className="invisible col-start-1 row-start-1"
                      >
                        {t("detail.refreshing")}
                      </span>
                      <span className="col-start-1 row-start-1">
                        {isWalletSyncRunning
                          ? t("detail.refreshing")
                          : t("detail.recentTransactions.refreshNow")}
                      </span>
                    </span>
                  </Button>
                  <Button asChild type="button" variant="outline" size="sm">
                    <Link to="/imports">
                      <Plus className="size-4" aria-hidden="true" />
                      {t("detail.recentTransactions.importFile")}
                    </Link>
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>

        <div className="space-y-3">
          <WalletBalanceHistoryCard
            walletId={connection.id}
            hideSensitive={hideSensitive}
          />
          <Card>
            <CardHeader className="border-b px-4 pb-3">
              <CardTitle className="text-sm sm:text-base">
                {t("detail.connectionDetails.title")}
              </CardTitle>
              <CardDescription>
                {t("detail.connectionDetails.description")}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3 px-4 pt-4">
              <DetailRow
                label={t("detail.connectionDetails.syncMode")}
                value={
                  syncModeLabel(
                    walletDetail?.sync_mode || connection.syncMode || "",
                    t,
                  ) ??
                  walletDetail?.sync_mode ??
                  connection.syncMode ??
                  "—"
                }
              />
              <DetailRow
                label={t("detail.connectionDetails.backend")}
                value={formatBackendDetail(walletDetail?.backend, t)}
              />
              {walletDetail?.account?.label || walletDetail?.account?.code ? (
                <DetailRow
                  label={t("detail.connectionDetails.account")}
                  value={[
                    walletDetail.account.code,
                    walletDetail.account.label,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                />
              ) : null}
              {walletDetail?.chain || walletDetail?.network ? (
                <DetailRow
                  label={t("detail.connectionDetails.network")}
                  value={[walletDetail.chain, walletDetail.network]
                    .filter(Boolean)
                    .join(" · ")}
                />
              ) : null}
              <DetailRow
                label={t("detail.connectionDetails.created")}
                value={formatShortDate(walletDetail?.created_at)}
                mono
              />
              <DetailRow
                label={t("detail.connectionDetails.kassiberId")}
                value={connection.id}
                mono
                copy
              />
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="border-b px-4 pb-3">
              <CardTitle className="text-sm sm:text-base">
                {t("detail.relatedViews.title")}
              </CardTitle>
              <CardDescription>
                {t("detail.relatedViews.description")}
              </CardDescription>
            </CardHeader>
            <CardContent className="p-0">
              <div className="divide-y">
                <Link
                  to="/transactions"
                  search={{ wallet: connection.label }}
                  hash="transactions-table"
                  className={relatedViewLinkClass}
                >
                  <span className={relatedViewIconClass} aria-hidden="true">
                    <ArrowLeftRight className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {t("detail.relatedViews.allTransactions")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t("detail.relatedViews.allTransactionsDetail", {
                        value: txCount.toLocaleString("en-US"),
                      })}
                    </span>
                  </span>
                  <ArrowRight className={relatedViewArrowClass} aria-hidden="true" />
                </Link>
                <Link
                  to="/transactions"
                  search={{ wallet: connection.label, quick: "review_queue" }}
                  hash="transactions-table"
                  className={relatedViewLinkClass}
                >
                  <span className={relatedViewIconClass} aria-hidden="true">
                    <ListChecks className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {t("detail.relatedViews.needsReview")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t("detail.relatedViews.needsReviewDetail")}
                    </span>
                  </span>
                  <ArrowRight className={relatedViewArrowClass} aria-hidden="true" />
                </Link>
                <Link
                  to="/transactions"
                  search={{ wallet: connection.label, quick: "missing_price" }}
                  hash="transactions-table"
                  className={relatedViewLinkClass}
                >
                  <span className={relatedViewIconClass} aria-hidden="true">
                    <CircleDollarSign className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {t("detail.relatedViews.missingPrice")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t("detail.relatedViews.missingPriceDetail")}
                    </span>
                  </span>
                  <ArrowRight className={relatedViewArrowClass} aria-hidden="true" />
                </Link>
                <Link to="/source-of-funds" className={relatedViewLinkClass}>
                  <span className={relatedViewIconClass} aria-hidden="true">
                    <ShieldCheck className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {t("detail.relatedViews.sourceOfFunds")}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {t("detail.relatedViews.sourceOfFundsDetail")}
                    </span>
                  </span>
                  <ArrowRight className={relatedViewArrowClass} aria-hidden="true" />
                </Link>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("detail.edit.title")}</DialogTitle>
            <DialogDescription>
              {encryptedWorkspace
                ? t("detail.edit.descriptionEncrypted")
                : t("detail.edit.descriptionPlaintext")}
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={onEditSubmit}>
            <div className="space-y-2">
              <Label htmlFor="connection-label">{t("detail.edit.label")}</Label>
              <Input
                id="connection-label"
                value={editLabel}
                onChange={(event) => setEditLabel(event.target.value)}
              />
            </div>
            {canEditLiveBackend && editConfigKind !== "btcpay" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-backend">
                  {t("detail.edit.syncBackend")}
                </Label>
                <select
                  id="connection-edit-backend"
                  className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                  value={editBackend}
                  onChange={(event) => setEditBackend(event.target.value)}
                >
                  <option value="">
                    {walletDetail?.backend?.name
                      ? t("detail.edit.keepCurrentBackendNamed", {
                          name: walletDetail.backend.name,
                        })
                      : t("detail.edit.keepCurrentBackend")}
                  </option>
                  {canClearLiveBackend ? (
                    <option value={CLEAR_BACKEND_SELECTION}>
                      {t("detail.edit.useDefaultBitcoinBackend")}
                    </option>
                  ) : null}
                  {liveBackendOptions.map((backend) => (
                    <option key={backend.name} value={backend.name}>
                      {backendOptionLabel(backend)}
                      {backend.network ? ` · ${backend.network}` : ""}
                      {backend.is_default
                        ? t("detail.edit.backendOptionDefault")
                        : ""}
                    </option>
                  ))}
                </select>
                <p className="text-xs text-muted-foreground">
                  {walletChain === "liquid"
                    ? t("detail.edit.liquidBackendHelper")
                    : t("detail.edit.backendHelper")}
                </p>
              </div>
            ) : null}
            {editConfigKind === "descriptor" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-material">
                  {t("detail.edit.descriptorOrXpub")}
                </Label>
                <Textarea
                  id="connection-edit-material"
                  className="min-h-24 font-mono text-xs"
                  value={editWalletMaterial}
                  onChange={(event) =>
                    setEditWalletMaterial(event.target.value)
                  }
                  placeholder={t("detail.edit.materialPlaceholder")}
                />
                {editWalletMaterial.trim()
                  ? (() => {
                      const detection = detectWalletMaterial(editWalletMaterial);
                      const tone =
                        detection.kind === "bare-xpub" ||
                        detection.kind === "unknown"
                          ? "text-amber-700 dark:text-amber-300"
                          : "text-emerald-700 dark:text-emerald-300";
                      return (
                        <p className={cn("text-xs", tone)}>
                          {detection.hint
                            ? t("detail.edit.detectedWithHint", {
                                label: detection.label,
                                hint: detection.hint,
                              })
                            : t("detail.edit.detected", {
                                label: detection.label,
                              })}
                        </p>
                      );
                    })()
                  : (
                      <p className="text-xs text-muted-foreground">
                        {t("detail.edit.materialHelper")}
                      </p>
                    )}
              </div>
            ) : null}
            {editConfigKind === "descriptor" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-gap-limit">
                  {t("detail.edit.gapLimit")}
                </Label>
                <Input
                  id="connection-edit-gap-limit"
                  type="number"
                  min={1}
                  max={MAX_DESCRIPTOR_GAP_LIMIT}
                  value={editGapLimit}
                  onChange={(event) => setEditGapLimit(event.target.value)}
                  placeholder="40"
                />
                <p className="text-xs text-muted-foreground">
                  {t("detail.edit.gapLimitHelper")}
                </p>
              </div>
            ) : null}
            {editConfigKind === "btcpay" ? (
              <>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-backend">
                    {t("detail.edit.btcpayInstance")}
                  </Label>
                  <select
                    id="connection-edit-backend"
                    className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                    value={editBackend}
                    onChange={(event) => setEditBackend(event.target.value)}
                  >
                    <option value="">{t("detail.edit.keepCurrentInstance")}</option>
                    {btcpayBackendOptions.map((backend) => (
                      <option key={backend.name} value={backend.name}>
                        {backendOptionLabel(backend)}
                        {backend.is_default
                          ? t("detail.edit.backendOptionDefault")
                          : ""}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-store">
                    {t("detail.edit.btcpayStoreId")}
                  </Label>
                  <Input
                    id="connection-edit-store"
                    value={editStoreId}
                    onChange={(event) => setEditStoreId(event.target.value)}
                    placeholder={t("detail.edit.storeIdPlaceholder")}
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-payment-method">
                    {t("detail.edit.btcpayWallet")}
                  </Label>
                  <Input
                    id="connection-edit-payment-method"
                    value={editPaymentMethodId}
                    onChange={(event) =>
                      setEditPaymentMethodId(event.target.value)
                    }
                    placeholder={t("detail.edit.paymentMethodPlaceholder")}
                  />
                </div>
              </>
            ) : null}
            {editConfigKind === "file-wallet" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-source">
                  {t("detail.edit.sourceFile")}
                </Label>
                <div className="flex gap-2">
                  <Input
                    id="connection-edit-source"
                    value={editSourceFile}
                    onChange={(event) => setEditSourceFile(event.target.value)}
                    placeholder={t("detail.edit.sourceFilePlaceholder")}
                  />
                  {isFilePickerAvailable ? (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={async () => {
                        const picked = await pickFile({
                          title: t("detail.edit.selectExportFileTitle", {
                            label: connection.label,
                          }),
                        });
                        if (picked) setEditSourceFile(picked);
                      }}
                    >
                      {t("detail.edit.browse")}
                    </Button>
                  ) : null}
                </div>
              </div>
            ) : null}
            {walletProvenanceRoutes.length > 0 ? (
              <div className="space-y-2 rounded-md border border-border/70 p-3">
                <label className="flex items-start gap-3 text-sm">
                  <input
                    type="checkbox"
                    className="mt-0.5"
                    checked={editClearProvenance}
                    onChange={(event) =>
                      setEditClearProvenance(event.target.checked)
                    }
                  />
                  <span className="grid gap-0.5">
                    <span>{t("detail.edit.clearProvenance")}</span>
                    <span className="text-xs text-muted-foreground">
                      {t("detail.edit.clearProvenanceHelper", {
                        count: walletProvenanceRoutes.length,
                      })}
                    </span>
                  </span>
                </label>
              </div>
            ) : null}
            {encryptedWorkspace ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-passphrase">
                  {t("detail.edit.passphrase")}
                </Label>
                <Input
                  id="connection-edit-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={editPassphrase}
                  onChange={(event) => setEditPassphrase(event.target.value)}
                />
              </div>
            ) : (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-ack">
                  {t("detail.edit.plaintextChallenge")}
                </Label>
                <Input
                  id="connection-edit-ack"
                  value={editPlaintextAck}
                  placeholder={PLAINTEXT_CHANGE_ACK}
                  onChange={(event) => setEditPlaintextAck(event.target.value)}
                />
              </div>
            )}
            {editError && (
              <p className="m-0 text-sm text-destructive">{editError}</p>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setEditOpen(false)}
              >
                {t("common:actions.cancel")}
              </Button>
              <Button type="submit" disabled={updateWallet.isPending}>
                {updateWallet.isPending
                  ? t("detail.edit.saving")
                  : t("common:actions.save")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("detail.delete.title")}</DialogTitle>
            <DialogDescription>
              {t("detail.delete.description", {
                label: connection.label,
                challenge: encryptedWorkspace
                  ? t("detail.delete.challengePassphrase")
                  : t("detail.delete.challengePlaintext"),
              })}
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={onDeleteSubmit}>
            {encryptedWorkspace ? (
              <div className="space-y-2">
                <Label htmlFor="connection-delete-passphrase">
                  {t("detail.delete.passphrase")}
                </Label>
                <Input
                  id="connection-delete-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={deletePassphrase}
                  onChange={(event) => setDeletePassphrase(event.target.value)}
                />
              </div>
            ) : (
              <div className="space-y-2">
                <Label htmlFor="connection-delete-ack">
                  {t("detail.delete.plaintextChallenge")}
                </Label>
                <Input
                  id="connection-delete-ack"
                  value={deletePlaintextAck}
                  placeholder={PLAINTEXT_DELETE_ACK}
                  onChange={(event) => setDeletePlaintextAck(event.target.value)}
                />
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="connection-delete-label">
                {t("detail.delete.connectionLabel")}
              </Label>
              <Input
                id="connection-delete-label"
                value={deleteConfirm}
                placeholder={connection.label}
                onChange={(event) => setDeleteConfirm(event.target.value)}
              />
            </div>
            {deleteError && (
              <p className="m-0 text-sm text-destructive">{deleteError}</p>
            )}
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setDeleteOpen(false)}
              >
                {t("common:actions.cancel")}
              </Button>
              <Button
                type="submit"
                variant="destructive"
                disabled={deleteWallet.isPending}
              >
                {deleteWallet.isPending
                  ? t("detail.delete.removing")
                  : t("common:actions.remove")}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
      {detailSheet}
    </div>
  );
}

function ConnectionTransactionRow({
  tx,
  hideSensitive,
}: {
  tx: OverviewSnapshot["txs"][number];
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("connections");
  const flow =
    tx.type === "Swap" || tx.type === "Transfer" || tx.type === "Rebalance"
      ? "transfer"
      : tx.amountSat >= 0
        ? "incoming"
        : "outgoing";
  const Icon =
    flow === "incoming"
      ? ArrowDownRight
      : flow === "outgoing"
        ? ArrowUpRight
        : ArrowLeftRight;
  const tone =
    flow === "incoming"
      ? "border-emerald-600/20 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
      : flow === "outgoing"
        ? "border-red-600/20 bg-red-500/10 text-red-700 dark:text-red-300"
        : "border-sky-600/20 bg-sky-500/10 text-sky-700 dark:text-sky-300";

  return (
    <Link
      to="/transactions"
      search={{ tx: tx.id }}
      className="flex min-w-0 items-start gap-3 px-5 py-3 transition-colors hover:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
    >
      <span
        className={cn(
          "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
          tone,
        )}
        aria-hidden="true"
      >
        <Icon className="size-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 flex-1 truncate text-sm font-medium">
            {tx.counter || tx.type}
          </span>
          <Badge variant="outline" className="rounded-md whitespace-nowrap">
            {tx.type}
          </Badge>
          {tx.conf > 0 ? null : (
            <Badge
              variant="outline"
              className="rounded-md whitespace-nowrap text-amber-700 ring-amber-600/20 dark:text-amber-300"
            >
              {t("detail.recentTransactions.pending")}
            </Badge>
          )}
        </span>
        <span className="mt-1 flex min-w-0 items-center gap-1.5 text-[10px] text-muted-foreground sm:text-xs">
          <span className="shrink-0">{tx.date}</span>
          <span aria-hidden="true">·</span>
          <span className={cn("truncate font-mono", blurClass(hideSensitive))}>
            {fmtShortTxid(tx.externalId ?? tx.id)}
          </span>
        </span>
      </span>
      <span className="shrink-0 text-right">
        <span
          className={cn(
            "block font-mono text-sm font-semibold tabular-nums",
            tx.amountSat > 0
              ? "text-emerald-600 dark:text-emerald-400"
              : tx.amountSat < 0
                ? "text-red-600 dark:text-red-400"
                : "text-muted-foreground",
            blurClass(hideSensitive),
          )}
        >
          {fmtSatSigned(tx.amountSat)}
        </span>
        <span
          className={cn(
            "mt-0.5 block font-mono text-[10px] text-muted-foreground tabular-nums sm:text-xs",
            blurClass(hideSensitive),
          )}
        >
          {fmtEurSigned(tx.eur)}
        </span>
      </span>
    </Link>
  );
}
