import { AlertTriangle, Coins, RefreshCw } from "lucide-react";

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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Skeleton } from "@/components/ui/skeleton";
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
  onRefresh: () => void;
}

const BTC_ASSETS = new Set(["BTC", "LBTC", "L-BTC"]);

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
  onRefresh,
}: UtxosInventoryPanelProps) {
  const rows = inventory?.utxos ?? [];
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
                <p>{inventory?.support.message || "This wallet source does not expose a watch-only UTXO inventory."}</p>
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
                  <TableHead className="text-right">Seen</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((row) => (
                  <TableRow key={row.id || row.outpoint}>
                    <TableCell className="font-mono text-xs">
                      {formatOutpoint(row.outpoint)}
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
                        <div className={cn("truncate font-mono text-xs text-muted-foreground", hideSensitive && "sensitive")}>
                          {row.address}
                        </div>
                      ) : null}
                    </TableCell>
                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {row.source.last_seen_at || "never"}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </>
        )}
      </CardContent>
    </Card>
  );
}
