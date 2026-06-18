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
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import {
  formatCount,
  formatKrakenRange,
  KRAKEN_MARKET_DATA_BLOG_URL,
  KRAKEN_OHLCVT_SUPPORT_URL,
  type MaintenanceSettingsData,
  type MarketRateProvider,
  marketRateProviderLabel,
  rateRebuildJournalError,
  rateRebuildTransactionProgress,
  type Backend,
  type KrakenRatesImportData,
  type KrakenRatesImportOperation,
  type RateRebuildData,
} from "./SettingsModel";

export function MarketDataSettingsPanel({ backends }: { backends: Backend[] }) {
  const { t } = useTranslation(["settings", "common"]);
  const rateBackends = backends.filter((backend) => backend.net === "FX");
  const maintenanceSettingsQuery = useDaemon<MaintenanceSettingsData>(
    "ui.maintenance.settings",
    undefined,
    { refetchOnMount: "always" },
  );
  const importKrakenRates = useDaemonMutation<KrakenRatesImportData>(
    "ui.rates.kraken_csv.import",
  );
  const rebuildRates = useDaemonMutation<RateRebuildData>("ui.rates.rebuild");
  const configureMaintenance =
    useDaemonMutation<MaintenanceSettingsData>("ui.maintenance.configure");
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
          title: t("marketData.openLinkErrorTitle"),
          body:
            error instanceof Error
              ? error.message
              : t("marketData.openLinkErrorBody"),
          tone: "warning",
        });
      });
    },
    [addNotification, t],
  );

  const chooseKrakenArchive = async () => {
    setKrakenImportError(null);
    const selected = await pickFile({
      title: t("marketData.pickArchiveTitle"),
      filters: [
        {
          name: t("marketData.pickArchiveFilterName"),
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
      title: t("marketData.pickFolderTitle"),
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
            ? t("marketData.pickFullTitle")
            : t("marketData.pickUpdateTitle"),
        directory: operation === "full",
        filters:
          operation === "full"
            ? undefined
            : [
                {
                  name: t("marketData.pickArchiveFilterName"),
                  extensions: ["zip", "csv"],
                },
              ],
      });
      if (!selected) return;
      archivePath = selected;
      setKrakenArchivePath(selected);
    }

    if (!options.bundled && !archivePath) {
      setKrakenImportError(t("marketData.importPathError"));
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
        error instanceof Error ? error.message : t("marketData.importError"),
      );
    } finally {
      setPendingKrakenOperation(null);
      setPendingBundledKrakenImport(false);
    }
  };

  const isImportingKraken = importKrakenRates.isPending;
  const isRebuildingRates = rebuildRates.isPending;
  const maintenanceSettings = maintenanceSettingsQuery.data?.data ?? null;
  const freshnessSettings = maintenanceSettings?.settings ?? null;
  const autoMarketRatesEnabled = Boolean(
    freshnessSettings?.background_enabled &&
      freshnessSettings.source_classes?.market_rates,
  );
  const autoMarketRatesDisabled =
    maintenanceSettingsQuery.isLoading ||
    configureMaintenance.isPending ||
    !maintenanceSettings?.profile;
  const marketRateProvider: MarketRateProvider =
    freshnessSettings?.market_rate_provider ?? "coinbase-exchange";
  const marketRateProviderOptions: MarketRateProvider[] =
    freshnessSettings?.market_rate_providers?.length
      ? freshnessSettings.market_rate_providers
      : ["coinbase-exchange", "coingecko", "mempool"];
  const requireCoarseReview = freshnessSettings?.require_coarse_review ?? false;
  const coarsePricedCount = freshnessSettings?.coarse_priced_count ?? 0;
  // The coarse-review policy is independent of auto-pricing; gate it only on
  // load/mutation state and an active profile, not the market-rate toggles.
  const maintenanceBusy =
    maintenanceSettingsQuery.isLoading ||
    configureMaintenance.isPending ||
    !maintenanceSettings?.profile;
  const marketRateProviderLabelText = marketRateProviderLabel(marketRateProvider);
  const activeRatePair = freshnessSettings?.active_rate_pair ?? "BTC-fiat";
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
      title: t("marketData.rebuildStartedTitle"),
      body: t("marketData.rebuildStartedBody", {
        provider: marketRateProviderLabelText,
        pair: activeRatePair,
      }),
      tone: "warning",
      progress: {
        indeterminate: true,
        label: t("marketData.rebuildingProgressLabel"),
      },
    });
    try {
      const envelope = await rebuildRates.mutateAsync({
        source: marketRateProvider,
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
          ? t("marketData.rebuiltBlockedTitle")
          : t("marketData.rebuiltTitle"),
        body: payload
          ? t("marketData.rebuiltBody", {
              cleared: formatCount(payload.deleted.transaction_prices),
              fetched: formatCount(fetchedRows),
              blocker: journalBlocker ? ` ${journalBlocker}` : "",
            })
          : t("marketData.rebuiltBodyFallback", {
              provider: marketRateProviderLabelText,
            }),
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
        error instanceof Error
          ? error.message
          : t("marketData.rebuildFailedError");
      setRateRebuildError(message);
      const notification = {
        title: t("marketData.rebuildFailedTitle"),
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
  const setAutoMarketRates = async (enabled: boolean) => {
    if (!freshnessSettings) return;
    const sourceClasses = {
      ...freshnessSettings.source_classes,
      market_rates: enabled,
    };
    const anySourceClassEnabled = Object.values(sourceClasses).some(Boolean);
    const backgroundEnabled = enabled
      ? true
      : Boolean(freshnessSettings.background_enabled && anySourceClassEnabled);
    try {
      await configureMaintenance.mutateAsync({
        background_enabled: backgroundEnabled,
        source_classes: sourceClasses,
      });
      addNotification({
        title: enabled
          ? t("marketData.autoEnabledTitle")
          : t("marketData.autoDisabledTitle"),
        body: enabled
          ? t("marketData.autoEnabledBody")
          : t("marketData.autoDisabledBody"),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("marketData.autoFailedTitle"),
        body:
          error instanceof Error
            ? error.message
            : t("marketData.autoFailedBody"),
        tone: "error",
      });
    }
  };
  const setMarketRateProvider = async (provider: MarketRateProvider) => {
    if (provider === marketRateProvider) return;
    try {
      const envelope = await configureMaintenance.mutateAsync({
        market_rate_provider: provider,
      });
      const selectedProvider =
        envelope.data?.settings.market_rate_provider ?? provider;
      const selectedLabel = marketRateProviderLabel(selectedProvider);
      addNotification({
        title: t("marketData.providerUpdatedTitle"),
        body: t("marketData.providerUpdatedBody", { provider: selectedLabel }),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("marketData.providerFailedTitle"),
        body:
          error instanceof Error
            ? error.message
            : t("marketData.providerFailedBody"),
        tone: "error",
      });
    }
  };
  const setRequireCoarseReview = async (enabled: boolean) => {
    if (enabled === requireCoarseReview) return;
    try {
      await configureMaintenance.mutateAsync({ require_coarse_review: enabled });
      addNotification({
        title: enabled
          ? t("marketData.coarse.heldTitle")
          : t("marketData.coarse.acceptedTitle"),
        body: enabled
          ? t("marketData.coarse.heldBody")
          : t("marketData.coarse.acceptedBody"),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("marketData.coarse.errorTitle"),
        body:
          error instanceof Error
            ? error.message
            : t("marketData.coarse.errorBody"),
        tone: "error",
      });
    }
  };
  const importedPairs = krakenImportResult?.summary ?? [];
  const importedTotals = krakenImportResult?.totals;
  return (
    <section className="space-y-4">
      <p className="max-w-2xl text-sm text-muted-foreground">
        {t("marketData.intro")}
      </p>

      {coarsePricedCount > 0 && !requireCoarseReview ? (
        <div className="rounded-md border bg-background p-3 text-sm">
          <p className="font-medium">
            {t("marketData.coarse.countNotice", { count: coarsePricedCount })}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("marketData.coarse.countDescription")}
          </p>
        </div>
      ) : null}

      <div className="rounded-md border bg-background p-3">
        <div className="mb-3 flex flex-col gap-2 border-b pb-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="grid gap-1">
            <Label htmlFor="market-rate-provider">
              {t("marketData.priceSourceLabel")}
            </Label>
            <p className="text-xs text-muted-foreground">
              {t("marketData.priceSourceHint")}
            </p>
          </div>
          <Select
            value={marketRateProvider}
            disabled={autoMarketRatesDisabled}
            onValueChange={(value) => {
              void setMarketRateProvider(value as MarketRateProvider);
            }}
          >
            <SelectTrigger id="market-rate-provider" className="w-full sm:w-56">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {marketRateProviderOptions.map((provider) => (
                <SelectItem key={provider} value={provider}>
                  {marketRateProviderLabel(provider)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="flex items-start gap-3">
          <Checkbox
            id="market-data-auto-refresh"
            checked={autoMarketRatesEnabled}
            disabled={autoMarketRatesDisabled}
            onCheckedChange={(checked) => {
              void setAutoMarketRates(checked === true);
            }}
          />
          <Label
            htmlFor="market-data-auto-refresh"
            className="grid gap-1 text-sm leading-relaxed"
          >
            <span>{t("marketData.autoRefreshLabel")}</span>
            <span className="font-normal text-muted-foreground">
              {t("marketData.autoRefreshHint", {
                provider: marketRateProviderLabelText,
              })}
            </span>
          </Label>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          {t("marketData.autoRefreshFootnote")}
        </p>
        <div className="mt-3 flex items-start gap-3 border-t pt-3">
          <Checkbox
            id="require-coarse-review"
            checked={requireCoarseReview}
            disabled={maintenanceBusy}
            onCheckedChange={(checked) => {
              void setRequireCoarseReview(checked === true);
            }}
          />
          <Label
            htmlFor="require-coarse-review"
            className="grid gap-1 text-sm leading-relaxed"
          >
            <span>{t("marketData.coarse.reviewToggleLabel")}</span>
            <span className="font-normal text-muted-foreground">
              {t("marketData.coarse.reviewToggleDescription")}
            </span>
          </Label>
        </div>
      </div>

      <div className="space-y-2">
        <p className="text-sm font-medium">{t("marketData.providersHeading")}</p>
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
                    {index === 0
                      ? t("marketData.primary")
                      : t("marketData.fallback")}
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
            <p className="text-sm font-medium">
              {t("marketData.rebuildHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("marketData.rebuildDescription", { pair: activeRatePair })}
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
            {t("marketData.rebuildButton")}
          </Button>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          {t("marketData.rebuildFootnote")}
        </p>
        {isRebuildingRates ? (
          <div className="mt-3 rounded-md border border-primary/25 bg-primary/5 p-3">
            <div className="flex items-center justify-between gap-3 text-xs">
              <span className="font-medium text-foreground">
                {t("marketData.rebuildingTitle")}
              </span>
              <span className="text-muted-foreground">
                {t("marketData.rebuildingCounting")}
              </span>
            </div>
            <div
              className="mt-2 h-2 overflow-hidden rounded-full bg-muted"
              role="progressbar"
              aria-label={t("marketData.rebuildProgressAria")}
              aria-valuetext={t("marketData.rebuildProgressValue")}
            >
              <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              {t("marketData.rebuildingFootnote")}
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
                  ? t("marketData.resultBlocked")
                  : rateRebuildProgress?.total
                  ? t("marketData.resultProgress", {
                      refreshed: formatCount(rateRebuildProgress.refreshed),
                      total: formatCount(rateRebuildProgress.total),
                    })
                  : t("marketData.resultRebuilt")}
              </span>
              <span
                className={cn(
                  "text-xs",
                  rateRebuildJournalBlocker
                    ? "text-amber-800/80 dark:text-amber-200/80"
                    : "text-emerald-700/80 dark:text-emerald-300/80",
                )}
              >
                {t("marketData.resultRowsFetched", {
                  pair: rateRebuildResult.pair ?? activeRatePair,
                  rows: formatCount(rateRebuildSamples),
                })}
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
              aria-label={t("marketData.txRefreshAria")}
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
              {t("marketData.resultRemoved", {
                rates: formatCount(rateRebuildResult.deleted.rates),
                minutes: formatCount(rateRebuildResult.deleted.checked_minutes),
                prices: formatCount(
                  rateRebuildResult.deleted.transaction_prices,
                ),
              })}
            </p>
          </div>
        ) : null}
      </div>

      <div className="rounded-md border bg-background p-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">
              {t("marketData.krakenHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("marketData.krakenDescription")}
            </p>
            <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
              <a
                href={KRAKEN_OHLCVT_SUPPORT_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_OHLCVT_SUPPORT_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                {t("marketData.krakenGetArchive")}
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
              <a
                href={KRAKEN_MARKET_DATA_BLOG_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_MARKET_DATA_BLOG_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                {t("marketData.krakenBlog")}
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
              {t("marketData.krakenMinuteHeading")}
            </p>
            <p className="text-xs text-muted-foreground">
              {t("marketData.krakenMinuteDescription")}
            </p>
          </div>

          <div className="mt-3 grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
            <Input
              value={krakenArchivePath}
              onChange={(event) => setKrakenArchivePath(event.target.value)}
              placeholder={t("marketData.krakenPathPlaceholder")}
              aria-label={t("marketData.krakenPathAria")}
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
                    ? t("marketData.chooseFileTitle")
                    : t("marketData.filePickerUnavailable")
                }
              >
                <Upload className="size-4" aria-hidden="true" />
                {t("marketData.chooseFile")}
              </Button>
              <Button
                type="button"
                variant="outline"
                className="flex-1 sm:flex-none"
                onClick={() => void chooseKrakenDirectory()}
                disabled={!isFilePickerAvailable || isImportingKraken}
                title={
                  isFilePickerAvailable
                    ? t("marketData.chooseFolderTitle")
                    : t("marketData.filePickerUnavailable")
                }
              >
                <FileInput className="size-4" aria-hidden="true" />
                {t("marketData.chooseFolder")}
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
              {t("marketData.fullMinuteHistory")}
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
              {t("marketData.incrementalMinuteUpdate")}
            </Button>
          </div>
        </div>

        <div className="mt-3 rounded-md border bg-muted/30 p-3">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-medium">
                {t("marketData.bundledHeading")}
              </p>
              <p className="text-xs text-muted-foreground">
                {t("marketData.bundledDescription")}
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
              {t("marketData.importDailyValues")}
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
                  ? t("marketData.importedTotals", {
                      count: importedTotals.pairs,
                      rows: formatCount(importedTotals.samples),
                      pairs: formatCount(importedTotals.pairs),
                    })
                  : t("marketData.noRowsImported")}
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
                      {t("marketData.importedRowGranularity", {
                        rows: formatCount(row.samples),
                        granularity: row.granularity ?? "",
                      })}
                    </span>
                  </div>
                ))}
              </div>
            ) : null}
            {importedTotals?.skipped_rows || importedTotals?.skipped_files ? (
              <p className="mt-2 text-xs text-muted-foreground">
                {t("marketData.skipped", {
                  rows: formatCount(importedTotals.skipped_rows),
                  files: formatCount(importedTotals.skipped_files),
                })}
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
      <Dialog open={rateRebuildOpen} onOpenChange={setRateRebuildOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("marketData.rebuildDialogTitle")}</DialogTitle>
            <DialogDescription>
              {t("marketData.rebuildDialogDescription", {
                provider: marketRateProviderLabelText,
                pair: activeRatePair,
              })}
            </DialogDescription>
          </DialogHeader>
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
            <div className="flex items-start gap-2">
              <AlertTriangle
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
              <div className="space-y-1">
                <p className="font-medium">
                  {t("marketData.rebuildDialogWarningTitle")}
                </p>
                <p>{t("marketData.rebuildDialogWarningBody")}</p>
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
              {t("common:actions.cancel")}
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
              {isRebuildingRates
                ? t("marketData.rebuildDialogPending")
                : t("marketData.rebuildDialogSubmit")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
