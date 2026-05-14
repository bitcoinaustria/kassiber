/**
 * Connections list view.
 *
 * Uses the shared shadcn dashboard language while keeping row navigation.
 */

import { type KeyboardEvent, type ReactNode, useEffect, useState } from "react";
import { Plus, RefreshCw, Wallet } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
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
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { useCurrency, type Currency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";

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

export function Connections() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const { syncAll, isSyncing } = useWalletSyncAction();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [addConnectionOpen, setAddConnectionOpen] = useState(false);
  const [resumeSourceId, setResumeSourceId] = useState<string | null>(null);
  const deferredConnectionSetup = useUiStore(
    (s) => s.deferredConnectionSetup,
  );
  const clearDeferredConnectionSetup = useUiStore(
    (s) => s.clearDeferredConnectionSetup,
  );

  useEffect(() => {
    if (!deferredConnectionSetup) return;
    setResumeSourceId(deferredConnectionSetup.sourceId);
    setAddConnectionOpen(true);
    clearDeferredConnectionSetup();
  }, [deferredConnectionSetup, clearDeferredConnectionSetup]);
  const onSyncAll = () => {
    syncAll();
  };

  if (isLoading || !data?.data) {
    return <ScreenSkeleton titleWidth="w-32" metricCount={3} />;
  }

  const snapshot = data.data;
  const totalBtc = snapshot.connections.reduce((s, c) => s + c.balance, 0);
  const totalEur = totalBtc * snapshot.priceEur;
  const errorN = snapshot.connections.filter((c) => c.status === "error").length;
  const snapshotSyncingN = snapshot.connections.filter((c) => c.status === "syncing").length;
  const syncingN = isSyncing
    ? snapshot.connections.length
    : snapshotSyncingN;
  const syncedN = snapshot.connections.filter((c) => c.status === "synced").length;

  const onSelect = (id: string) =>
    void navigate({
      to: "/connections/$connectionId",
      params: { connectionId: id },
    });

  return (
    <div className={screenShellClassName}>
      <div className="flex flex-col gap-2.5 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {snapshot.connections.length} wallets and sources ·{" "}
            {errorN > 0 ? `${errorN} need attention · ` : ""}
            {isSyncing
              ? `${syncingN} refreshing now`
              : snapshotSyncingN > 0
                ? `${snapshotSyncingN} refreshing`
                : "watch-only sources current"}
          </p>
          <h2 className="text-xl font-semibold tracking-tight">
            Wallets
          </h2>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-2"
            onClick={onSyncAll}
            disabled={isSyncing}
          >
            <RefreshCw
              className={cn("size-4", isSyncing && "animate-spin")}
              aria-hidden="true"
            />
            {isSyncing ? "Refreshing" : "Refresh all"}
          </Button>
          <Button
            size="sm"
            className="h-8 gap-2"
            onClick={() => setAddConnectionOpen(true)}
          >
            <Plus className="size-4" aria-hidden="true" />
            Add wallet
          </Button>
        </div>
      </div>
      <AddConnectionDialog
        open={addConnectionOpen}
        onOpenChange={(next) => {
          setAddConnectionOpen(next);
          if (!next) setResumeSourceId(null);
        }}
        initialSourceId={resumeSourceId}
      />
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <ConnectionMetric
          label="Total balance"
          value={
            <span className={blurClass(hideSensitive)}>
              <CurrencyToggleText>
                {currency === "eur"
                  ? fmtEur(totalEur)
                  : `₿ ${fmtBtc(totalBtc)}`}
              </CurrencyToggleText>
            </span>
          }
          sub={
            <CurrencyToggleText>
              {currency === "eur"
                ? `₿ ${fmtBtc(totalBtc)}`
                : fmtEur(totalEur)}
            </CurrencyToggleText>
          }
        />
        <ConnectionMetric
          label="Current"
          value={syncedN.toLocaleString("en-US")}
          sub={`${snapshot.connections.length} configured sources`}
        />
        <ConnectionMetric
          label="Needs attention"
          value={errorN.toLocaleString("en-US")}
          sub={
            syncingN > 0
              ? `${syncingN.toLocaleString("en-US")} refreshing now`
              : "No failed sources"
          }
        />
      </div>

      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="text-sm sm:text-base">Wallets and sources</CardTitle>
          <CardDescription>
            Local wallet, Lightning, ecash, and import sources available to
            Kassiber.
          </CardDescription>
        </CardHeader>
        <CardContent className="overflow-x-auto p-0">
          <Table>
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead className="min-w-[260px]">Wallet/source</TableHead>
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
  sub: ReactNode;
}

function ConnectionMetric({ label, value, sub }: ConnectionMetricProps) {
  return (
    <Card className="gap-2.5 py-4">
      <CardContent className="space-y-2 px-4">
        <div className="flex items-center gap-2 text-muted-foreground">
          <Wallet className="size-4" aria-hidden="true" />
          <span className="text-xs font-medium">{label}</span>
        </div>
        <p className="text-xl font-semibold tracking-tight">{value}</p>
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
          <CurrencyToggleText>
            {isEur ? fmtEur(c.balance * priceEur) : `₿ ${fmtBtc(c.balance)}`}
          </CurrencyToggleText>
        </div>
        <div
          className={cn(
            "text-xs text-muted-foreground tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          <CurrencyToggleText>
            {isEur ? `₿ ${fmtBtc(c.balance)}` : fmtEur(c.balance * priceEur)}
          </CurrencyToggleText>
        </div>
      </TableCell>
    </TableRow>
  );
}
