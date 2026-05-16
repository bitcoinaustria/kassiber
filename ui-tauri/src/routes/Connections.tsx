/**
 * Connections list view.
 *
 * Uses the shared shadcn dashboard language while keeping row navigation.
 */

import { type KeyboardEvent, type ReactNode, useEffect, useState } from "react";
import { Filter, Plus, RefreshCw, Wallet, X } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";

import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
import {
  connectionKindCategoryLabels,
  connectionKindTone,
  connectionStatusStyles,
} from "@/lib/connectionDisplay";
import { useCurrency, type Currency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";

import type {
  Connection,
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

const statusDotStyles: Record<ConnectionStatus, string> = {
  synced: "bg-emerald-500",
  syncing: "bg-amber-500",
  idle: "bg-muted-foreground/50",
  error: "bg-red-500",
};

const kindFilterOptions = Array.from(
  new Set(Object.values(connectionKindCategoryLabels)),
);
const statusFilterOptions: ConnectionStatus[] = [
  "synced",
  "syncing",
  "idle",
  "error",
];

const filterChipClassName =
  "inline-flex items-center gap-1 rounded-md border bg-background px-2 py-1 text-[10px] font-medium text-muted-foreground transition-colors hover:bg-muted sm:text-xs";
const headerActionClassName = "h-9 min-w-[112px] justify-center gap-2";

export function Connections() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const { syncAll, isSyncing } = useWalletSyncAction();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [addConnectionOpen, setAddConnectionOpen] = useState(false);
  const [resumeSourceId, setResumeSourceId] = useState<string | null>(null);
  const [kindFilter, setKindFilter] = useState<string | "all">("all");
  const [statusFilter, setStatusFilter] = useState<ConnectionStatus | "all">(
    "all",
  );
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
  const filteredConnections = snapshot.connections.filter(
    (connection) =>
      (kindFilter === "all" ||
        connectionKindCategoryLabels[connection.kind] === kindFilter) &&
      (statusFilter === "all" || connection.status === statusFilter),
  );
  const hasActiveFilters = kindFilter !== "all" || statusFilter !== "all";
  const clearFilters = () => {
    setKindFilter("all");
    setStatusFilter("all");
  };

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
            className={headerActionClassName}
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
            className={headerActionClassName}
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

      <div className="rounded-xl border bg-card">
        <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5">
          <div className="flex flex-1 items-center gap-2">
            <span className="text-sm font-medium sm:text-base">
              Wallets and sources
            </span>
            <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
              {filteredConnections.length}
            </span>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className={cn(
                    "h-8 gap-1.5 sm:h-9 sm:gap-2",
                    statusFilter !== "all" && "border-primary",
                  )}
                  aria-label="Filter by status"
                >
                  <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                  <span className="hidden sm:inline">Status</span>
                  {statusFilter !== "all" && (
                    <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                  )}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-[180px]">
                <DropdownMenuLabel>Filter by status</DropdownMenuLabel>
                <DropdownMenuRadioGroup
                  value={statusFilter}
                  onValueChange={(value) =>
                    setStatusFilter(value as ConnectionStatus | "all")
                  }
                >
                  <DropdownMenuRadioItem value="all">
                    All statuses
                  </DropdownMenuRadioItem>
                  {statusFilterOptions.map((status) => (
                    <DropdownMenuRadioItem key={status} value={status}>
                      {status}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>

            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className={cn(
                    "h-8 gap-1.5 sm:h-9 sm:gap-2",
                    kindFilter !== "all" && "border-primary",
                  )}
                  aria-label="Filter by kind"
                >
                  <Wallet className="size-3.5 sm:size-4" aria-hidden="true" />
                  <span className="hidden sm:inline">Kind</span>
                  {kindFilter !== "all" && (
                    <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                  )}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-[200px]">
                <DropdownMenuLabel>Filter by kind</DropdownMenuLabel>
                <DropdownMenuRadioGroup
                  value={kindFilter}
                  onValueChange={(value) => setKindFilter(value)}
                >
                  <DropdownMenuRadioItem value="all">
                    All kinds
                  </DropdownMenuRadioItem>
                  {kindFilterOptions.map((kind) => (
                    <DropdownMenuRadioItem key={kind} value={kind}>
                      {kind}
                    </DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        {hasActiveFilters && (
          <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
            <span className="text-[10px] text-muted-foreground sm:text-xs">
              Filters:
            </span>
            {statusFilter !== "all" && (
              <button
                type="button"
                className={filterChipClassName}
                onClick={() => setStatusFilter("all")}
                aria-label={`Clear ${statusFilter} status filter`}
              >
                {statusFilter}
                <X className="size-2.5 sm:size-3" aria-hidden="true" />
              </button>
            )}
            {kindFilter !== "all" && (
              <button
                type="button"
                className={filterChipClassName}
                onClick={() => setKindFilter("all")}
                aria-label={`Clear ${kindFilter} kind filter`}
              >
                {kindFilter}
                <X className="size-2.5 sm:size-3" aria-hidden="true" />
              </button>
            )}
            <button
              type="button"
              onClick={clearFilters}
              className="text-[10px] text-destructive hover:underline sm:text-xs"
            >
              Clear all
            </button>
          </div>
        )}

        <div className="border-t">
          <div className="overflow-x-auto px-3 pb-3 pt-3 sm:px-6 sm:pb-4">
            <Table className="min-w-[920px]">
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead className="min-w-[280px] text-xs font-medium text-muted-foreground sm:text-sm">
                  Wallet/source
                </TableHead>
                <TableHead className="min-w-[140px] text-xs font-medium text-muted-foreground sm:text-sm">
                  Kind
                </TableHead>
                <TableHead className="min-w-[140px] text-xs font-medium text-muted-foreground sm:text-sm">
                  Last sync
                </TableHead>
                <TableHead className="hidden min-w-[220px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                  Composition
                </TableHead>
                <TableHead className="min-w-[150px] text-right text-xs font-medium text-muted-foreground sm:text-sm">
                  Balance
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filteredConnections.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={5}
                    className="h-24 text-center text-sm text-muted-foreground"
                  >
                    No wallets or sources match your filters.
                  </TableCell>
                </TableRow>
              ) : (
                filteredConnections.map((connection) => (
                  <ConnectionRow
                    key={connection.id}
                    connection={connection}
                    totalBtc={totalBtc}
                    priceEur={snapshot.priceEur}
                    hideSensitive={hideSensitive}
                    currency={currency}
                    onSelect={() => onSelect(connection.id)}
                  />
                ))
              )}
            </TableBody>
          </Table>
          </div>
        </div>
      </div>
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
  const metadataItems = [
    c.addresses != null ? `${c.addresses} addresses` : null,
    c.channels != null ? `${c.channels} channels` : null,
    c.gap != null ? `gap limit ${c.gap}` : null,
  ].filter(Boolean);

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
      className="cursor-pointer align-top hover:bg-muted/35"
      onClick={onSelect}
      onKeyDown={onKeyDown}
    >
      <TableCell className="min-w-[280px]">
        <div className="flex min-w-0 items-start gap-3">
          <span
            className={cn(
              "relative mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
              connectionKindTone(c.kind),
            )}
            aria-hidden="true"
          >
            <Wallet className="size-4" />
            <span
              className={cn(
                "absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2 ring-card",
                statusDotStyles[c.status],
              )}
            />
          </span>
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-foreground">
              {c.label}
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
              {metadataItems.map((item, index) => (
                <span key={item}>
                  {index > 0 ? "· " : ""}
                  {item}
                </span>
              ))}
            </div>
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant="outline" className="rounded-md">
          {connectionKindCategoryLabels[c.kind]}
        </Badge>
      </TableCell>
      <TableCell>
        <div className="grid gap-1">
          <span className="text-sm">{c.last}</span>
          <span
            className={cn(
              "w-fit rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
              connectionStatusStyles[c.status],
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
