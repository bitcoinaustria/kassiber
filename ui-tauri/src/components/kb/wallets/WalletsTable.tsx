import { type KeyboardEvent } from "react";
import { Wallet } from "lucide-react";

import { ConnectionStatusPill } from "@/components/kb/ConnectionStatusPill";
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
  connectionKindCategoryLabels,
  connectionKindTone,
} from "@/lib/connectionDisplay";
import type { Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { Connection } from "@/mocks/seed";

import {
  formatBtc,
  formatEur,
  hiddenSensitiveClassName,
  statusDotStyles,
} from "./format";

interface WalletsTableProps {
  connections: Connection[];
  currency: Currency;
  hideSensitive: boolean;
  onSelectConnection: (id: string) => void;
  priceEur: number;
  totalBtc: number;
}

export function WalletsTable({
  connections,
  currency,
  hideSensitive,
  onSelectConnection,
  priceEur,
  totalBtc,
}: WalletsTableProps) {
  return (
    <div className="border-t">
      <div className="overflow-x-auto px-3 pb-3 pt-3 sm:px-6 sm:pb-4">
        <Table>
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="min-w-[200px] text-xs font-medium text-muted-foreground sm:text-sm">
                Wallet/source
              </TableHead>
              <TableHead className="w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Kind
              </TableHead>
              <TableHead className="w-[110px] text-xs font-medium text-muted-foreground sm:text-sm">
                Last sync
              </TableHead>
              <TableHead className="hidden w-[120px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Composition
              </TableHead>
              <TableHead className="w-[140px] text-right text-xs font-medium text-muted-foreground sm:text-sm">
                Balance
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {connections.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={5}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  No wallets or sources match your filters.
                </TableCell>
              </TableRow>
            ) : (
              connections.map((connection) => (
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
  const pct = totalBtc > 0 ? (connection.balance / totalBtc) * 100 : 0;
  const isEur = currency === "eur";
  const metadataItems = [
    connection.addresses != null ? `${connection.addresses} addresses` : null,
    connection.channels != null ? `${connection.channels} channels` : null,
    connection.gap != null ? `gap limit ${connection.gap}` : null,
  ].filter(Boolean);
  const compositionTitle = hideSensitive
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
          <span
            className={cn(
              "relative mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
              connectionKindTone(connection.kind),
            )}
            aria-hidden="true"
          >
            <Wallet className="size-4" />
            <span
              className={cn(
                "absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2 ring-card",
                statusDotStyles[connection.status],
              )}
            />
          </span>
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
          {connectionKindCategoryLabels[connection.kind]}
        </Badge>
      </TableCell>
      <TableCell>
        <div className="flex flex-col gap-1 text-sm whitespace-nowrap">
          <span>{connection.last}</span>
          <ConnectionStatusPill status={connection.status} />
        </div>
      </TableCell>
      <TableCell className="hidden lg:table-cell">
        <div
          className="relative h-2 overflow-hidden rounded-full bg-muted"
          title={compositionTitle}
        >
          <div
            className="absolute inset-y-0 left-0 rounded-full bg-primary transition-[width]"
            style={{ width: `${Math.max(1.5, pct)}%` }}
          />
        </div>
      </TableCell>
      <TableCell className="text-right">
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
      </TableCell>
    </TableRow>
  );
}
