/**
 * Connection detail page.
 *
 * The detail surface uses the shared shadcn component vocabulary from the
 * Connections and Overview screens.
 */

import { useState, type FormEvent, type ReactNode } from "react";
import { Link, useNavigate, useParams } from "@tanstack/react-router";
import {
  ArrowLeft,
  Check,
  Copy,
  Database,
  Eye,
  EyeOff,
  KeyRound,
  Pencil,
  RefreshCw,
  Trash2,
  Wallet,
} from "lucide-react";

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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "@/hooks/useSyncProgressNotice";
import type {
  Connection,
  ConnectionKind,
  ConnectionStatus,
  OverviewSnapshot,
} from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (value: number) => `₿ ${value.toFixed(8)}`;
const fmtEur = (value: number) =>
  "€ " +
  value.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
const fmtSatSigned = (amountSat: number) =>
  `${amountSat >= 0 ? "+ " : "- "}${Math.abs(amountSat).toLocaleString(
    "en-US",
  )}`;
const fmtEurSigned = (amountEur: number) =>
  `${amountEur >= 0 ? "+ " : "- "}${fmtEur(Math.abs(amountEur))}`;
const compactAddress = (value: string) =>
  value.length <= 16 ? value : `${value.slice(0, 8)}…${value.slice(-8)}`;

const SYNTHETIC_ADDRESSES = [
  "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
  "bc1q9d4ywgfnd8h43da5tpcxcn6ajv590cg6d3tg6a",
  "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh",
  "bc1pgw9z80zvz6jcdqfp3hjlam77t34ddln0wfqp6w",
  "bc1qm34lsc65zpw79lxes69zkqmk6ee3ewf0j77s3h",
];

const FULL_XPUB =
  "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4ogpiMZbpiaQL2j8mdfKB3kRvvKUC7vw3R7Y8eYS9zPNxKr1J9";
const SHORT_XPUB = "xpub6C…aQL2j";
const PLAINTEXT_CHANGE_ACK = "CHANGE LOCAL DATA";
const PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA";

const kindLabels: Record<ConnectionKind, string> = {
  xpub: "XPUB",
  address: "Address",
  descriptor: "Descriptor",
  "core-ln": "Core Lightning",
  lnd: "LND",
  nwc: "NWC",
  cashu: "Cashu",
  btcpay: "BTCPay",
  kraken: "Kraken",
  bitstamp: "Bitstamp",
  coinbase: "Coinbase",
  bitpanda: "Bitpanda",
  river: "River",
  strike: "Strike",
  phoenix: "Phoenix",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
};

const statusStyles: Record<ConnectionStatus, string> = {
  synced:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  syncing:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  idle: "border-border bg-muted text-muted-foreground",
  error: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
};

interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
}

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

export function ConnectionDetail() {
  const { connectionId } = useParams({ from: "/_app/connections/$connectionId" });
  const { data, isLoading } = useDaemon<OverviewSnapshot>(
    "ui.overview.snapshot",
  );
  const hideSensitive = useUiStore((state) => state.hideSensitive);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading connection...
      </div>
    );
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
            Back to connections
          </Link>
        </Button>
      </div>
    );
  }

  return (
    <ConnectionDetailView
      connection={connection}
      priceEur={snapshot.priceEur}
      txs={snapshot.txs}
      hideSensitive={hideSensitive}
    />
  );
}

interface ConnectionDetailViewProps {
  connection: Connection;
  priceEur: number;
  txs: OverviewSnapshot["txs"];
  hideSensitive: boolean;
}

function ConnectionDetailView({
  connection,
  priceEur,
  txs,
  hideSensitive,
}: ConnectionDetailViewProps) {
  const navigate = useNavigate();
  const addNotification = useUiStore((state) => state.addNotification);
  const identity = useUiStore((state) => state.identity);
  const syncWallet = useDaemonMutation<{ results: SyncResult[] }>("ui.wallets.sync");
  const updateWallet =
    useDaemonMutation<UpdateWalletResult>("ui.wallets.update");
  const deleteWallet =
    useDaemonMutation<DeleteWalletResult>("ui.wallets.delete");
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const [syncMessage, setSyncMessage] = useState<string | null>(null);
  const [editOpen, setEditOpen] = useState(false);
  const [editLabel, setEditLabel] = useState(connection.label);
  const [editPassphrase, setEditPassphrase] = useState("");
  const [editPlaintextAck, setEditPlaintextAck] = useState("");
  const [editError, setEditError] = useState<string | null>(null);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deletePassphrase, setDeletePassphrase] = useState("");
  const [deletePlaintextAck, setDeletePlaintextAck] = useState("");
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const isXpubLike =
    connection.kind === "xpub" || connection.kind === "descriptor";
  const addressCount = connection.addresses ?? connection.channels ?? 0;
  const txsForConnection = txs
    .filter((tx) =>
      tx.account
        .toLowerCase()
        .includes(connection.label.toLowerCase().split(" ")[0].toLowerCase()),
    )
    .slice(0, 6);
  const displayTxs = txsForConnection.length > 0 ? txsForConnection : txs.slice(0, 6);

  const onSync = () => {
    if (syncWallet.isPending) return;
    setSyncMessage(null);
    addNotification({
      title: "Wallet sync started",
      body: `${connection.label} is syncing.`,
      tone: "warning",
    });
    startSyncNotice(
      `${connection.label} is still syncing. Large descriptors or slow backends can take a bit; Kassiber will update when the daemon returns.`,
    );
    syncWallet.mutate(
      { wallet: connection.label },
      {
        onSuccess: (envelope) => {
          const result = envelope.data?.results?.find(
            (item) => item.wallet === connection.label,
          );
          const status = result?.status ?? "synced";
          const message =
            status === "error"
              ? `${connection.label} sync returned an error.`
              : `${connection.label} sync ${status}.`;
          setSyncMessage(message);
          addNotification({
            title: status === "error" ? "Wallet sync failed" : "Wallet sync finished",
            body: message,
            tone: status === "error" ? "error" : "success",
          });
        },
        onError: (error) => {
          const message =
            error instanceof Error ? error.message : "Wallet sync failed.";
          setSyncMessage(message);
          addNotification({
            title: "Wallet sync failed",
            body: message,
            tone: "error",
          });
        },
        onSettled: clearSyncNotice,
      },
    );
  };

  const openEditDialog = () => {
    setEditLabel(connection.label);
    setEditPassphrase("");
    setEditPlaintextAck("");
    setEditError(null);
    setEditOpen(true);
  };

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

    try {
      await updateWallet.mutateAsync({
        wallet: connection.id,
        label: nextLabel,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: editPassphrase }
          : { plaintext_change_ack: PLAINTEXT_CHANGE_ACK },
      });
      addNotification({
        title: "Connection changed",
        body: `${connection.label} was renamed to ${nextLabel}.`,
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
        body: `${connection.label} and its local rows were removed.`,
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
      <Card className="py-4">
        <CardContent className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 items-center gap-3">
            <Button asChild variant="outline" size="icon" className="shrink-0">
              <Link to="/connections" aria-label="Back to connections">
                <ArrowLeft className="size-4" aria-hidden="true" />
              </Link>
            </Button>
            <div className="min-w-0">
              <div className="mb-2 flex flex-wrap items-center gap-2">
                <Badge variant="outline">{kindLabels[connection.kind]}</Badge>
                <Badge className={statusStyles[connection.status]} variant="outline">
                  {connection.status}
                </Badge>
              </div>
              <h1 className="truncate text-2xl font-semibold tracking-tight">
                {connection.label}
              </h1>
            </div>
          </div>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="self-start sm:self-center"
            disabled={syncWallet.isPending}
            onClick={onSync}
          >
            <RefreshCw
              className={cn("size-4", syncWallet.isPending && "animate-spin")}
              aria-hidden="true"
            />
            {syncWallet.isPending ? "Syncing" : "Sync"}
          </Button>
        </CardContent>
        {syncMessage && (
          <div className="px-6 pt-4">
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
          label={connection.channels != null ? "Channels" : "Addresses"}
          value={addressCount.toLocaleString("en-US")}
          detail={connection.channels != null ? "Lightning channels" : "Derived rows"}
          icon={<KeyRound className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label="Last sync"
          value={connection.last}
          detail={connection.status}
          icon={<RefreshCw className="size-4" aria-hidden="true" />}
        />
        <MetricCard
          label="Gap limit"
          value={connection.gap?.toLocaleString("en-US") ?? "—"}
          detail={connection.gap != null ? "Unused address window" : "Not applicable"}
          icon={<Database className="size-4" aria-hidden="true" />}
        />
      </div>

      <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.85fr)]">
        <Card>
          <CardHeader className="border-b">
            <CardTitle>Recent transactions</CardTitle>
            <CardDescription>
              Recent rows that match this connection label.
            </CardDescription>
          </CardHeader>
          <CardContent className="p-0">
            <Table>
              <TableHeader>
                <TableRow className="bg-muted/50 hover:bg-muted/50">
                  <TableHead>Date</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead className="text-right">Sats</TableHead>
                  <TableHead className="text-right">EUR</TableHead>
                  <TableHead className="text-right">Conf</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {displayTxs.map((tx) => (
                  <TableRow key={tx.id}>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {tx.date.slice(5)}
                    </TableCell>
                    <TableCell>{tx.type}</TableCell>
                    <TableCell
                      className={cn(
                        "text-right font-mono text-xs tabular-nums",
                        tx.amountSat > 0
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-foreground",
                        blurClass(hideSensitive),
                      )}
                    >
                      {fmtSatSigned(tx.amountSat)}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-right font-mono text-xs tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {fmtEurSigned(tx.eur)}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground tabular-nums">
                      {tx.conf}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>

        <div className="space-y-4">
          <Card>
            <CardHeader className="border-b">
              <CardTitle>Connection details</CardTitle>
              <CardDescription>
                Local metadata and source configuration.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4 pt-6">
              <DetailRow label="Label" value={connection.label} />
              <DetailRow label="Type" value={connection.kind.toUpperCase()} mono />
              <DetailRow
                label="Derivation path"
                value={connection.kind === "xpub" ? "m / 84' / 0' / 0'" : "—"}
                mono
              />
              {isXpubLike && (
                <>
                  <DetailRow label="Fingerprint" value="5f3a8c0e" mono copy />
                  <RevealRow
                    label="Account xpub"
                    full={FULL_XPUB}
                    short={SHORT_XPUB}
                    hideSensitive={hideSensitive}
                  />
                </>
              )}
              <DetailRow label="Backend" value="mempool" />
              <DetailRow label="Created" value="2026-03-02 10:14" mono />
              <DetailRow label="Kassiber ID" value={`conn_${connection.id}`} mono />
            </CardContent>
          </Card>

          <Card>
            <CardHeader className="border-b">
              <CardTitle>Connection actions</CardTitle>
              <CardDescription>
                Changing or removing a wallet source requires local confirmation.
              </CardDescription>
            </CardHeader>
            <CardContent className="flex flex-wrap gap-2 pt-6">
              <Button type="button" variant="outline" onClick={openEditDialog}>
                <Pencil className="size-4" aria-hidden="true" />
                Edit label
              </Button>
              <Button
                type="button"
                variant="destructive"
                onClick={openDeleteDialog}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                Remove
              </Button>
            </CardContent>
          </Card>

          {isXpubLike && (
            <Card>
              <CardHeader className="border-b">
                <CardTitle>Derived addresses</CardTitle>
                <CardDescription>
                  Preview rows from the receive branch.
                </CardDescription>
              </CardHeader>
              <CardContent className="p-0">
                <Table>
                  <TableBody>
                    {SYNTHETIC_ADDRESSES.map((address, index) => (
                      <TableRow key={address}>
                        <TableCell className="p-2 align-middle">
                          <div className="flex min-w-0 items-center gap-2">
                            <span
                              className={cn(
                                "min-w-0 font-mono text-xs tabular-nums",
                                blurClass(hideSensitive),
                              )}
                            >
                              {compactAddress(address)}
                            </span>
                            <CopyButton
                              value={address}
                              ariaLabel="Copy full address"
                            />
                          </div>
                        </TableCell>
                        <TableCell className="text-right font-mono text-xs text-muted-foreground">
                          m/84&apos;/0&apos;/0&apos;/0/{index}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          )}
        </div>
      </div>

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Edit connection label</DialogTitle>
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
              This removes {connection.label} and its local imported rows. Confirm
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
    </div>
  );
}

interface MetricCardProps {
  label: string;
  value: ReactNode;
  detail: string;
  icon: ReactNode;
}

function MetricCard({ label, value, detail, icon }: MetricCardProps) {
  return (
    <Card className="gap-3 py-5">
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2 text-muted-foreground">
          {icon}
          <span className="text-xs font-medium">{label}</span>
        </div>
        <p className="text-2xl font-semibold tracking-tight tabular-nums">
          {value}
        </p>
        <p className="text-xs text-muted-foreground">{detail}</p>
      </CardContent>
    </Card>
  );
}

interface DetailRowProps {
  label: string;
  value: ReactNode;
  mono?: boolean;
  copy?: boolean;
}

function DetailRow({ label, value, mono, copy }: DetailRowProps) {
  return (
    <div className="grid gap-1">
      <div className="text-xs font-medium text-muted-foreground">{label}</div>
      <div className="flex min-w-0 items-center gap-2">
        <div
          className={cn(
            "min-w-0 flex-1 truncate text-sm",
            mono && "font-mono text-xs",
          )}
        >
          {value}
        </div>
        {copy && typeof value === "string" && <CopyButton value={value} />}
      </div>
    </div>
  );
}

interface RevealRowProps {
  label: string;
  full: string;
  short: string;
  hideSensitive: boolean;
}

function RevealRow({ label, full, short, hideSensitive }: RevealRowProps) {
  const [revealed, setRevealed] = useState(false);
  const masked = !revealed || hideSensitive;
  return (
    <div className="grid gap-1">
      <div className="text-xs font-medium text-muted-foreground">{label}</div>
      <div className="flex min-w-0 items-center gap-2">
        <div
          className={cn(
            "min-w-0 flex-1 truncate font-mono text-xs",
            masked && "sensitive",
          )}
        >
          {revealed && !hideSensitive ? full : short}
        </div>
        <Button
          type="button"
          variant="outline"
          size="icon-xs"
          aria-label={revealed ? "Hide xpub" : "Reveal xpub"}
          onClick={() => setRevealed((current) => !current)}
        >
          {revealed ? (
            <EyeOff className="size-3" aria-hidden="true" />
          ) : (
            <Eye className="size-3" aria-hidden="true" />
          )}
        </Button>
        <CopyButton value={full} />
      </div>
    </div>
  );
}

function CopyButton({
  value,
  ariaLabel = "Copy",
}: {
  value: string;
  ariaLabel?: string;
}) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1100);
    } catch {
      // Clipboard access is best-effort in browser preview.
    }
  };
  return (
    <Button
      type="button"
      variant="outline"
      size="icon-xs"
      aria-label={copied ? "Copied" : ariaLabel}
      onClick={onCopy}
    >
      {copied ? (
        <Check
          className="size-3 text-emerald-600 dark:text-emerald-400"
          aria-hidden="true"
        />
      ) : (
        <Copy className="size-3" aria-hidden="true" />
      )}
    </Button>
  );
}
