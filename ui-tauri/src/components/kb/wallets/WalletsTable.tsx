import { useMemo, useState, type KeyboardEvent } from "react";
import {
  ArrowDown,
  ArrowUp,
  ChevronsUpDown,
  type LucideIcon,
} from "lucide-react";

import { ConnectionAssetBadge } from "@/components/kb/ConnectionAssetBadge";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  connectionCategoryLabel,
  connectionCategorySortRank,
  connectionTypeLabel,
} from "@/lib/connectionDisplay";
import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { Connection } from "@/mocks/seed";

import { formatBtc, formatEur, hiddenSensitiveClassName } from "./format";

interface WalletsTableProps {
  connections: Connection[];
  currency: Currency;
  hideSensitive: boolean;
  onSelectConnection: (id: string) => void;
  priceEur: number;
  totalBtc: number;
  /** Unfiltered wallet count, to distinguish empty book from empty filter. */
  totalCount?: number;
}

type SortKey = "label" | "kind" | "transactions" | "last" | "balance";
type SortDir = "asc" | "desc";

const collator = new Intl.Collator(undefined, {
  sensitivity: "base",
  numeric: true,
});

/** First click on a column starts in the most useful direction. */
const defaultSortDir: Record<SortKey, SortDir> = {
  label: "asc",
  kind: "asc",
  transactions: "desc",
  last: "desc",
  balance: "desc",
};

function activityMillis(connection: Connection): number | null {
  if (!connection.lastTransactionAt) return null;
  const ms = Date.parse(connection.lastTransactionAt);
  return Number.isNaN(ms) ? null : ms;
}

function activityLabel(connection: Connection): string {
  const ms = activityMillis(connection);
  if (ms === null) return "never";
  const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (diffSec < 60) return "just now";
  const minutes = Math.floor(diffSec / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(ms).toISOString().slice(0, 10);
}

/** Ascending comparator per key; a connection without activity counts as oldest. */
function compareBy(a: Connection, b: Connection, key: SortKey): number {
  switch (key) {
    case "label":
      return collator.compare(a.label, b.label);
    case "kind":
      return (
        connectionCategorySortRank(a) - connectionCategorySortRank(b) ||
        collator.compare(connectionCategoryLabel(a), connectionCategoryLabel(b)) ||
        collator.compare(connectionTypeLabel(a), connectionTypeLabel(b)) ||
        collator.compare(a.label, b.label)
      );
    case "transactions":
      return (a.transactionCount ?? 0) - (b.transactionCount ?? 0);
    case "last": {
      const ma = activityMillis(a);
      const mb = activityMillis(b);
      if (ma === mb) return 0;
      if (ma === null) return -1;
      if (mb === null) return 1;
      return ma - mb;
    }
    case "balance":
      return a.balance - b.balance;
  }
}

export function WalletsTable({
  connections,
  currency,
  hideSensitive,
  onSelectConnection,
  priceEur,
  totalBtc,
  totalCount,
}: WalletsTableProps) {
  const [sortKey, setSortKey] = useState<SortKey | null>("kind");
  const [sortDir, setSortDir] = useState<SortDir>("asc");

  const sortedConnections = useMemo(() => {
    if (!sortKey) return connections;
    const factor = sortDir === "asc" ? 1 : -1;
    return [...connections].sort((a, b) => compareBy(a, b, sortKey) * factor);
  }, [connections, sortKey, sortDir]);

  const onSort = (key: SortKey) => {
    if (key === sortKey) {
      setSortDir((dir) => (dir === "asc" ? "desc" : "asc"));
    } else {
      setSortKey(key);
      setSortDir(defaultSortDir[key]);
    }
  };

  return (
    <div className="border-t">
      <div className="overflow-x-auto px-3 pb-3 pt-3 sm:px-6 sm:pb-4">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <SortableHead
                label="Connection"
                sortKey="label"
                activeKey={sortKey}
                dir={sortDir}
                onSort={onSort}
                className="min-w-[200px]"
              />
              <SortableHead
                label="Kind"
                sortKey="kind"
                activeKey={sortKey}
                dir={sortDir}
                onSort={onSort}
                className="w-[100px]"
              />
              <SortableHead
                label="Transactions"
                sortKey="transactions"
                activeKey={sortKey}
                dir={sortDir}
                onSort={onSort}
                align="right"
                className="w-[120px] text-right"
              />
              <SortableHead
                label="Last activity"
                sortKey="last"
                activeKey={sortKey}
                dir={sortDir}
                onSort={onSort}
                className="w-[110px]"
              />
              <TableHead className="hidden w-[120px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Composition
              </TableHead>
              <SortableHead
                label="Balance"
                sortKey="balance"
                activeKey={sortKey}
                dir={sortDir}
                onSort={onSort}
                align="right"
                className="w-[140px] text-right"
              />
            </TableRow>
          </TableHeader>
          <TableBody>
            {sortedConnections.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={6}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  {totalCount === 0
                    ? "No wallets yet — add a wallet to start syncing or importing history."
                    : "No wallets or sources match your filters."}
                </TableCell>
              </TableRow>
            ) : (
              sortedConnections.map((connection) => (
                <WalletRow
                  key={connection.id}
                  connection={connection}
                  totalBtc={totalBtc}
                  priceEur={priceEur}
                  hideSensitive={hideSensitive}
                  currency={currency}
                  onSelect={() => onSelectConnection(connection.id)}
                />
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

interface SortableHeadProps {
  label: string;
  sortKey: SortKey;
  activeKey: SortKey | null;
  dir: SortDir;
  onSort: (key: SortKey) => void;
  align?: "left" | "right";
  className?: string;
}

function SortableHead({
  label,
  sortKey,
  activeKey,
  dir,
  onSort,
  align = "left",
  className,
}: SortableHeadProps) {
  const active = activeKey === sortKey;
  const Icon: LucideIcon = active
    ? dir === "asc"
      ? ArrowUp
      : ArrowDown
    : ChevronsUpDown;

  return (
    <TableHead
      className={className}
      aria-sort={active ? (dir === "asc" ? "ascending" : "descending") : "none"}
    >
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className={cn(
          "group inline-flex w-full items-center gap-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground sm:text-sm",
          align === "right" && "flex-row-reverse",
        )}
      >
        <span className="truncate">{label}</span>
        <Icon
          aria-hidden="true"
          className={cn(
            "size-3.5 shrink-0 transition-opacity",
            active
              ? "text-foreground opacity-100"
              : "opacity-0 group-hover:opacity-60",
          )}
        />
      </button>
    </TableHead>
  );
}

interface WalletRowProps {
  connection: Connection;
  currency: Currency;
  hideSensitive: boolean;
  onSelect: () => void;
  priceEur: number;
  totalBtc: number;
}

function WalletRow({
  connection,
  currency,
  hideSensitive,
  onSelect,
  priceEur,
  totalBtc,
}: WalletRowProps) {
  const isBackend = connection.role === "backend";
  const pct = totalBtc > 0 ? (connection.balance / totalBtc) * 100 : 0;
  const isEur = currency === "eur";
  const metadataItems = [
    isBackend ? connection.endpoint : null,
    isBackend && connection.isDefaultBackend ? "default backend" : null,
    isBackend ? "first-party infra" : null,
    connection.addresses != null ? `${connection.addresses} addresses` : null,
    connection.channels != null ? `${connection.channels} channels` : null,
    connection.gap != null ? `gap limit ${connection.gap}` : null,
    connection.deprecated ? "deprecated" : null,
  ].filter(Boolean);
  const compositionTitle = isBackend
    ? "First-party infrastructure"
    : hideSensitive
    ? "Wallet share hidden"
    : pct < 0.1
      ? "<0.1% of total balance"
      : `${pct.toFixed(pct < 10 ? 1 : 0)}% of total balance`;

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
      <TableCell className="min-w-[200px]">
        <div className="flex min-w-0 items-start gap-3">
          <ConnectionAssetBadge
            connection={connection}
            className="mt-0.5"
          />
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-foreground">
              {connection.label}
            </div>
            {metadataItems.length > 0 ? (
              <div className="mt-1 truncate text-[10px] text-muted-foreground sm:text-xs">
                {metadataItems.join(" · ")}
              </div>
            ) : null}
          </div>
        </div>
      </TableCell>
      <TableCell>
        <Badge variant="outline" className="rounded-md whitespace-nowrap">
          {connectionCategoryLabel(connection)}
        </Badge>
      </TableCell>
      <TableCell className="text-right">
        {isBackend ? (
          <span className="text-sm text-muted-foreground">—</span>
        ) : (
          <span className="text-sm font-medium tabular-nums">
            {(connection.transactionCount ?? 0).toLocaleString()}
          </span>
        )}
      </TableCell>
      <TableCell>
        <div className="text-sm whitespace-nowrap">
          <span>{activityLabel(connection)}</span>
        </div>
      </TableCell>
      <TableCell className="hidden lg:table-cell">
        {isBackend ? (
          <div
            className="text-xs text-muted-foreground"
            title={compositionTitle}
          >
            {connection.syncSource ?? "Backend endpoint"}
          </div>
        ) : (
          <div
            className="relative h-2 overflow-hidden rounded-full bg-muted"
            title={compositionTitle}
          >
            <div
              className="absolute inset-y-0 left-0 rounded-full bg-primary transition-[width]"
              style={{ width: `${Math.max(1.5, pct)}%` }}
            />
          </div>
        )}
      </TableCell>
      <TableCell className="text-right">
        {isBackend ? (
          <>
            <div className="font-medium tabular-nums">
              {connection.isDefaultBackend ? "Default" : "Configured"}
            </div>
            <div className="text-xs text-muted-foreground tabular-nums">
              {connection.walletRefs?.length
                ? `${connection.walletRefs.length.toLocaleString()} wallets`
                : connection.backendKind}
            </div>
          </>
        ) : (
          <>
            <div
              className={cn(
                "font-medium tabular-nums",
                hiddenSensitiveClassName(hideSensitive),
              )}
            >
              <CurrencyToggleText>
                {isEur
                  ? formatEur(connection.balance * priceEur)
                  : `₿ ${formatBtc(connection.balance)}`}
              </CurrencyToggleText>
            </div>
            <div
              className={cn(
                "text-xs text-muted-foreground tabular-nums",
                hiddenSensitiveClassName(hideSensitive),
              )}
            >
              <CurrencyToggleText>
                {isEur
                  ? `₿ ${formatBtc(connection.balance)}`
                  : formatEur(connection.balance * priceEur)}
              </CurrencyToggleText>
            </div>
          </>
        )}
      </TableCell>
    </TableRow>
  );
}
