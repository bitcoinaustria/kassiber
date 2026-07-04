import {
  useEffect,
  useMemo,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type UIEvent,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import i18n from "@/i18n";
import {
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  ArrowUpDown,
  Coins,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import { openExternalUrl } from "@/daemon/transport";
import { CopyButton } from "@/components/kb/CopyButton";
import { CountBadge } from "@/components/kb/CountBadge";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
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
  transaction_id?: string;
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

export interface WalletUtxoTotal {
  asset: string;
  amount: number | string;
  amount_sat: number;
  amount_msat: number;
}

export interface WalletUtxosData {
  wallet: {
    id: string;
    label: string;
  } | null;
  utxos: WalletUtxoRow[];
  totals: WalletUtxoTotal[];
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
    returned_count?: number;
    truncated?: boolean;
    row_limit?: number | null;
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
  onOpenTransaction?: (transactionId: string) => void;
}

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
type SortableUtxoColumn = "outpoint" | "amount" | "status" | "confirmed";

// Reveal rows incrementally so wallets with hundreds of coins stay responsive;
// the header total stays accurate because the server reports the full set.
export const UTXO_PAGE_SIZE = 50;
const WALLET_TABLE_VIEWPORT_CLASS =
  "max-h-[clamp(18rem,36dvh,28rem)] min-h-0 overflow-auto";

const UTXO_COLUMN_SORTS: Record<
  SortableUtxoColumn,
  { asc: UtxoSortValue; desc: UtxoSortValue; default: UtxoSortValue }
> = {
  outpoint: {
    asc: "outpoint-asc",
    desc: "outpoint-desc",
    default: "outpoint-asc",
  },
  amount: {
    asc: "size-asc",
    desc: "size-desc",
    default: "size-desc",
  },
  status: {
    asc: "confirmations-asc",
    desc: "confirmations-desc",
    default: "confirmations-desc",
  },
  confirmed: {
    asc: "date-asc",
    desc: "date-desc",
    default: "date-desc",
  },
};

function nextSortForColumn(sort: UtxoSortValue, column: SortableUtxoColumn) {
  const options = UTXO_COLUMN_SORTS[column];
  if (sort === options.default) {
    return options.default === options.desc ? options.asc : options.desc;
  }
  if (sort === options.asc || sort === options.desc) return "default";
  return options.default;
}

function directionForColumn(sort: UtxoSortValue, column: SortableUtxoColumn) {
  const options = UTXO_COLUMN_SORTS[column];
  if (sort === options.asc) return "ascending";
  if (sort === options.desc) return "descending";
  return null;
}

function formatAmountText(value: number | string) {
  const amount = Number(value);
  return Number.isFinite(amount) ? amount.toFixed(8) : String(value);
}

// Bitcoin renders with the ₿ glyph as its unit (house style, matches the rest
// of the app). Liquid and other assets keep an explicit ticker so L-BTC is
// never shown as on-chain ₿, and the ticker is rendered exactly once.
function AmountDisplay({
  value,
  asset,
  hideSensitive,
  className,
}: {
  value: number | string;
  asset: string;
  hideSensitive?: boolean;
  className?: string;
}) {
  const text = formatAmountText(value);
  const isBtc = asset === "BTC";
  return (
    <span className={cn("inline-flex items-baseline gap-1", className)}>
      <span className={cn("tabular-nums", hideSensitive && "sensitive")}>
        {isBtc ? `₿ ${text}` : text}
      </span>
      {isBtc ? null : (
        <span className="text-xs font-normal text-muted-foreground">{asset}</span>
      )}
    </span>
  );
}

function formatOutpoint(value: string) {
  const [txid, vout] = value.split(":");
  if (!txid) return value;
  return `${txid.slice(0, 8)}…${txid.slice(-6)}:${vout ?? "?"}`;
}

function formatAddressPreview(value: string) {
  return value.length <= 24 ? value : `${value.slice(0, 12)}…${value.slice(-8)}`;
}

function formatLocation(
  row: WalletUtxoRow,
  t: TFunction<"connections">,
) {
  if (row.branch_label && row.address_index !== null && row.address_index !== undefined) {
    return t("utxos.locationLabel", {
      branch: row.branch_label,
      index: row.address_index,
    });
  }
  return row.address_label || row.branch_label || t("utxos.watchTarget");
}

function statusLabel(row: WalletUtxoRow, t: TFunction<"connections">) {
  if (row.confirmation_status === "confirmed") {
    const confirmations = row.confirmations;
    return confirmations
      ? t("utxos.confLabel", { count: confirmations })
      : t("utxos.confirmedLabel");
  }
  return t("utxos.mempool");
}

function isMempool(row: WalletUtxoRow) {
  return row.confirmation_status !== "confirmed";
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

// The Status column already conveys confirmed/mempool state, so the date column
// shows only the block confirmation time (or "—" when still unconfirmed).
function primaryDateLabel(row: WalletUtxoRow) {
  return row.block_time ? dateLabel(row.block_time) : null;
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

export function explorerButtonTitle(
  target: ExplorerTarget,
  t?: TFunction<"connections">,
) {
  const translate = t ?? ((key: string, opts?: Record<string, unknown>) =>
    // dynamic key
    i18n.t(key as never, { ns: "connections", ...opts }) as string);
  return translate("utxos.openExplorer", { explorer: target.label });
}

function localTransactionTitle(
  row: WalletUtxoRow,
  t: TFunction<"connections">,
) {
  return t("utxos.openLocalTransaction", {
    outpoint: formatOutpoint(row.outpoint),
  });
}

export function transactionRefForRow(row: WalletUtxoRow) {
  return row.transaction_id || row.txid;
}

function explorerOpenErrorMessage(
  error: unknown,
  t: TFunction<"connections">,
) {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === "string" && error) return error;
  return t("utxos.dialog.couldNotOpen");
}

function OutpointButton({
  row,
  explorer,
  hideSensitive,
  onOpen,
  onOpenTransaction,
  t,
}: {
  row: WalletUtxoRow;
  explorer: ExplorerTarget | null;
  hideSensitive: boolean;
  onOpen: (row: WalletUtxoRow) => void;
  onOpenTransaction?: (transactionId: string) => void;
  t: TFunction<"connections">;
}) {
  if (onOpenTransaction) {
    return (
      <button
        type="button"
        className={cn(
          "inline-flex max-w-[22ch] items-center gap-1 truncate text-left font-mono text-xs underline-offset-4 hover:underline",
          hideSensitive && "sensitive",
        )}
        title={localTransactionTitle(row, t)}
        onKeyDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          onOpenTransaction(transactionRefForRow(row));
        }}
      >
        <span className="truncate">{formatOutpoint(row.outpoint)}</span>
      </button>
    );
  }
  if (!explorer) {
    return (
      <span className={cn("font-mono text-xs", hideSensitive && "sensitive")}>
        {formatOutpoint(row.outpoint)}
      </span>
    );
  }
  return (
    <button
      type="button"
      className={cn(
        "inline-flex max-w-[22ch] items-center gap-1 truncate text-left font-mono text-xs underline-offset-4 hover:underline",
        hideSensitive && "sensitive",
      )}
      title={explorerButtonTitle(explorer, t)}
      onKeyDown={(event) => event.stopPropagation()}
      onClick={(event) => {
        event.stopPropagation();
        onOpen(row);
      }}
    >
      <span className="truncate">{formatOutpoint(row.outpoint)}</span>
    </button>
  );
}

function LocationBlock({
  row,
  hideSensitive,
  t,
}: {
  row: WalletUtxoRow;
  hideSensitive: boolean;
  t: TFunction<"connections">;
}) {
  return (
    <div className="min-w-0">
      <div className="truncate text-sm">{formatLocation(row, t)}</div>
      {row.address ? (
        <div className="flex min-w-0 items-center gap-1">
          <span
            className={cn(
              "truncate font-mono text-xs text-muted-foreground",
              hideSensitive && "sensitive",
            )}
          >
            {formatAddressPreview(row.address)}
          </span>
          <span
            onClick={(event) => event.stopPropagation()}
            onKeyDown={(event) => event.stopPropagation()}
          >
            <CopyButton
              value={row.address}
              ariaLabel={t("utxos.copyAddress")}
              variant="ghost"
              className="size-5 shrink-0 text-muted-foreground"
            />
          </span>
        </div>
      ) : null}
    </div>
  );
}

function SortableTableHead({
  children,
  column,
  sort,
  onSort,
  className,
}: {
  children: ReactNode;
  column: SortableUtxoColumn;
  sort: UtxoSortValue;
  onSort: (column: SortableUtxoColumn) => void;
  className?: string;
}) {
  const direction = directionForColumn(sort, column);
  const Icon =
    direction === "ascending"
      ? ArrowUp
      : direction === "descending"
        ? ArrowDown
        : ArrowUpDown;
  return (
    <TableHead aria-sort={direction ?? "none"} className={className}>
      <button
        type="button"
        className={cn(
          "inline-flex h-8 items-center gap-1.5 rounded-sm text-left transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          direction ? "text-foreground" : "text-muted-foreground",
          className?.includes("text-right") && "ml-auto",
        )}
        onClick={() => onSort(column)}
      >
        <span>{children}</span>
        <Icon className="size-3.5" aria-hidden="true" />
      </button>
    </TableHead>
  );
}

function UtxoExplorerOpenDialog({
  row,
  target,
  onRowChange,
  t,
}: {
  row: WalletUtxoRow | null;
  target: ExplorerTarget | null;
  onRowChange: (row: WalletUtxoRow | null) => void;
  t: TFunction<["connections", "common"]>;
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
      setOpenError(explorerOpenErrorMessage(error, t));
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
            <DialogTitle>{t("utxos.dialog.title")}</DialogTitle>
            <DialogDescription className="max-w-prose">
              {t("utxos.dialog.description", {
                explorer: target?.label ?? t("utxos.dialog.explorerFallback"),
              })}
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
                {t("common:actions.cancel")}
              </Button>
            </DialogClose>
            <Button
              type="button"
              disabled={!target || opening}
              onClick={() => void openExplorer()}
            >
              {opening ? t("utxos.dialog.opening") : t("utxos.dialog.openExplorer")}
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
  t,
}: {
  title: string;
  body: string;
  onRefresh: () => void;
  isRefreshing: boolean;
  t: TFunction<"connections">;
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
        {/* Reserve the widest label's width so the spinning icon can't ghost
            on a mid-refresh resize: WKWebView (the macOS webview) strands the
            old composited tile of the GPU-promoted icon when the button
            changes width. Stacking both labels pins the width in every locale. */}
        <span className="grid justify-items-center">
          <span aria-hidden="true" className="invisible col-start-1 row-start-1">
            {t("utxos.refresh")}
          </span>
          <span aria-hidden="true" className="invisible col-start-1 row-start-1">
            {t("utxos.refreshing")}
          </span>
          <span className="col-start-1 row-start-1">
            {isRefreshing ? t("utxos.refreshing") : t("utxos.refresh")}
          </span>
        </span>
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
  onOpenTransaction,
}: UtxosInventoryPanelProps) {
  const { t } = useTranslation(["connections", "common"]);
  const rows = inventory?.utxos ?? [];
  const walletId = inventory?.wallet?.id ?? null;
  const totalCount = inventory?.summary?.count ?? inventory?.freshness.active_count ?? rows.length;
  const returnedCount = inventory?.summary?.returned_count ?? rows.length;
  const serverTruncated = Boolean(inventory?.summary?.truncated);
  const [sort, setSort] = useState<UtxoSortValue>("default");
  const [explorerRow, setExplorerRow] = useState<WalletUtxoRow | null>(null);
  const [visibleCount, setVisibleCount] = useState(UTXO_PAGE_SIZE);
  // Collapse back to the first page when switching wallets.
  useEffect(() => {
    setVisibleCount(UTXO_PAGE_SIZE);
  }, [walletId, sort]);
  const sortedRows = useMemo(
    () => sortUtxosForDisplay(rows, sort),
    [rows, sort],
  );
  const handleSort = (column: SortableUtxoColumn) => {
    setSort((current) => nextSortForColumn(current, column));
  };
  const openRowTransaction = (row: WalletUtxoRow, explorer: ExplorerTarget | null) => {
    if (onOpenTransaction) {
      onOpenTransaction(transactionRefForRow(row));
      return;
    }
    if (explorer) setExplorerRow(row);
  };
  const openRowOnKeyboard = (
    event: KeyboardEvent<HTMLElement>,
    row: WalletUtxoRow,
    explorer: ExplorerTarget | null,
  ) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    openRowTransaction(row, explorer);
  };
  const revealMoreRows = () => {
    setVisibleCount((count) =>
      Math.min(sortedRows.length, count + UTXO_PAGE_SIZE),
    );
  };
  const handleRowsScroll = (event: UIEvent<HTMLDivElement>) => {
    if (visibleCount >= sortedRows.length) return;
    const target = event.currentTarget;
    const distanceFromBottom =
      target.scrollHeight - target.scrollTop - target.clientHeight;
    if (distanceFromBottom < 96) revealMoreRows();
  };
  const visibleRows = sortedRows.slice(0, visibleCount);
  const explorerTarget = explorerRow
    ? explorerTargetForUtxo(explorerRow, explorerSettings)
    : null;
  const stale = Boolean(inventory?.freshness.stale);
  const unsupported = inventory?.support.supported === false;
  const liquidBlocked = inventory?.support.status === "liquid_unblind_blocked";
  const title = liquidBlocked
    ? t("utxos.liquidNeedsUnblinding")
    : t("utxos.unavailableTitle");
  const lastSyncedLabel = dateLabel(
    inventory?.freshness.last_synced_at ?? inventory?.freshness.last_seen_at,
  );

  return (
    <Card>
      <CardHeader className="border-b px-4 py-2.5">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
              <Coins className="size-4" aria-hidden="true" />
              {t("utxos.title")}
              <CountBadge>
                {serverTruncated
                  ? t("utxos.countOf", {
                      returned: returnedCount.toLocaleString("en-US"),
                      total: totalCount.toLocaleString("en-US"),
                    })
                : totalCount.toLocaleString("en-US")}
              </CountBadge>
            </CardTitle>
          </div>
          <Button
            type="button"
            variant="outline"
            size="icon-xs"
            className="shrink-0 self-start"
            disabled={isRefreshing}
            aria-label={isRefreshing ? t("utxos.refreshing") : t("utxos.refresh")}
            title={isRefreshing ? t("utxos.refreshing") : t("utxos.refresh")}
            onClick={onRefresh}
          >
            <RefreshCw
              className={cn("size-3", isRefreshing && "animate-spin")}
              aria-hidden="true"
            />
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
            title={t("utxos.couldNotLoadTitle")}
            body={errorMessage}
            onRefresh={onRefresh}
            isRefreshing={isRefreshing}
            t={t}
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
                    t("utxos.unsupportedFallback")}
                </p>
              </div>
            </div>
          </div>
        ) : rows.length === 0 ? (
          <EmptyState
            title={t("utxos.noneTitle")}
            body={t("utxos.noneBody")}
            onRefresh={onRefresh}
            isRefreshing={isRefreshing}
            t={t}
          />
        ) : (
          <>
            <div className={WALLET_TABLE_VIEWPORT_CLASS} onScroll={handleRowsScroll}>
              {stale ? (
                <div className="flex items-start gap-2 border-b bg-amber-50 px-4 py-2.5 text-xs text-amber-800 dark:bg-amber-950/30 dark:text-amber-200">
                  <AlertTriangle
                    className="mt-0.5 size-3.5 shrink-0"
                    aria-hidden="true"
                  />
                  <span>
                    {lastSyncedLabel
                      ? t("utxos.staleWithDate", { date: lastSyncedLabel })
                      : t("utxos.stale")}
                  </span>
                </div>
              ) : null}
              {/* Desktop: dense columnar table for scanning amounts/confirmations. */}
              <div className="hidden sm:block">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <SortableTableHead
                        column="outpoint"
                        sort={sort}
                        onSort={handleSort}
                      >
                        {t("utxos.column.outpoint")}
                      </SortableTableHead>
                      <SortableTableHead
                        column="amount"
                        sort={sort}
                        onSort={handleSort}
                      >
                        {t("utxos.column.amount")}
                      </SortableTableHead>
                      <SortableTableHead
                        column="status"
                        sort={sort}
                        onSort={handleSort}
                      >
                        {t("utxos.column.status")}
                      </SortableTableHead>
                      <TableHead>{t("utxos.column.location")}</TableHead>
                      <SortableTableHead
                        column="confirmed"
                        sort={sort}
                        onSort={handleSort}
                        className="text-right"
                      >
                        {t("utxos.column.confirmed")}
                      </SortableTableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {visibleRows.map((row) => {
                      const explorer = explorerTargetForUtxo(
                        row,
                        explorerSettings,
                      );
                      return (
                        <TableRow
                          key={row.id || row.outpoint}
                          role="button"
                          tabIndex={0}
                          className={cn(
                            "cursor-pointer hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                            isMempool(row) && "bg-muted/20",
                          )}
                          onClick={() => openRowTransaction(row, explorer)}
                          onKeyDown={(event) =>
                            openRowOnKeyboard(event, row, explorer)
                          }
                        >
                          <TableCell>
                            <div className="flex items-center gap-1">
                              <OutpointButton
                                row={row}
                                explorer={explorer}
                                hideSensitive={hideSensitive}
                                onOpen={setExplorerRow}
                                onOpenTransaction={onOpenTransaction}
                                t={t}
                              />
                              <span
                                onClick={(event) => event.stopPropagation()}
                                onKeyDown={(event) => event.stopPropagation()}
                              >
                                <CopyButton
                                  value={row.outpoint}
                                  ariaLabel={t("utxos.copyOutpoint")}
                                  variant="ghost"
                                  className="size-5 shrink-0 text-muted-foreground"
                                />
                              </span>
                            </div>
                          </TableCell>
                          <TableCell>
                            <AmountDisplay
                              value={row.amount}
                              asset={row.asset}
                              hideSensitive={hideSensitive}
                              className="font-medium"
                            />
                          </TableCell>
                          <TableCell>
                            <Badge
                              variant={
                                row.confirmation_status === "confirmed"
                                  ? "secondary"
                                  : "outline"
                              }
                            >
                              {statusLabel(row, t)}
                            </Badge>
                          </TableCell>
                          <TableCell className="max-w-[220px]">
                            <LocationBlock
                              row={row}
                              hideSensitive={hideSensitive}
                              t={t}
                            />
                          </TableCell>
                          <TableCell className="text-right font-mono text-xs text-muted-foreground">
                            {primaryDateLabel(row) ?? "—"}
                          </TableCell>
                        </TableRow>
                      );
                    })}
                  </TableBody>
                </Table>
              </div>
              {/* Mobile: stacked rows, matching the Recent transactions layout. */}
              <div className="divide-y sm:hidden">
                {visibleRows.map((row) => {
                  const explorer = explorerTargetForUtxo(row, explorerSettings);
                  return (
                    <div
                      key={row.id || row.outpoint}
                      className={cn(
                        "flex cursor-pointer flex-col gap-2 px-4 py-3 hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                        isMempool(row) && "bg-muted/20",
                      )}
                      role="button"
                      tabIndex={0}
                      onClick={() => openRowTransaction(row, explorer)}
                      onKeyDown={(event) =>
                        openRowOnKeyboard(event, row, explorer)
                      }
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex min-w-0 items-center gap-1">
                          <OutpointButton
                            row={row}
                            explorer={explorer}
                            hideSensitive={hideSensitive}
                            onOpen={setExplorerRow}
                            onOpenTransaction={onOpenTransaction}
                            t={t}
                          />
                          <span
                            onClick={(event) => event.stopPropagation()}
                            onKeyDown={(event) => event.stopPropagation()}
                          >
                            <CopyButton
                              value={row.outpoint}
                              ariaLabel={t("utxos.copyOutpoint")}
                              variant="ghost"
                              className="size-5 shrink-0 text-muted-foreground"
                            />
                          </span>
                        </div>
                        <Badge
                          variant={
                            row.confirmation_status === "confirmed"
                              ? "secondary"
                              : "outline"
                          }
                          className="shrink-0"
                        >
                          {statusLabel(row, t)}
                        </Badge>
                      </div>
                      <div className="flex items-baseline justify-between gap-3">
                        <AmountDisplay
                          value={row.amount}
                          asset={row.asset}
                          hideSensitive={hideSensitive}
                          className="text-sm font-medium"
                        />
                        <span className="shrink-0 font-mono text-xs text-muted-foreground">
                          {primaryDateLabel(row) ?? "—"}
                        </span>
                      </div>
                      <LocationBlock
                        row={row}
                        hideSensitive={hideSensitive}
                        t={t}
                      />
                    </div>
                  );
                })}
              </div>
            </div>
            {visibleRows.length < sortedRows.length || serverTruncated ? (
              <div className="border-t px-4 py-2.5 text-xs text-muted-foreground">
                {t("utxos.loadedRows", {
                  loaded: (serverTruncated
                    ? returnedCount
                    : visibleRows.length
                  ).toLocaleString("en-US"),
                  total: (serverTruncated
                    ? totalCount
                    : sortedRows.length
                  ).toLocaleString("en-US"),
                })}
              </div>
            ) : null}
            <UtxoExplorerOpenDialog
              row={explorerRow}
              target={explorerTarget}
              onRowChange={setExplorerRow}
              t={t}
            />
          </>
        )}
      </CardContent>
    </Card>
  );
}
