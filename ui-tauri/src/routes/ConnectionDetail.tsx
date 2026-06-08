/**
 * Connection detail page.
 *
 * The detail surface uses the shared shadcn component vocabulary from the
 * Connections and Overview screens.
 */

import { useEffect, useRef, useState, type FormEvent } from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeft,
  ArrowLeftRight,
  ArrowUpRight,
  Database,
  MoreHorizontal,
  Pencil,
  Plus,
  RefreshCw,
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
import { MISSING_FIAT_LABEL } from "@/lib/currency";
import {
  startingSyncProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import { detectWalletMaterial } from "@/lib/walletMaterialFormat";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "@/hooks/useSyncProgressNotice";
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

const PLAINTEXT_CHANGE_ACK = "CHANGE LOCAL DATA";
const PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA";

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

const syncModeLabels: Record<string, string> = {
  backend_descriptor: "Watch-only descriptor",
  backend_addresses: "Watch-only addresses",
  file_import: "File import",
  btcpay: "BTCPay enrichment",
  not_configured: "Manual / not configured",
};

function formatBackendDetail(backend?: WalletListItem["backend"]) {
  if (!backend?.name) return "Not configured";
  const kind = backend.kind ? ` · ${backend.kind}` : "";
  const source =
    backend.source && backend.source !== "none" ? ` (${backend.source})` : "";
  return `${backend.name}${kind}${source}`;
}

function samouraiSectionLabel(value?: string) {
  const labels: Record<string, string> = {
    deposit: "Deposit",
    badbank: "Badbank / Toxic Change",
    premix: "Premix",
    postmix: "Postmix",
    ricochet: "Ricochet",
  };
  return labels[value ?? ""] ?? value ?? "Samourai group";
}

function samouraiSourceLabel(value?: string) {
  const labels: Record<string, string> = {
    source_set: "Descriptor/xpub set",
  };
  return labels[value ?? ""] ?? value ?? "Imported";
}

function backendOptionLabel(backend: { name: string; display_name?: string }) {
  const label = backend.display_name?.trim() || backend.name;
  return label === backend.name ? label : `${label} (${backend.name})`;
}

export function ConnectionDetail() {
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
        <p className="text-sm">No connection found for {connectionId}.</p>
        <Button asChild variant="outline" size="sm">
          <Link to="/connections">
            <ArrowLeft className="size-4" aria-hidden="true" />
            Back to wallets
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
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const syncNoticeIdRef = useRef<string | null>(null);
  const progressValueRef = useRef(startingSyncProgress().value ?? 5);
  const nodeSnapshotQuery = useDaemon<NodeSnapshot>(
    "ui.connections.node.snapshot",
    { connection: connection.id },
    { retry: false },
  );
  const liveSnapshot = nodeSnapshotQuery.data?.data;
  const resolvedConnection = liveSnapshot
    ? { ...connection, node: liveSnapshot }
    : connection;
  const walletSyncMutationKey = daemonMutationKey(dataMode, "ui.wallets.sync");
  const walletSyncsInFlight = useIsMutating({
    mutationKey: walletSyncMutationKey,
  });
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

  const isSyncRunning =
    syncWallet.isPending ||
    walletSyncsInFlight > 0 ||
    connection.status === "syncing";

  const onSync = () => {
    if (
      syncWallet.isPending ||
      queryClient.isMutating({ mutationKey: walletSyncMutationKey }) > 0
    ) {
      addNotification({
        title: "Node refresh already running",
        body: `${connection.label} is already scanning. Kassiber will update this page when the daemon finishes.`,
        tone: "info",
        dedupeKey: nodeSyncDedupeKey,
      });
      return;
    }
    progressValueRef.current = startingSyncProgress().value ?? 5;
    syncNoticeIdRef.current = addNotification({
      title: "Node refresh started",
      body: `${connection.label} is fetching channel and routing data in read-only mode.`,
      tone: "warning",
      dedupeKey: nodeSyncDedupeKey,
      progress: startingSyncProgress(),
    });
    startSyncNotice(
      `${connection.label} is still scanning. Large channel histories can take a moment; Kassiber will update when the daemon returns.`,
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
                ? "Node refresh failed"
                : "Node refresh finished",
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
            error instanceof Error ? error.message : "Node refresh failed.";
          const notification = {
            title: "Node refresh failed",
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
  const walletSyncsInFlight = useIsMutating({
    mutationKey: walletSyncMutationKey,
  });
  const [syncProgress, setSyncProgress] = useState<{
    wallet: string;
    processed: number;
    total: number;
  } | null>(null);
  const progressValueRef = useRef(startingSyncProgress().value ?? 5);
  const syncWallet = useDaemonStreamMutation<
    { results: SyncResult[] },
    WalletSyncProgress
  >("ui.wallets.sync", {
    onProgress: (record) => {
      const wallet = record.wallet ?? connection.label;
      setSyncProgress({
        wallet,
        processed: record.processed ?? 0,
        total: record.total ?? 0,
      });
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
    backends: Array<{
      name: string;
      display_name?: string;
      kind: string;
      is_default?: boolean;
    }>;
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
    { retry: false },
  );
  const utxoTransactionQuery = useDaemon<{
    transaction?: OverviewSnapshot["txs"][number] | null;
  }>(
    "ui.transactions.resolve",
    { query: pendingUtxoTransactionId ?? "" },
    { enabled: Boolean(pendingUtxoTransactionId), retry: false },
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
  const samouraiInventory = coinsInventoryQuery.data?.data;
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const [syncMessage, setSyncMessage] = useState<string | null>(null);
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
      ? "Live sync source"
      : connection.syncMode === "file"
        ? "File import source"
        : "Wallet source";
  const hasGapMetric = connection.gap != null;
  const txsForConnection = txs
    .filter((tx) => {
      const account = tx.account.toLowerCase();
      const label = connection.label.toLowerCase();
      return account === label || account.includes(label);
    })
    .slice(0, 6);

  useEffect(() => {
    if (!pendingUtxoTransactionId || utxoTransactionQuery.isLoading) return;
    if (resolvedUtxoTransaction) {
      openTransactionDetail(resolvedUtxoTransaction.id);
      setPendingUtxoTransactionId(null);
      return;
    }
    if (utxoTransactionQuery.error) {
      addNotification({
        title: "Transaction not opened",
        body:
          utxoTransactionQuery.error instanceof Error
            ? utxoTransactionQuery.error.message
            : "Kassiber could not resolve this UTXO transaction.",
        tone: "warning",
        dedupeKey: `utxo-transaction-open-${pendingUtxoTransactionId}`,
      });
      setPendingUtxoTransactionId(null);
      return;
    }
    if (utxoTransactionQuery.data) {
      addNotification({
        title: "Transaction not found",
        body: "This UTXO has no matching imported Kassiber transaction yet.",
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
  ]);
  const txCount = connection.transactionCount ?? txsForConnection.length;
  const isWalletSyncRunning =
    syncWallet.isPending ||
    walletSyncsInFlight > 0 ||
    connection.status === "syncing";
  const refreshButtonLabel = isWalletSyncRunning ? "Refreshing" : "Refresh";

  const onSync = () => {
    if (
      syncWallet.isPending ||
      queryClient.isMutating({ mutationKey: walletSyncMutationKey }) > 0
    ) {
      addNotification({
        title: "Connection refresh already running",
        body: `${connection.label} is already scanning. Kassiber will update this page when the daemon finishes.`,
        tone: "info",
        dedupeKey: "wallet-sync",
      });
      return;
    }
    setSyncMessage(null);
    setSyncProgress(null);
    progressValueRef.current = startingSyncProgress().value ?? 5;
    syncNoticeIdRef.current = addNotification({
      title: "Connection refresh started",
      body: `${connection.label} is scanning in watch-only mode.`,
      tone: "warning",
      dedupeKey: "wallet-sync",
      progress: startingSyncProgress(),
    });
    startSyncNotice(
      `${connection.label} is still scanning. Large descriptors or slow backends can take a bit; Kassiber will update when the daemon returns.`,
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
          setSyncMessage(message);
          const notification = {
            title: status === "error" ? "Connection refresh failed" : "Connection refresh finished",
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
            error instanceof Error ? error.message : "Connection refresh failed.";
          setSyncMessage(message);
          const notification = {
            title: "Connection refresh failed",
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
          setSyncProgress(null);
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
      setError("Enter the database passphrase.");
      return false;
    }
    return true;
  };

  const onEditSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setEditError(null);
    const nextLabel = editLabel.trim();
    if (!nextLabel) {
      setEditError("Enter a connection label.");
      return;
    }
    if (!(await verifyLocalPassphrase(editPassphrase, setEditError))) return;
    if (!encryptedWorkspace && editPlaintextAck.trim() !== PLAINTEXT_CHANGE_ACK) {
      setEditError(`Type ${PLAINTEXT_CHANGE_ACK} to confirm the change.`);
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
        setEditError("Gap limit must be a positive integer.");
        return;
      }
      if (gapLimit > MAX_DESCRIPTOR_GAP_LIMIT) {
        setEditError(
          `Gap limit must be ${MAX_DESCRIPTOR_GAP_LIMIT.toLocaleString()} or lower.`,
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
    if (editConfigKind === "file-wallet" && sourceFile) {
      configChanges.source_file = sourceFile;
    }
    const clearFields: string[] = [];
    if (editClearProvenance && walletProvenanceRoutes.length > 0) {
      clearFields.push("btcpay_provenance");
    }
    if (
      !labelChanged &&
      Object.keys(configChanges).length === 0 &&
      clearFields.length === 0
    ) {
      setEditError("Change the label or update at least one field.");
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
        ? `${connection.label} was renamed to ${nextLabel}.`
        : `${connection.label} was updated.`;
      addNotification({
        title: "Connection changed",
        body: summary,
        tone: "success",
      });
      setEditOpen(false);
    } catch (error) {
      setEditError(
        error instanceof Error ? error.message : "Could not change connection.",
      );
    }
  };

  const onDeleteSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setDeleteError(null);
    if (deleteConfirm.trim() !== connection.label) {
      setDeleteError(`Type ${connection.label} to confirm removal.`);
      return;
    }
    if (!(await verifyLocalPassphrase(deletePassphrase, setDeleteError))) return;
    if (
      !encryptedWorkspace &&
      deletePlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setDeleteError(`Type ${PLAINTEXT_DELETE_ACK} to confirm local deletion.`);
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
        title: "Connection removed",
        body: `${connection.label} and its local transactions were removed.`,
        tone: "success",
      });
      setDeleteOpen(false);
      void navigate({ to: "/connections", replace: true });
    } catch (error) {
      setDeleteError(
        error instanceof Error ? error.message : "Could not remove connection.",
      );
    }
  };

  return (
    <div className={screenShellClassName}>
      <Card className="rounded-xl py-3">
        <CardContent className="flex flex-col gap-3 px-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <Button asChild variant="outline" size="icon" className="shrink-0">
              <Link to="/connections" aria-label="Back to wallets">
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
              className="min-w-[7.5rem]"
              disabled={isWalletSyncRunning}
              aria-busy={isWalletSyncRunning}
              aria-label={`${refreshButtonLabel} ${connection.label}`}
              onClick={onSync}
            >
              <RefreshCw
                className={cn("size-4", isWalletSyncRunning && "animate-spin")}
                aria-hidden="true"
              />
              <span>{refreshButtonLabel}</span>
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label="More actions"
                >
                  <MoreHorizontal className="size-4" aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem onClick={openEditDialog}>
                  <Pencil className="size-4" aria-hidden="true" />
                  Edit
                </DropdownMenuItem>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  className="text-destructive focus:text-destructive"
                  onClick={openDeleteDialog}
                >
                  <Trash2 className="size-4" aria-hidden="true" />
                  Remove
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </CardContent>
        {syncProgress && syncWallet.isPending ? (
          <div className="px-4 pt-3">
            <div className="space-y-1 rounded-md border bg-background px-3 py-2 text-sm">
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">
                  Importing {syncProgress.wallet}…
                </span>
                <span className="font-medium tabular-nums">
                  {syncProgress.processed.toLocaleString()} /{" "}
                  {syncProgress.total.toLocaleString()} transactions
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-all"
                  style={{
                    width:
                      syncProgress.total > 0
                        ? `${Math.min(
                            100,
                            (syncProgress.processed / syncProgress.total) * 100,
                          )}%`
                        : "0%",
                  }}
                />
              </div>
            </div>
          </div>
        ) : null}
        {syncMessage && (
          <div className="px-4 pt-3">
            <div
              className={cn(
                "rounded-md border px-3 py-2 text-sm",
                syncWallet.isError
                  ? "border-red-200 bg-red-50 text-red-700 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300"
                  : "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-300",
              )}
              role="status"
            >
              {syncMessage}
            </div>
          </div>
        )}
      </Card>

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <MetricCard
          label="Balance"
          value={
            <span className={blurClass(hideSensitive)}>
              {fmtBtc(connection.balance)}
            </span>
          }
          detail={fmtEur(connection.balance * priceEur)}
          icon={<Wallet className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label="Transactions"
          value={txCount.toLocaleString("en-US")}
          detail="Imported into this wallet"
          icon={<ArrowLeftRight className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label="Last sync"
          value={connection.last}
          detail={connection.status}
          icon={<RefreshCw className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label={hasGapMetric ? "Gap limit" : "Source"}
          value={
            hasGapMetric ? connection.gap?.toLocaleString("en-US") ?? "—" : sourceValue
          }
          detail={hasGapMetric ? "Unused address window" : sourceDetail}
          icon={<Database className="size-4" aria-hidden="true" />}
        />
      </div>

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
              Samourai Wallet
              <Badge variant="secondary">
                {samouraiMetadata.role === "parent"
                  ? "Group"
                  : samouraiSectionLabel(samouraiMetadata.section)}
              </Badge>
              {samouraiMetadata.privacy_boundary ? (
                <Badge variant="outline">Privacy boundary</Badge>
              ) : null}
              {samouraiMetadata.paynym ? (
                <Badge variant="outline">BIP47</Badge>
              ) : null}
            </CardTitle>
            <CardDescription>
              Watch-only Samourai/Whirlpool import state.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-3 px-4 pt-4 md:grid-cols-2">
            <DetailRow
              label="Source"
              value={samouraiSourceLabel(samouraiMetadata.source)}
            />
            <DetailRow
              label="Group"
              value={samouraiMetadata.group_label || connection.label}
            />
            {samouraiMetadata.role !== "parent" ? (
              <>
                <DetailRow
                  label="Section"
                  value={samouraiSectionLabel(samouraiMetadata.section)}
                />
                <DetailRow
                  label="Script"
                  value={samouraiMetadata.script_type || "—"}
                />
                <DetailRow
                  label="Root"
                  value={samouraiMetadata.root_path || "—"}
                  mono
                />
                <DetailRow
                  label="Gap"
                  value={
                    samouraiMetadata.gap_limit?.toLocaleString("en-US") ??
                    connection.gap?.toLocaleString("en-US") ??
                    "—"
                  }
                />
              </>
            ) : (
              <DetailRow
                label="Sections"
                value={(samouraiMetadata.sections ?? [])
                  .map(samouraiSectionLabel)
                  .join(", ")}
              />
            )}
            {samouraiMetadata.minimum_mix_count ? (
              <DetailRow
                label="Mix count"
                value={`at least ${samouraiMetadata.minimum_mix_count.toLocaleString("en-US")} · ${samouraiMetadata.mix_count_confidence ?? "minimum"}`}
              />
            ) : null}
            {samouraiMetadata.mix_count ? (
              <DetailRow
                label="Observed mixes"
                value={`${samouraiMetadata.mix_count.toLocaleString("en-US")} · ${samouraiMetadata.mix_count_confidence ?? "imported"}`}
              />
            ) : null}
            {samouraiMetadata.target_mix_count ? (
              <DetailRow
                label="Target mixes"
                value={samouraiMetadata.target_mix_count.toLocaleString("en-US")}
              />
            ) : null}
            {samouraiMetadata.pool_denomination_sat ? (
              <DetailRow
                label="Pool"
                value={`${samouraiMetadata.pool_denomination_sat.toLocaleString("en-US")} sats`}
              />
            ) : null}
            {samouraiMetadata.role !== "parent" ? (
              <DetailRow
                label="Coins"
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
                Toxic-change spends remain reportable and need normal pricing
                evidence.
              </div>
            ) : null}
            {samouraiMetadata.section === "postmix" ? (
              <div className="rounded-md border bg-background px-3 py-2 text-muted-foreground">
                Postmix rows without exact Whirlpool metadata are shown as
                having at least one mix; Kassiber does not claim exact sat
                lineage through other participants.
              </div>
            ) : null}
            {samouraiInventory?.freshness.stale ? (
              <div className="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-200">
                Samourai inventory is stale. Refresh before relying on wallet
                detail, reports, or source-of-funds readiness.
              </div>
            ) : null}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isWalletSyncRunning}
              onClick={onSync}
            >
              <RefreshCw
                className={cn("size-4", isWalletSyncRunning && "animate-spin")}
                aria-hidden="true"
              />
              {isWalletSyncRunning ? "Refreshing" : "Refresh Samourai source"}
            </Button>
          </div>
        </Card>
      ) : null}

      {walletProvenanceRoutes.length > 0 ? (
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="text-sm sm:text-base">BTCPay provenance</CardTitle>
            <CardDescription>
              BTCPay comments and labels enrich matching transactions during sync.
              Descriptor or file sync remains the balance source.
            </CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Instance</TableHead>
                  <TableHead>Store</TableHead>
                  <TableHead>Payment method</TableHead>
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
            Add more routes from <strong>Add wallet</strong> &rarr;{" "}
            <em>BTCPay Server</em> &rarr; <em>Map existing wallets</em>, or via{" "}
            <code>kassiber wallets attach-btcpay</code>. Use Edit to clear all
            routes from this wallet.
          </div>
        </Card>
      ) : null}

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.85fr)]">
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
              Recent transactions
              <CountBadge>{txsForConnection.length}</CountBadge>
            </CardTitle>
            <CardDescription>
              Recent transactions for this wallet source.
            </CardDescription>
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
                <p>No transactions imported for this wallet yet.</p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={isWalletSyncRunning}
                    onClick={onSync}
                  >
                    <RefreshCw
                      className={cn(
                        "size-4",
                        isWalletSyncRunning && "animate-spin",
                      )}
                      aria-hidden="true"
                    />
                    {isWalletSyncRunning ? "Refreshing" : "Refresh now"}
                  </Button>
                  <Button asChild type="button" variant="outline" size="sm">
                    <Link to="/imports">
                      <Plus className="size-4" aria-hidden="true" />
                      Import file
                    </Link>
                  </Button>
                </div>
              </div>
            )}
          </CardContent>
        </Card>

        <div className="space-y-3">
          <Card>
            <CardHeader className="border-b px-4 pb-3">
              <CardTitle className="text-sm sm:text-base">Connection details</CardTitle>
              <CardDescription>
                Local sync configuration.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3 px-4 pt-4">
              <DetailRow
                label="Sync mode"
                value={
                  syncModeLabels[
                    walletDetail?.sync_mode || connection.syncMode || ""
                  ] ??
                  walletDetail?.sync_mode ??
                  connection.syncMode ??
                  "—"
                }
              />
              <DetailRow
                label="Backend"
                value={formatBackendDetail(walletDetail?.backend)}
              />
              {walletDetail?.account?.label || walletDetail?.account?.code ? (
                <DetailRow
                  label="Account"
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
                  label="Network"
                  value={[walletDetail.chain, walletDetail.network]
                    .filter(Boolean)
                    .join(" · ")}
                />
              ) : null}
              <DetailRow
                label="Created"
                value={formatShortDate(walletDetail?.created_at)}
                mono
              />
              <DetailRow label="Kassiber ID" value={connection.id} mono copy />
            </CardContent>
          </Card>
        </div>
      </div>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit connection</DialogTitle>
            <DialogDescription>
              {encryptedWorkspace
                ? "Confirm this change with the local database passphrase."
                : "These plaintext books have no database passphrase; type the explicit local-change challenge to continue."}
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={onEditSubmit}>
            <div className="space-y-2">
              <Label htmlFor="connection-label">Label</Label>
              <Input
                id="connection-label"
                value={editLabel}
                onChange={(event) => setEditLabel(event.target.value)}
              />
            </div>
            {editConfigKind === "descriptor" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-material">
                  Descriptor or xpub
                </Label>
                <Textarea
                  id="connection-edit-material"
                  className="min-h-24 font-mono text-xs"
                  value={editWalletMaterial}
                  onChange={(event) =>
                    setEditWalletMaterial(event.target.value)
                  }
                  placeholder="Paste a fresh descriptor or extended public key to overwrite"
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
                          Detected: {detection.label}
                          {detection.hint ? ` — ${detection.hint}` : ""}
                        </p>
                      );
                    })()
                  : (
                      <p className="text-xs text-muted-foreground">
                        Leave empty unless you need to change the saved wallet material.
                      </p>
                    )}
              </div>
            ) : null}
            {editConfigKind === "descriptor" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-gap-limit">Gap limit</Label>
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
                  Raise this for wallets with long unused address runs. Large
                  values up to 5,000 can take longer, but refreshes keep
                  running until the daemon returns.
                </p>
              </div>
            ) : null}
            {editConfigKind === "btcpay" ? (
              <>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-backend">BTCPay instance</Label>
                  <select
                    id="connection-edit-backend"
                    className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                    value={editBackend}
                    onChange={(event) => setEditBackend(event.target.value)}
                  >
                    <option value="">Keep current instance</option>
                    {btcpayBackendOptions.map((backend) => (
                      <option key={backend.name} value={backend.name}>
                        {backendOptionLabel(backend)}
                        {backend.is_default ? " (default)" : ""}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-store">BTCPay store ID</Label>
                  <Input
                    id="connection-edit-store"
                    value={editStoreId}
                    onChange={(event) => setEditStoreId(event.target.value)}
                    placeholder="Leave empty to keep the current store ID"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="connection-edit-payment-method">
                    BTCPay wallet
                  </Label>
                  <Input
                    id="connection-edit-payment-method"
                    value={editPaymentMethodId}
                    onChange={(event) =>
                      setEditPaymentMethodId(event.target.value)
                    }
                    placeholder="Leave empty to keep BTC-CHAIN"
                  />
                </div>
              </>
            ) : null}
            {editConfigKind === "file-wallet" ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-source">Source file</Label>
                <div className="flex gap-2">
                  <Input
                    id="connection-edit-source"
                    value={editSourceFile}
                    onChange={(event) => setEditSourceFile(event.target.value)}
                    placeholder="Leave empty to keep the current file"
                  />
                  {isFilePickerAvailable ? (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={async () => {
                        const picked = await pickFile({
                          title: `Select ${connection.label} export file`,
                        });
                        if (picked) setEditSourceFile(picked);
                      }}
                    >
                      Browse…
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
                    <span>Clear BTCPay provenance routes</span>
                    <span className="text-xs text-muted-foreground">
                      Removes all {walletProvenanceRoutes.length} stored
                      route(s). Descriptor/file sync remains the balance
                      source. Re-add routes from Add wallet.
                    </span>
                  </span>
                </label>
              </div>
            ) : null}
            {encryptedWorkspace ? (
              <div className="space-y-2">
                <Label htmlFor="connection-edit-passphrase">Passphrase</Label>
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
                  Plaintext change challenge
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
                Cancel
              </Button>
              <Button type="submit" disabled={updateWallet.isPending}>
                {updateWallet.isPending ? "Saving..." : "Save"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Remove connection</DialogTitle>
            <DialogDescription>
              This removes {connection.label} and its local imported transactions. Confirm
              with{" "}
              {encryptedWorkspace
                ? "the database passphrase"
                : "the plaintext local-delete challenge"}{" "}
              and the exact connection label.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={onDeleteSubmit}>
            {encryptedWorkspace ? (
              <div className="space-y-2">
                <Label htmlFor="connection-delete-passphrase">Passphrase</Label>
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
                  Plaintext delete challenge
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
              <Label htmlFor="connection-delete-label">Connection label</Label>
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
                Cancel
              </Button>
              <Button
                type="submit"
                variant="destructive"
                disabled={deleteWallet.isPending}
              >
                {deleteWallet.isPending ? "Removing..." : "Remove"}
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
              Pending
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
