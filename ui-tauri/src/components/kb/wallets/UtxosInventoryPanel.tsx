import { useMemo, useState } from "react";
import {
  AlertTriangle,
  Coins,
  ExternalLink,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { openExternalUrl } from "@/daemon/transport";
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
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
import {
  explorerTargetForTransaction,
  type ExplorerSettings,
  type ExplorerTarget,
} from "@/lib/explorer";
import { cn } from "@/lib/utils";

export interface WalletUtxoRow {
  id: string;
  outpoint: string;
  txid: string;
  vout: number;
  asset: string;
  amount: number | string;
  amount_sat: number;
  amount_msat: number;
  confirmation_status: string;
  confirmations?: number | null;
  block_height?: number | null;
  block_time?: string | null;
  address?: string;
  address_label?: string;
  branch_label?: string;
  branch_index?: number | null;
  address_index?: number | null;
  source: {
    backend: string;
    backend_kind: string;
    chain: string;
    network: string;
    first_seen_at: string;
    last_seen_at: string;
    spent_at?: string | null;
  };
}

export interface WalletUtxosData {
  wallet: {
    id: string;
    label: string;
  } | null;
  utxos: WalletUtxoRow[];
  totals: Array<{
    asset: string;
    amount: number | string;
    amount_sat: number;
    amount_msat: number;
  }>;
  support: {
    supported: boolean;
    status: string;
    reason: string;
    message: string;
  };
  freshness: {
    status: string;
    last_seen_at?: string | null;
    last_synced_at?: string | null;
    stale: boolean;
    active_count?: number;
  };
  summary?: {
    workspace?: string | null;
    profile?: string | null;
    count: number;
  };
}

interface UtxosInventoryPanelProps {
  inventory?: WalletUtxosData | null;
  isLoading?: boolean;
  errorMessage?: string | null;
  hideSensitive: boolean;
  isRefreshing: boolean;
  explorerSettings: ExplorerSettings;
  onRefresh: () => void;
}

const BTC_ASSETS = new Set(["BTC", "LBTC", "L-BTC"]);
type UtxoSortValue =
  | "default"
  | "size-desc"
  | "size-asc"
  | "date-desc"
  | "date-asc"
  | "confirmations-desc"
  | "confirmations-asc"
  | "outpoint-asc"
  | "outpoint-desc";

export const UTXO_SORT_OPTIONS: Array<{ value: UtxoSortValue; label: string }> = [
  { value: "default", label: "Default order" },
  { value: "size-desc", label: "Size: largest first" },
  { value: "size-asc", label: "Size: smallest first" },
  { value: "date-desc", label: "Chain date: newest first" },
  { value: "date-asc", label: "Chain date: oldest first" },
  { value: "confirmations-desc", label: "Confirmations: most first" },
  { value: "confirmations-asc", label: "Confirmations: fewest first" },
  { value: "outpoint-asc", label: "Outpoint: A-Z" },
  { value: "outpoint-desc", label: "Outpoint: Z-A" },
];

function formatAmount(value: number | string, asset: string) {
  const amount = Number(value);
  if (!Number.isFinite(amount)) return `${value} ${asset}`;
  if (BTC_ASSETS.has(asset)) return `₿ ${amount.toFixed(8)}`;
  return `${amount.toFixed(8)} ${asset}`;
}

function formatOutpoint(value: string) {
  const [txid, vout] = value.split(":");
  if (!txid) return value;
  return `${txid.slice(0, 8)}…${txid.slice(-6)}:${vout ?? "?"}`;
}

function formatLocation(row: WalletUtxoRow) {
  if (row.branch_label && row.address_index !== null && row.address_index !== undefined) {
    return `${row.branch_label} #${row.address_index}`;
  }
  return row.address_label || row.branch_label || "watch target";
}

function statusLabel(row: WalletUtxoRow) {
  if (row.confirmation_status === "confirmed") {
    const confirmations = row.confirmations;
    return confirmations ? `${confirmations.toLocaleString()} conf` : "confirmed";
  }
  return "mempool";
}

function dateLabel(value: string | null | undefined) {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return value;
  return new Date(parsed).toLocaleString("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

function primaryDateLabel(row: WalletUtxoRow) {
  if (row.block_time) return dateLabel(row.block_time);
  if (row.confirmation_status !== "confirmed") return "mempool";
  return "unknown";
}

function rowDateMs(row: WalletUtxoRow) {
  const value = row.block_time;
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : 0;
}

function rowAmountMsat(row: WalletUtxoRow) {
  if (Number.isFinite(row.amount_msat)) return row.amount_msat;
  if (Number.isFinite(row.amount_sat)) return row.amount_sat * 1_000;
  const amount = Number(row.amount);
  return Number.isFinite(amount) ? amount : 0;
}

function rowConfirmations(row: WalletUtxoRow) {
  return row.confirmations ?? (row.confirmation_status === "confirmed" ? 1 : 0);
}

function compareText(left: string, right: string) {
  return left.localeCompare(right, "en-US", { numeric: true, sensitivity: "base" });
}

export function compareUtxos(left: WalletUtxoRow, right: WalletUtxoRow, sort: UtxoSortValue) {
  let result = 0;
  switch (sort) {
    case "size-desc":
    case "size-asc":
      result = rowAmountMsat(left) - rowAmountMsat(right);
      break;
    case "date-desc":
    case "date-asc":
      result = rowDateMs(left) - rowDateMs(right);
      break;
    case "confirmations-desc":
    case "confirmations-asc":
      result = rowConfirmations(left) - rowConfirmations(right);
      break;
    case "outpoint-desc":
    case "outpoint-asc":
      result = compareText(left.outpoint, right.outpoint);
      break;
    case "default":
      return 0;
  }
  if (sort.endsWith("-desc")) result *= -1;
  return result || compareText(left.outpoint, right.outpoint);
}

export function sortUtxosForDisplay(rows: WalletUtxoRow[], sort: UtxoSortValue) {
  return sort === "default"
    ? rows
    : [...rows].sort((left, right) => compareUtxos(left, right, sort));
}

function networkForUtxo(row: WalletUtxoRow) {
  const chain = row.source.chain.trim().toLowerCase();
  if (chain === "liquid" || row.asset === "LBTC" || row.asset === "L-BTC") {
    return "liquid";
  }
  if (chain === "bitcoin" || row.asset === "BTC") {
    return "bitcoin";
  }
  return null;
}

export function explorerTargetForUtxo(row: WalletUtxoRow, settings: ExplorerSettings) {
  const network = networkForUtxo(row);
  if (!network) return null;
  return explorerTargetForTransaction({
    txid: row.txid,
    network,
    settings,
  });
}

export function explorerButtonTitle(target: ExplorerTarget) {
  return `Open UTXO transaction on ${target.label}`;
}

function explorerOpenErrorMessage(error: unknown) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return "Could not open explorer in the default browser.";
}

function UtxoExplorerOpenDialog({
  row,
  target,
  onRowChange,
}: {
  row: WalletUtxoRow | null;
  target: ExplorerTarget | null;
  onRowChange: (row: WalletUtxoRow | null) => void;
}) {
  const [openError, setOpenError] = useState<string | null>(null);
  const [opening, setOpening] = useState(false);

  const openExplorer = async () => {
    if (!target) return;
    setOpenError(null);
    setOpening(true);
    try {
      await openExternalUrl(target.url);
      onRowChange(null);
    } catch (error) {
      setOpenError(explorerOpenErrorMessage(error));
    } finally {
      setOpening(false);
    }
  };

  return (
    <Dialog
      open={Boolean(row)}
      onOpenChange={(open) => {
        if (!open) {
          setOpenError(null);
          onRowChange(null);
        }
      }}
    >
      <DialogContent className="max-h-[calc(100dvh-2rem)] w-[min(calc(100vw-2rem),34rem)] overflow-hidden p-0 sm:max-w-none">
        <div className="grid max-h-[calc(100dvh-2rem)] min-w-0 gap-4 overflow-y-auto p-4 sm:p-6">
          <DialogHeader className="min-w-0 pr-8">
            <div className="mb-1 flex size-10 shrink-0 items-center justify-center rounded-full bg-amber-100 text-amber-700 dark:bg-amber-950/50 dark:text-amber-300">
              <ShieldAlert className="size-5" aria-hidden="true" />
            </div>
            <DialogTitle>Open UTXO transaction in a browser?</DialogTitle>
            <DialogDescription className="max-w-prose">
              This opens {target?.label ?? "the configured explorer"} outside Kassiber.
              The explorer can see your IP address and the transaction id you
              request.
            </DialogDescription>
          </DialogHeader>
          {row && target ? (
            <div className="min-w-0 rounded-md border bg-muted/35 p-3 text-sm">
              <p className="truncate font-medium">{formatOutpoint(row.outpoint)}</p>
              <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
                {target.url}
              </p>
            </div>
          ) : null}
          {openError ? (
            <p
              role="alert"
              className="rounded-md border border-destructive/25 bg-destructive/10 px-3 py-2 text-sm text-destructive"
            >
              {openError}
            </p>
          ) : null}
          <DialogFooter className="gap-2 sm:flex-wrap">
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="button"
              disabled={!target || opening}
              onClick={() => void openExplorer()}
            >
              <ExternalLink className="size-4" aria-hidden="true" />
              {opening ? "Opening..." : "Open explorer"}
            </Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function EmptyState({
  title,
  body,
  onRefresh,
  isRefreshing,
}: {
  title: string;
  body: string;
  onRefresh: () => void;
  isRefreshing: boolean;
}) {
  return (
    <div className="flex flex-col items-start gap-3 px-5 py-8 text-sm text-muted-foreground">
      <div className="space-y-1">
        <p className="font-medium text-foreground">{title}</p>
        <p>{body}</p>
      </div>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={isRefreshing}
        onClick={onRefresh}
      >
        <RefreshCw
          className={cn("size-4", isRefreshing && "animate-spin")}
          aria-hidden="true"
        />
        {isRefreshing ? "Refreshing" : "Refresh"}
      </Button>
    </div>
  );
}

export function UtxosInventoryPanel({
  inventory,
  isLoading = false,
  errorMessage,
  hideSensitive,
  isRefreshing,
  explorerSettings,
  onRefresh,
}: UtxosInventoryPanelProps) {
  const rows = inventory?.utxos ?? [];
  const [sort, setSort] = useState<UtxoSortValue>("default");
  const [explorerRow, setExplorerRow] = useState<WalletUtxoRow | null>(null);
  const sortedRows = useMemo(
    () => sortUtxosForDisplay(rows, sort),
    [rows, sort],
  );
  const explorerTarget = explorerRow
    ? explorerTargetForUtxo(explorerRow, explorerSettings)
    : null;
  const stale = Boolean(inventory?.freshness.stale);
  const unsupported = inventory?.support.supported === false;
  const liquidBlocked = inventory?.support.status === "liquid_unblind_blocked";
  const title = liquidBlocked ? "Liquid UTXOs need unblinding" : "UTXO inventory unavailable";

  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
              <Coins className="size-4" aria-hidden="true" />
              UTXOs
              <span className="inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
                {rows.length.toLocaleString("en-US")}
              </span>
            </CardTitle>
            <CardDescription>
              Currently unspent transaction outputs known from this source.
            </CardDescription>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {rows.length > 1 ? (
              <Select
                value={sort}
                onValueChange={(value) => setSort(value as UtxoSortValue)}
              >
                <SelectTrigger
                  size="sm"
                  className="w-[min(100%,14rem)]"
                  aria-label="Sort UTXOs"
                >
                  <SelectValue placeholder="Sort UTXOs" />
                </SelectTrigger>
                <SelectContent align="end">
                  {UTXO_SORT_OPTIONS.map((option) => (
                    <SelectItem key={option.value} value={option.value}>
                      {option.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            ) : null}
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isRefreshing}
              onClick={onRefresh}
            >
              <RefreshCw
                className={cn("size-4", isRefreshing && "animate-spin")}
                aria-hidden="true"
              />
              {isRefreshing ? "Refreshing" : "Refresh"}
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent className="p-0">
        {isLoading ? (
          <div className="space-y-3 p-4">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-2/3" />
          </div>
        ) : errorMessage ? (
          <EmptyState
            title="UTXO inventory could not load"
            body={errorMessage}
            onRefresh={onRefresh}
            isRefreshing={isRefreshing}
          />
        ) : unsupported ? (
          <div className="flex flex-col items-start gap-3 px-5 py-8 text-sm text-muted-foreground">
            <div className="flex items-start gap-3">
              <AlertTriangle
                className="mt-0.5 size-4 shrink-0 text-amber-600"
                aria-hidden="true"
              />
              <div className="space-y-1">
                <p className="font-medium text-foreground">{title}</p>
                <p>
                  {inventory?.support.message ||
                    "This wallet source does not expose a watch-only UTXO inventory."}
                </p>
              </div>
            </div>
          </div>
        ) : rows.length === 0 ? (
          <EmptyState
            title="No UTXOs known"
            body="Refresh this source to update Kassiber's UTXO inventory."
            onRefresh={onRefresh}
            isRefreshing={isRefreshing}
          />
        ) : (
          <>
            {stale ? (
              <div className="flex items-start gap-2 border-b bg-amber-50 px-4 py-2.5 text-xs text-amber-800 dark:bg-amber-950/30 dark:text-amber-200">
                <AlertTriangle className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
                <span>Refresh this source to update the UTXO inventory.</span>
              </div>
            ) : null}
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Outpoint</TableHead>
                  <TableHead>Amount</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Address</TableHead>
                  <TableHead className="text-right">Chain date</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sortedRows.map((row) => {
                  const explorer = explorerTargetForUtxo(row, explorerSettings);
                  return (
                    <TableRow key={row.id || row.outpoint}>
                      <TableCell className="font-mono text-xs">
                        {explorer ? (
                          <button
                            type="button"
                            className={cn(
                              "inline-flex max-w-[22ch] items-center gap-1 truncate text-left underline-offset-4 hover:underline",
                              hideSensitive && "sensitive",
                            )}
                            title={explorerButtonTitle(explorer)}
                            onClick={() => setExplorerRow(row)}
                          >
                            <span className="truncate">
                              {formatOutpoint(row.outpoint)}
                            </span>
                            <ExternalLink
                              className="size-3 shrink-0 text-muted-foreground"
                              aria-hidden="true"
                            />
                          </button>
                        ) : (
                          formatOutpoint(row.outpoint)
                        )}
                      </TableCell>
                      <TableCell>
                        <span className={cn("font-medium", hideSensitive && "sensitive")}>
                          {formatAmount(row.amount, row.asset)}
                        </span>
                        <span className="ml-1 text-xs text-muted-foreground">
                          {row.asset}
                        </span>
                      </TableCell>
                      <TableCell>
                        <Badge variant={row.confirmation_status === "confirmed" ? "secondary" : "outline"}>
                          {statusLabel(row)}
                        </Badge>
                      </TableCell>
                      <TableCell className="max-w-[220px]">
                        <div className="truncate text-sm">{formatLocation(row)}</div>
                        {row.address ? (
                          <div
                            className={cn(
                              "truncate font-mono text-xs text-muted-foreground",
                              hideSensitive && "sensitive",
                            )}
                          >
                            {row.address}
                          </div>
                        ) : null}
                      </TableCell>
                      <TableCell className="text-right text-xs text-muted-foreground">
                        <div className="font-mono">
                          {primaryDateLabel(row) || "unknown"}
                        </div>
                        {row.source.last_seen_at &&
                        row.source.last_seen_at !== row.block_time ? (
                          <div className="mt-1 font-mono">
                            seen {dateLabel(row.source.last_seen_at)}
                          </div>
                        ) : null}
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
            <UtxoExplorerOpenDialog
              row={explorerRow}
              target={explorerTarget}
              onRowChange={setExplorerRow}
            />
          </>
        )}
      </CardContent>
    </Card>
  );
}
