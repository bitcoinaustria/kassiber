/**
 * Connections list view.
 *
 * Uses the shared shadcn dashboard language while keeping row navigation.
 */

import { type KeyboardEvent, type ReactNode, useState } from "react";
import { Plus, RefreshCw, Wallet } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

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
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "@/hooks/useSyncProgressNotice";
import { useCurrency, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import type {
  Connection,
  ConnectionKind,
  ConnectionStatus,
  OverviewSnapshot,
} from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (v: number) => v.toFixed(8);
const fmtEur = (v: number) =>
  "€ " +
  v.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

const kindLabels: Record<ConnectionKind, string> = {
  xpub: "On-chain",
  address: "On-chain",
  descriptor: "On-chain",
  "core-ln": "Lightning",
  lnd: "Lightning",
  nwc: "NWC",
  cashu: "Ecash",
  btcpay: "BTCPay",
  kraken: "Exchange",
  bitstamp: "Exchange",
  coinbase: "Exchange",
  bitpanda: "Exchange",
  river: "Exchange",
  strike: "Lightning",
  phoenix: "Lightning",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
};

const statusStyles: Record<ConnectionStatus, string> = {
  synced:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  syncing:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  idle: "bg-muted text-muted-foreground ring-border",
  error:
    "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
};

const statusDotStyles: Record<ConnectionStatus, string> = {
  synced: "bg-emerald-500",
  syncing: "bg-amber-500",
  idle: "bg-muted-foreground/50",
  error: "bg-red-500",
};

interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
  message?: string;
  reason?: string;
}

export function Connections() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const syncWallets = useDaemonMutation<{ results: SyncResult[] }>("ui.wallets.sync");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const addNotification = useUiStore((s) => s.addNotification);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [syncMessage, setSyncMessage] = useState<string | null>(null);
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const onSyncAll = () => {
    if (syncWallets.isPending) return;
    setSyncMessage(null);
    addNotification({
      title: "Wallet sync started",
      body: "Kassiber is syncing all configured wallet sources.",
      tone: "warning",
    });
    startSyncNotice();
    syncWallets.mutate(
      { all: true },
      {
        onSuccess: (envelope) => {
          const results = envelope.data?.results ?? [];
          const synced = results.filter((result) => result.status === "synced").length;
          const skipped = results.filter((result) => result.status === "skipped").length;
          const errors = results.filter((result) => result.status === "error").length;
          setSyncMessage(
            [
              synced ? `${synced} synced` : null,
              skipped ? `${skipped} skipped` : null,
              errors ? `${errors} failed` : null,
            ]
              .filter(Boolean)
              .join(", ") || "Sync finished",
          );
          addNotification({
            title: errors ? "Wallet sync finished with errors" : "Wallet sync finished",
            body:
              [
                synced ? `${synced} synced` : null,
                skipped ? `${skipped} skipped` : null,
                errors ? `${errors} failed` : null,
              ]
                .filter(Boolean)
                .join(", ") || "No wallet changes returned.",
            tone: errors ? "error" : "success",
          });
        },
        onError: (error) => {
          const message =
            error instanceof Error ? error.message : "Wallet sync failed";
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

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading connections...
      </div>
    );
  }

  const snapshot = data.data;
  const totalBtc = snapshot.connections.reduce((s, c) => s + c.balance, 0);
  const totalEur = totalBtc * snapshot.priceEur;
  const errorN = snapshot.connections.filter((c) => c.status === "error").length;
  const snapshotSyncingN = snapshot.connections.filter((c) => c.status === "syncing").length;
  const syncingN = syncWallets.isPending
    ? snapshot.connections.length
    : snapshotSyncingN;
  const syncedN = snapshot.connections.filter((c) => c.status === "synced").length;

  const onSelect = (id: string) =>
    void navigate({
      to: "/connections/$connectionId",
      params: { connectionId: id },
    });

  return (
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {snapshot.connections.length} connections ·{" "}
            {errorN > 0 ? `${errorN} need attention · ` : ""}
            {syncWallets.isPending
              ? `${syncingN} syncing now`
              : snapshotSyncingN > 0
                ? `${snapshotSyncingN} updating`
                : "all synced"}
          </p>
          <h2 className="text-2xl font-semibold tracking-tight">
            Connections
          </h2>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-9 gap-2"
            onClick={onSyncAll}
            disabled={syncWallets.isPending}
          >
            <RefreshCw
              className={cn("size-4", syncWallets.isPending && "animate-spin")}
              aria-hidden="true"
            />
            {syncWallets.isPending ? "Syncing" : "Sync all"}
          </Button>
          <Button
            size="sm"
            className="h-9 gap-2"
            onClick={() => void navigate({ to: "/imports" })}
          >
            <Plus className="size-4" aria-hidden="true" />
            Add connection
          </Button>
        </div>
      </div>
      {syncMessage && (
        <div
          className={cn(
            "rounded-md border px-3 py-2 text-sm",
            syncWallets.isError
              ? "border-red-200 bg-red-50 text-red-700 dark:border-red-900/60 dark:bg-red-950/40 dark:text-red-300"
              : "border-emerald-200 bg-emerald-50 text-emerald-700 dark:border-emerald-900/60 dark:bg-emerald-950/40 dark:text-emerald-300",
          )}
          role="status"
        >
          {syncMessage}
        </div>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <ConnectionMetric
          label="Total balance"
          value={
            <span className={blurClass(hideSensitive)}>
              {currency === "eur" ? fmtEur(totalEur) : `₿ ${fmtBtc(totalBtc)}`}
            </span>
          }
          sub={
            currency === "eur"
              ? `₿ ${fmtBtc(totalBtc)}`
              : fmtEur(totalEur)
          }
        />
        <ConnectionMetric
          label="Synced"
          value={syncedN.toLocaleString("en-US")}
          sub={`${snapshot.connections.length} configured sources`}
        />
        <ConnectionMetric
          label="Active sync"
          value={syncingN.toLocaleString("en-US")}
          sub={errorN > 0 ? `${errorN} need attention` : "No errors"}
        />
      </div>

      <Card>
        <CardHeader className="border-b">
          <CardTitle>Wallets and sources</CardTitle>
          <CardDescription>
            Local wallet, Lightning, ecash, and import sources available to
            Kassiber.
          </CardDescription>
        </CardHeader>
        <CardContent className="overflow-x-auto p-0">
          <Table>
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead className="min-w-[260px]">Connection</TableHead>
                <TableHead>Kind</TableHead>
                <TableHead>Last sync</TableHead>
                <TableHead className="hidden min-w-[180px] lg:table-cell">
                  Composition
                </TableHead>
                <TableHead className="text-right">Balance</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {snapshot.connections.map((connection) => (
                <ConnectionRow
                  key={connection.id}
                  connection={connection}
                  totalBtc={totalBtc}
                  priceEur={snapshot.priceEur}
                  hideSensitive={hideSensitive}
                  currency={currency}
                  onSelect={() => onSelect(connection.id)}
                />
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}

interface ConnectionMetricProps {
  label: string;
  value: ReactNode;
  sub: string;
}

function ConnectionMetric({ label, value, sub }: ConnectionMetricProps) {
  return (
    <Card className="gap-3 py-5">
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2 text-muted-foreground">
          <Wallet className="size-4" aria-hidden="true" />
          <span className="text-xs font-medium">{label}</span>
        </div>
        <p className="text-2xl font-semibold tracking-tight">{value}</p>
        <p className="text-xs text-muted-foreground">{sub}</p>
      </CardContent>
    </Card>
  );
}

interface ConnectionRowProps {
  connection: Connection;
  totalBtc: number;
  priceEur: number;
  hideSensitive: boolean;
  currency: Currency;
  onSelect: () => void;
}

function ConnectionRow({
  connection: c,
  totalBtc,
  priceEur,
  hideSensitive,
  currency,
  onSelect,
}: ConnectionRowProps) {
  const sats = Math.round(c.balance * 1e8);
  const pct = totalBtc > 0 ? (c.balance / totalBtc) * 100 : 0;
  const isEur = currency === "eur";

  const onKeyDown = (event: KeyboardEvent<HTMLTableRowElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect();
    }
  };

  return (
    <TableRow
      role="button"
      tabIndex={0}
      className="cursor-pointer"
      onClick={onSelect}
      onKeyDown={onKeyDown}
    >
      <TableCell>
        <div className="flex items-center gap-3">
          <span
            className={cn("size-2.5 rounded-full", statusDotStyles[c.status])}
            aria-hidden="true"
          />
          <div className="min-w-0">
            <div className="truncate font-medium">{c.label}</div>
            <div className="truncate text-xs text-muted-foreground">
              {c.addresses != null && `${c.addresses} addresses`}
              {c.channels != null && `${c.channels} channels`}
              {c.gap != null && ` · gap ${c.gap}`}
              {c.addresses == null && c.channels == null && "Ready"}
            </div>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <span className="inline-flex items-center rounded-md border px-2 py-1 text-xs text-muted-foreground">
          {kindLabels[c.kind]}
        </span>
      </TableCell>
      <TableCell>
        <div className="grid gap-1">
          <span className="text-sm">{c.last}</span>
          <span
            className={cn(
              "w-fit rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
              statusStyles[c.status],
            )}
          >
            {c.status}
          </span>
        </div>
      </TableCell>
      <TableCell className="hidden lg:table-cell">
        <div className="flex items-center gap-2">
          <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-muted">
            <div
              className="absolute inset-y-0 left-0 rounded-full bg-primary transition-[width]"
              style={{ width: `${Math.max(1.5, pct)}%` }}
            />
          </div>
          <span
            className={cn(
              "w-12 text-right text-xs text-muted-foreground tabular-nums",
              blurClass(hideSensitive),
            )}
          >
            {pct < 0.1 ? "<0.1%" : `${pct.toFixed(pct < 10 ? 1 : 0)}%`}
          </span>
        </div>
      </TableCell>
      <TableCell className="text-right">
        <div
          className={cn("font-medium tabular-nums", blurClass(hideSensitive))}
        >
          {isEur ? fmtEur(c.balance * priceEur) : `₿ ${fmtBtc(c.balance)}`}
        </div>
        <div
          className={cn(
            "text-xs text-muted-foreground tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {isEur
            ? `₿ ${fmtBtc(c.balance)} · ${sats.toLocaleString("en-US")} sat`
            : `${sats.toLocaleString("en-US")} sat · ${fmtEur(
                c.balance * priceEur,
              )}`}
        </div>
      </TableCell>
    </TableRow>
  );
}
