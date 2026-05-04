import { BadgeCheck, ShieldCheck } from "lucide-react";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { useDaemon } from "@/daemon/client";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import {
  fmtCcy,
  formatBtc,
  formatEur,
  useCurrency,
} from "@/lib/currency";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import type {
  Connection,
  ConnectionKind,
  ConnectionStatus,
  OverviewSnapshot,
} from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

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

const statusCopy: Record<ConnectionStatus, string> = {
  synced: "Synced",
  syncing: "Syncing",
  idle: "No rows",
  error: "Needs review",
};

export function SourceFunds() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const currency = useCurrency();
  const hideSensitive = useUiStore((s) => s.hideSensitive);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading source balances...
      </div>
    );
  }

  const snapshot = data.data;
  const wallets = snapshot.connections;
  const totalBtc = wallets.reduce((sum, wallet) => sum + wallet.balance, 0);
  const walletsWithBalance = wallets.filter((wallet) => wallet.balance !== 0).length;
  const activeProfile = snapshot.status?.profile ?? "Active books";
  const needsJournals = Boolean(snapshot.status?.needsJournals);

  return (
    <div className="w-full space-y-4 bg-background p-3 sm:space-y-6 sm:p-4 md:p-6">
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2">
              <BadgeCheck className="size-4" aria-hidden="true" />
              Source of Funds
            </CardTitle>
            <CardDescription>
              Local source-of-funds summaries for selected wallet sources.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4">
            {wallets.length === 0 ? (
              <div className="rounded-md border border-dashed p-6 text-center text-sm text-muted-foreground">
                No wallet sources in these books yet.
              </div>
            ) : (
              wallets.map((wallet) => (
                <SourceWalletRow
                  key={wallet.id}
                  wallet={wallet}
                  totalBtc={totalBtc}
                  priceEur={snapshot.priceEur}
                  currency={currency}
                  hideSensitive={hideSensitive}
                />
              ))
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="border-b">
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="size-4" aria-hidden="true" />
              Local Review
            </CardTitle>
            <CardDescription>Prepared from local wallet state.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 p-4 text-sm">
            <div className="flex items-center justify-between rounded-md border px-3 py-2">
              <span className="text-muted-foreground">Sources</span>
              <span className="font-medium">
                {wallets.length.toLocaleString("en-US")} wallets
              </span>
            </div>
            <div className="flex items-center justify-between rounded-md border px-3 py-2">
              <span className="text-muted-foreground">With balance</span>
              <span className="font-medium">
                {walletsWithBalance.toLocaleString("en-US")}
              </span>
            </div>
            <div className="rounded-md border px-3 py-2">
              <span className="block text-muted-foreground">Total balance</span>
              <span
                className={cn(
                  "mt-1 block font-medium tabular-nums",
                  blurClass(hideSensitive),
                )}
              >
                <CurrencyToggleText>
                  {fmtCcy(totalBtc, currency, snapshot.priceEur)}
                </CurrencyToggleText>
              </span>
              <span
                className={cn(
                  "block text-xs text-muted-foreground tabular-nums",
                  blurClass(hideSensitive),
                )}
              >
                <CurrencyToggleText>
                  {currency === "eur"
                    ? formatBtc(totalBtc)
                    : formatEur(totalBtc, snapshot.priceEur)}
                </CurrencyToggleText>
              </span>
            </div>
            <div className="rounded-md border px-3 py-2">
              <span className="block text-muted-foreground">Books</span>
              <span className="mt-1 block truncate font-medium">
                {activeProfile}
              </span>
            </div>
            {needsJournals && (
              <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-amber-800 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-300">
                Journals need processing before report totals are trusted.
              </div>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

interface SourceWalletRowProps {
  wallet: Connection;
  totalBtc: number;
  priceEur: number;
  currency: "btc" | "eur";
  hideSensitive: boolean;
}

function SourceWalletRow({
  wallet,
  totalBtc,
  priceEur,
  currency,
  hideSensitive,
}: SourceWalletRowProps) {
  const percent = totalBtc > 0 ? (wallet.balance / totalBtc) * 100 : 0;
  const detail =
    wallet.addresses != null
      ? `${wallet.addresses.toLocaleString("en-US")} addresses`
      : wallet.channels != null
        ? `${wallet.channels.toLocaleString("en-US")} channels`
        : "Configured source";

  return (
    <div className="rounded-md border p-3">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <p className="truncate font-medium">{wallet.label}</p>
            <span className="rounded-md border px-2 py-1 text-xs text-muted-foreground">
              {kindLabels[wallet.kind]}
            </span>
            <span
              className={cn(
                "rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
                statusStyles[wallet.status],
              )}
            >
              {statusCopy[wallet.status]}
            </span>
          </div>
          <p className="text-xs text-muted-foreground">
            {detail} · last activity {wallet.last}
          </p>
        </div>
        <div className="shrink-0 text-left sm:text-right">
          <p
            className={cn(
              "font-mono text-sm font-semibold tabular-nums",
              blurClass(hideSensitive),
            )}
          >
            <CurrencyToggleText>
              {fmtCcy(wallet.balance, currency, priceEur)}
            </CurrencyToggleText>
          </p>
          <p
            className={cn(
              "font-mono text-xs text-muted-foreground tabular-nums",
              blurClass(hideSensitive),
            )}
          >
            <CurrencyToggleText>
              {currency === "eur"
                ? formatBtc(wallet.balance)
                : formatEur(wallet.balance, priceEur)}
            </CurrencyToggleText>
          </p>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2">
        <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-muted">
          <div
            className="absolute inset-y-0 left-0 rounded-full bg-primary transition-[width]"
            style={{ width: `${Math.max(0, Math.min(100, percent))}%` }}
          />
        </div>
        <span
          className={cn(
            "w-12 text-right text-xs text-muted-foreground tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {percent > 0 && percent < 0.1
            ? "<0.1%"
            : `${percent.toFixed(percent < 10 ? 1 : 0)}%`}
        </span>
      </div>
    </div>
  );
}
