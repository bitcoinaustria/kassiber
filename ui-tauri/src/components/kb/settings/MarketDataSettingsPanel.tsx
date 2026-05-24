import * as React from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Database,
  ExternalLink,
  FileInput,
  RefreshCw,
  Upload,
  XCircle,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  formatCount,
  formatKrakenRange,
  KRAKEN_MARKET_DATA_BLOG_URL,
  KRAKEN_OHLCVT_SUPPORT_URL,
  rateRebuildJournalError,
  rateRebuildTransactionProgress,
  type Backend,
  type KrakenRatesImportData,
  type KrakenRatesImportOperation,
  type RateRebuildData,
} from "./SettingsModel";

export function MarketDataSettingsPanel({ backends }: { backends: Backend[] }) {
  const rateBackends = backends.filter((backend) => backend.net === "FX");
  const importKrakenRates = useDaemonMutation<KrakenRatesImportData>(
    "ui.rates.kraken_csv.import",
  );
  const rebuildRates = useDaemonMutation<RateRebuildData>("ui.rates.rebuild");
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const rebuildNoticeRef = React.useRef<string | null>(null);
  const [krakenArchivePath, setKrakenArchivePath] = React.useState("");
  const [krakenImportResult, setKrakenImportResult] =
    React.useState<KrakenRatesImportData | null>(null);
  const [krakenImportError, setKrakenImportError] = React.useState<string | null>(
    null,
  );
  const [pendingKrakenOperation, setPendingKrakenOperation] =
    React.useState<KrakenRatesImportOperation | null>(null);
  const [pendingBundledKrakenImport, setPendingBundledKrakenImport] =
    React.useState(false);
  const [rateRebuildOpen, setRateRebuildOpen] = React.useState(false);
  const [rateRebuildResult, setRateRebuildResult] =
    React.useState<RateRebuildData | null>(null);
  const [rateRebuildError, setRateRebuildError] = React.useState<string | null>(
    null,
  );
  const openMarketDataUrl = React.useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>, url: string) => {
      event.preventDefault();
      void openExternalUrl(url).catch((error) => {
        addNotification({
          title: "Could not open link",
          body:
            error instanceof Error
              ? error.message
              : "Could not open the link in the default browser.",
          tone: "warning",
        });
      });
    },
    [addNotification],
  );

  const chooseKrakenArchive = async () => {
    setKrakenImportError(null);
    const selected = await pickFile({
      title: "Choose Kraken OHLCVT CSV or ZIP",
      filters: [
        {
          name: "Kraken OHLCVT",
          extensions: ["zip", "csv"],
        },
      ],
    });
    if (selected) {
      setKrakenArchivePath(selected);
    }
  };

  const chooseKrakenDirectory = async () => {
    setKrakenImportError(null);
    const selected = await pickFile({
      title: "Choose extracted Kraken OHLCVT folder",
      directory: true,
    });
    if (selected) {
      setKrakenArchivePath(selected);
    }
  };

  const startKrakenImport = async (
    operation: KrakenRatesImportOperation,
    options: { bundled?: boolean } = {},
  ) => {
    let archivePath = krakenArchivePath.trim();
    setKrakenImportError(null);
    setKrakenImportResult(null);

    if (options.bundled) {
      archivePath = "";
    } else if (!archivePath && isFilePickerAvailable) {
      const selected = await pickFile({
        title:
          operation === "full"
            ? "Choose extracted Kraken OHLCVT folder"
            : "Choose Kraken update OHLCVT CSV or ZIP",
        directory: operation === "full",
        filters:
          operation === "full"
            ? undefined
            : [
                {
                  name: "Kraken OHLCVT",
                  extensions: ["zip", "csv"],
                },
              ],
      });
      if (!selected) return;
      archivePath = selected;
      setKrakenArchivePath(selected);
    }

    if (!options.bundled && !archivePath) {
      setKrakenImportError("Enter a local Kraken CSV or ZIP path.");
      return;
    }

    setPendingKrakenOperation(operation);
    setPendingBundledKrakenImport(Boolean(options.bundled));
    try {
      const envelope = await importKrakenRates.mutateAsync({
        ...(options.bundled ? { use_bundled: true } : { path: archivePath }),
        operation,
      });
      setKrakenImportResult(envelope.data ?? null);
    } catch (error) {
      setKrakenImportError(
        error instanceof Error ? error.message : "Kraken import failed.",
      );
    } finally {
      setPendingKrakenOperation(null);
      setPendingBundledKrakenImport(false);
    }
  };

  const isImportingKraken = importKrakenRates.isPending;
  const isRebuildingRates = rebuildRates.isPending;
  const rateRebuildProgress = rateRebuildTransactionProgress(rateRebuildResult);
  const rateRebuildSamples =
    rateRebuildResult?.sync.reduce(
      (total, row) => total + Number(row.samples ?? 0),
      0,
    ) ?? 0;
  const rateRebuildJournalBlocker = rateRebuildJournalError(rateRebuildResult);
  const startRateRebuild = async () => {
    setRateRebuildError(null);
    setRateRebuildResult(null);
    rebuildNoticeRef.current = addNotification({
      title: "Pricing cache rebuild started",
      body: "Kassiber is clearing provider-derived prices, fetching fresh Coinbase one-minute windows, and reprocessing journals.",
      tone: "warning",
      progress: {
        indeterminate: true,
        label: "Rebuilding",
      },
    });
    try {
      const envelope = await rebuildRates.mutateAsync({
        source: "coinbase-exchange",
        reprice_transactions: true,
      });
      const payload = envelope.data ?? null;
      setRateRebuildResult(payload);
      setRateRebuildOpen(false);
      const journalBlocker = rateRebuildJournalError(payload);
      const fetchedRows =
        payload?.sync.reduce(
          (total, row) => total + Number(row.samples ?? 0),
          0,
        ) ?? 0;
      const notification = {
        title: journalBlocker
          ? "Pricing cache rebuilt with journal blocker"
          : "Pricing cache rebuilt",
        body: payload
          ? `${formatCount(payload.deleted.transaction_prices)} cached transaction prices cleared; ${formatCount(
              fetchedRows,
            )} rate rows fetched.${journalBlocker ? ` ${journalBlocker}` : ""}`
          : "Coinbase pricing cache was rebuilt.",
        tone: journalBlocker ? "warning" : "success",
        progress: undefined,
      } as const;
      if (rebuildNoticeRef.current) {
        updateNotification(rebuildNoticeRef.current, notification);
        rebuildNoticeRef.current = null;
      } else {
        addNotification(notification);
      }
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not rebuild pricing cache.";
      setRateRebuildError(message);
      const notification = {
        title: "Pricing cache rebuild failed",
        body: message,
        tone: "error",
        progress: undefined,
      } as const;
      if (rebuildNoticeRef.current) {
        updateNotification(rebuildNoticeRef.current, notification);
        rebuildNoticeRef.current = null;
      } else {
        addNotification(notification);
      }
    }
  };
  const importedPairs = krakenImportResult?.summary ?? [];
  const importedTotals = krakenImportResult?.totals;
  return (
    <section className="space-y-4">
      <p className="max-w-2xl text-sm text-muted-foreground">
        Fiat reference rates are sourced independently of wallet sync. Kassiber
        keeps a local price cache so reports never have to query an exchange for
        every transaction. These lookups reveal pricing interest, not your
        wallet addresses.
      </p>

      <div className="space-y-2">
        <p className="text-sm font-medium">Rate providers</p>
        <div className="grid gap-2">
          {rateBackends.map((backend, index) => (
            <div
              key={backend.id}
              className="flex flex-wrap items-center justify-between gap-2 rounded-md border bg-background p-3"
            >
              <div className="min-w-0 space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">{backend.name}</span>
                  <span
                    className={cn(
                      "inline-flex items-center rounded-md border px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide",
                      index === 0
                        ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                        : "border-border bg-muted text-muted-foreground",
                    )}
                  >
                    {index === 0 ? "Primary" : "Fallback"}
                  </span>
                </div>
                <p className="truncate font-mono text-xs text-muted-foreground">
                  {backend.url}
                </p>
              </div>
              <span className="shrink-0 text-xs text-muted-foreground">
                {backend.health}
              </span>
            </div>
          ))}
        </div>
      </div>

      <div className="rounded-md border bg-background p-3">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">Rebuild pricing cache</p>
            <p className="text-xs text-muted-foreground">
              Clear Coinbase provider samples, checked-empty minutes, and
              cached provider-generated transaction prices, then fetch fresh
              one-minute rates for the active books.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={() => {
              setRateRebuildError(null);
              setRateRebuildOpen(true);
            }}
            disabled={isRebuildingRates || isImportingKraken}
          >
            {isRebuildingRates ? (
              <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Database className="size-4" aria-hidden="true" />
            )}
            Rebuild cache
          </Button>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          Manual overrides and imported exchange execution prices are kept. Large
          wallets can take a while because Kassiber refetches missing windows and
          reprocesses journals afterward.
        </p>
        {isRebuildingRates ? (
          <div className="mt-3 rounded-md border border-primary/25 bg-primary/5 p-3">
            <div className="flex items-center justify-between gap-3 text-xs">
              <span className="font-medium text-foreground">
                Rebuilding provider rates
              </span>
              <span className="text-muted-foreground">
                Counting transaction rates…
              </span>
            </div>
            <div
              className="mt-2 h-2 overflow-hidden rounded-full bg-muted"
              role="progressbar"
              aria-label="Pricing cache rebuild progress"
              aria-valuetext="Rebuilding pricing cache"
            >
              <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              Kassiber is fetching missing one-minute rates and will report how
              many transactions have provider rates when journals finish.
            </p>
          </div>
        ) : null}
        {rateRebuildResult ? (
          <div
            className={cn(
              "mt-3 rounded-md border p-3 text-sm",
              rateRebuildJournalBlocker
                ? "border-amber-500/40 bg-amber-500/10 text-amber-800 dark:text-amber-200"
                : "border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300",
            )}
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-medium">
                {rateRebuildJournalBlocker
                  ? "Pricing refreshed; journals still blocked"
                  : rateRebuildProgress?.total
                  ? `${formatCount(rateRebuildProgress.refreshed)} / ${formatCount(
                      rateRebuildProgress.total,
                    )} transaction rates refreshed`
                  : "Pricing cache rebuilt"}
              </span>
              <span
                className={cn(
                  "text-xs",
                  rateRebuildJournalBlocker
                    ? "text-amber-800/80 dark:text-amber-200/80"
                    : "text-emerald-700/80 dark:text-emerald-300/80",
                )}
              >
                {formatCount(rateRebuildSamples)} rate rows fetched
              </span>
            </div>
            <div
              className={cn(
                "mt-2 h-2 overflow-hidden rounded-full",
                rateRebuildJournalBlocker
                  ? "bg-amber-950/10 dark:bg-amber-100/15"
                  : "bg-emerald-950/10 dark:bg-emerald-100/15",
              )}
              role="progressbar"
              aria-label="Transaction rate refresh progress"
              aria-valuemin={0}
              aria-valuemax={rateRebuildProgress?.total ?? 1}
              aria-valuenow={rateRebuildProgress?.refreshed ?? 1}
            >
              <div
                className={cn(
                  "h-full w-full rounded-full",
                  rateRebuildJournalBlocker ? "bg-amber-500" : "bg-emerald-500",
                )}
              />
            </div>
            {rateRebuildJournalBlocker ? (
              <p className="mt-2 text-xs text-amber-800/80 dark:text-amber-200/80">
                {rateRebuildJournalBlocker}
              </p>
            ) : null}
            <p
              className={cn(
                "mt-2 text-xs",
                rateRebuildJournalBlocker
                  ? "text-amber-800/80 dark:text-amber-200/80"
                  : "text-emerald-700/80 dark:text-emerald-300/80",
              )}
            >
              Removed {formatCount(rateRebuildResult.deleted.rates)} rate rows,{" "}
              {formatCount(rateRebuildResult.deleted.checked_minutes)} checked
              minutes, and{" "}
              {formatCount(rateRebuildResult.deleted.transaction_prices)} cached
              transaction prices.
            </p>
          </div>
        ) : null}
      </div>

      <div className="rounded-md border bg-background p-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">Kraken offline history</p>
            <p className="text-xs text-muted-foreground">
              Bitcoin EUR/USD minute candles from a local Kraken CSV/ZIP
              archive, plus bundled daily values for fallback coverage.
            </p>
            <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
              <a
                href={KRAKEN_OHLCVT_SUPPORT_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_OHLCVT_SUPPORT_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                Get Kraken archive
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
              <a
                href={KRAKEN_MARKET_DATA_BLOG_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_MARKET_DATA_BLOG_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                Kraken market data blog
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
            </div>
          </div>
          <span className="inline-flex w-fit items-center rounded-md border bg-muted px-2 py-1 text-xs text-muted-foreground">
            kraken-csv
          </span>
        </div>

        <div className="mt-3 rounded-md border border-primary/20 bg-primary/5 p-3">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">
              Kraken offline history: minute data
            </p>
            <p className="text-xs text-muted-foreground">
              Import local Kraken BTC-EUR and BTC-USD one-minute candles for
              exact transaction pricing windows.
            </p>
          </div>

          <div className="mt-3 grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
            <Input
              value={krakenArchivePath}
              onChange={(event) => setKrakenArchivePath(event.target.value)}
              placeholder="~/Downloads/Kraken_OHLCVT.zip, CSV, or extracted folder"
              aria-label="Kraken CSV, ZIP, or folder path"
              disabled={isImportingKraken}
            />
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                className="flex-1 sm:flex-none"
                onClick={() => void chooseKrakenArchive()}
                disabled={!isFilePickerAvailable || isImportingKraken}
                title={
                  isFilePickerAvailable
                    ? "Choose CSV or ZIP"
                    : "Use the path field in browser mode"
                }
              >
                <Upload className="size-4" aria-hidden="true" />
                File
              </Button>
              <Button
                type="button"
                variant="outline"
                className="flex-1 sm:flex-none"
                onClick={() => void chooseKrakenDirectory()}
                disabled={!isFilePickerAvailable || isImportingKraken}
                title={
                  isFilePickerAvailable
                    ? "Choose extracted folder"
                    : "Use the path field in browser mode"
                }
              >
                <FileInput className="size-4" aria-hidden="true" />
                Folder
              </Button>
            </div>
          </div>

          <div className="mt-3 flex flex-col gap-2 sm:flex-row">
            <Button
              type="button"
              onClick={() => void startKrakenImport("full")}
              disabled={isImportingKraken}
            >
              {pendingKrakenOperation === "full" && !pendingBundledKrakenImport ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Database className="size-4" aria-hidden="true" />
              )}
              Full minute history
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => void startKrakenImport("incremental")}
              disabled={isImportingKraken}
            >
              {pendingKrakenOperation === "incremental" ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <RefreshCw className="size-4" aria-hidden="true" />
              )}
              Incremental minute update
            </Button>
          </div>
        </div>

        <div className="mt-3 rounded-md border bg-muted/30 p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-medium">Bundled daily values fallback</p>
              <p className="text-xs text-muted-foreground">
                Import bundled Kraken BTC-EUR and BTC-USD daily values from
                2013 through Q1 2026 for coarse fallback coverage.
              </p>
            </div>
            <Button
              type="button"
              variant="outline"
              className="shrink-0"
              onClick={() => void startKrakenImport("full", { bundled: true })}
              disabled={isImportingKraken}
            >
              {pendingBundledKrakenImport ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Database className="size-4" aria-hidden="true" />
              )}
              Import daily values
            </Button>
          </div>
        </div>

        {krakenImportError ? (
          <div className="mt-3 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
            <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <span>{krakenImportError}</span>
          </div>
        ) : null}

        {krakenImportResult ? (
          <div className="mt-3 rounded-md border border-emerald-500/30 bg-emerald-500/5 p-3">
            <div className="flex items-start gap-2 text-sm text-emerald-700 dark:text-emerald-300">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <span>
                {importedTotals?.pairs
                  ? `${formatCount(importedTotals.samples)} rows across ${formatCount(
                      importedTotals.pairs,
                    )} pair${importedTotals.pairs === 1 ? "" : "s"}`
                  : "No Bitcoin rows imported"}
              </span>
            </div>
            {importedPairs.length ? (
              <div className="mt-2 divide-y rounded-md border bg-background text-xs">
                {importedPairs.map((row) => (
                  <div
                    key={row.pair}
                    className="grid gap-1 px-3 py-2 sm:grid-cols-[120px_minmax(0,1fr)_120px]"
                  >
                    <span className="font-medium">{row.pair}</span>
                    <span className="truncate text-muted-foreground">
                      {formatKrakenRange(row)}
                    </span>
                    <span className="text-muted-foreground sm:text-right">
                      {formatCount(row.samples)} {row.granularity ?? ""} rows
                    </span>
                  </div>
                ))}
              </div>
            ) : null}
            {importedTotals?.skipped_rows || importedTotals?.skipped_files ? (
              <p className="mt-2 text-xs text-muted-foreground">
                Skipped {formatCount(importedTotals.skipped_rows)} rows and{" "}
                {formatCount(importedTotals.skipped_files)} files.
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
      <Dialog open={rateRebuildOpen} onOpenChange={setRateRebuildOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Rebuild pricing cache?</DialogTitle>
            <DialogDescription>
              Kassiber will delete Coinbase provider cache rows and refetch
              one-minute rates for missing transaction windows in the active
              books.
            </DialogDescription>
          </DialogHeader>
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
            <div className="flex items-start gap-2">
              <AlertTriangle
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
              <div className="space-y-1">
                <p className="font-medium">Large wallets can take a while.</p>
                <p>
                  The rebuild also clears provider-generated transaction prices
                  and reprocesses journals. Manual overrides and imported
                  execution prices are preserved.
                </p>
              </div>
            </div>
          </div>
          {rateRebuildError ? (
            <p className="text-sm text-destructive">{rateRebuildError}</p>
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setRateRebuildOpen(false)}
              disabled={isRebuildingRates}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void startRateRebuild()}
              disabled={isRebuildingRates}
            >
              {isRebuildingRates ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Database className="size-4" aria-hidden="true" />
              )}
              {isRebuildingRates ? "Rebuilding..." : "Rebuild"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
