import * as React from "react";
import { useTranslation } from "react-i18next";

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
import { Switch } from "@/components/ui/switch";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { openExternalUrl } from "@/daemon/transport";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
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
import { SettingsGroup, SettingsRow, SettingsSelect } from "./SettingsControls";

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
  const liveMinuteRatesEnabled = Boolean(
    freshnessSettings?.source_classes?.market_rates,
  );
  const liveMinuteRatesDisabled =
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
  const bitcoinRailCarryingValue =
    freshnessSettings?.bitcoin_rail_carrying_value ?? true;
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
    if (!liveMinuteRatesEnabled) return;
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
  const setLiveMinuteRates = async (enabled: boolean) => {
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
  const setBitcoinRailCarryingValue = async (enabled: boolean) => {
    if (enabled === bitcoinRailCarryingValue) return;
    try {
      await configureMaintenance.mutateAsync({
        bitcoin_rail_carrying_value: enabled,
      });
      addNotification({
        title: enabled
          ? t("marketData.bitcoinRails.enabledTitle")
          : t("marketData.bitcoinRails.disabledTitle"),
        body: enabled
          ? t("marketData.bitcoinRails.enabledBody")
          : t("marketData.bitcoinRails.disabledBody"),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("marketData.bitcoinRails.errorTitle"),
        body:
          error instanceof Error
            ? error.message
            : t("marketData.bitcoinRails.errorBody"),
        tone: "error",
      });
    }
  };
  const importedPairs = krakenImportResult?.summary ?? [];
  const importedTotals = krakenImportResult?.totals;
  const linkClass =
    "underline underline-offset-4 hover:text-foreground";
  return (
    <div className="space-y-6">
      <SettingsGroup
        title={t("marketData.liveHeading")}
        footer={t("marketData.intro")}
      >
        <SettingsRow
          label={t("marketData.autoRefreshLabel")}
          description={t("marketData.autoRefreshHint", {
            provider: marketRateProviderLabelText,
          })}
          htmlFor="market-data-auto-refresh"
        >
          <Switch
            id="market-data-auto-refresh"
            checked={liveMinuteRatesEnabled}
            disabled={liveMinuteRatesDisabled}
            onCheckedChange={(checked) => {
              void setLiveMinuteRates(checked);
            }}
          />
        </SettingsRow>
        <SettingsRow
          label={t("marketData.priceSourceLabel")}
          description={t("marketData.priceSourceHint")}
          htmlFor="market-rate-provider"
        >
          <SettingsSelect<MarketRateProvider>
            value={marketRateProvider}
            disabled={liveMinuteRatesDisabled}
            onValueChange={(value) => {
              void setMarketRateProvider(value);
            }}
            options={marketRateProviderOptions.map((provider) => ({
              value: provider,
              label: marketRateProviderLabel(provider),
            }))}
            triggerProps={{ id: "market-rate-provider" }}
          />
        </SettingsRow>
        <SettingsRow
          label={t("marketData.rebuildHeading")}
          description={
            liveMinuteRatesEnabled
              ? t("marketData.rebuildDescription", { pair: activeRatePair })
              : t("marketData.rebuildOptInRequired")
          }
        >
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => {
              setRateRebuildError(null);
              setRateRebuildOpen(true);
            }}
            disabled={
              isRebuildingRates || isImportingKraken || !liveMinuteRatesEnabled
            }
          >
            {isRebuildingRates
              ? t("marketData.rebuildDialogPending")
              : t("marketData.rebuildButton")}
          </Button>
        </SettingsRow>
        {isRebuildingRates ? (
          <div role="status" className="space-y-1 px-4 py-3 text-sm">
            <p>
              {t("marketData.rebuildingTitle")}
              <span className="text-muted-foreground">
                {" · "}
                {t("marketData.rebuildingCounting")}
              </span>
            </p>
            <p className="text-muted-foreground">
              {t("marketData.rebuildingFootnote")}
            </p>
          </div>
        ) : null}
        {rateRebuildResult ? (
          <div className="space-y-1 px-4 py-3 text-sm">
            <p
              className={
                rateRebuildJournalBlocker
                  ? "text-amber-600 dark:text-amber-400"
                  : "text-emerald-600 dark:text-emerald-400"
              }
            >
              {rateRebuildJournalBlocker
                ? t("marketData.resultBlocked")
                : rateRebuildProgress?.total
                  ? t("marketData.resultProgress", {
                      refreshed: formatCount(rateRebuildProgress.refreshed),
                      total: formatCount(rateRebuildProgress.total),
                    })
                  : t("marketData.resultRebuilt")}
            </p>
            {rateRebuildJournalBlocker ? (
              <p className="text-amber-600 dark:text-amber-400">
                {rateRebuildJournalBlocker}
              </p>
            ) : null}
            <p className="text-muted-foreground">
              {t("marketData.resultRowsFetched", {
                pair: rateRebuildResult.pair ?? activeRatePair,
                rows: formatCount(rateRebuildSamples),
              })}
              {" · "}
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
      </SettingsGroup>

      <SettingsGroup title={t("marketData.policyHeading")}>
        <SettingsRow
          label={t("marketData.coarse.reviewToggleLabel")}
          description={t("marketData.coarse.reviewToggleDescription")}
          htmlFor="require-coarse-review"
        >
          <Switch
            id="require-coarse-review"
            checked={requireCoarseReview}
            disabled={maintenanceBusy}
            onCheckedChange={(checked) => {
              void setRequireCoarseReview(checked);
            }}
          />
        </SettingsRow>
        {coarsePricedCount > 0 && !requireCoarseReview ? (
          <SettingsRow
            label={t("marketData.coarse.countNotice", { count: coarsePricedCount })}
            description={t("marketData.coarse.countDescription")}
          >
            {null}
          </SettingsRow>
        ) : null}
        <SettingsRow
          label={t("marketData.bitcoinRails.toggleLabel")}
          description={t("marketData.bitcoinRails.toggleDescription")}
          htmlFor="bitcoin-rail-carrying-value"
        >
          <Switch
            id="bitcoin-rail-carrying-value"
            checked={bitcoinRailCarryingValue}
            disabled={maintenanceBusy}
            onCheckedChange={(checked) => {
              void setBitcoinRailCarryingValue(checked);
            }}
          />
        </SettingsRow>
      </SettingsGroup>

      {rateBackends.length > 0 ? (
        <SettingsGroup title={t("marketData.providersHeading")}>
          {rateBackends.map((backend, index) => (
            <SettingsRow
              key={backend.id}
              label={backend.name}
              description={
                <span className="block truncate">
                  {index === 0 ? t("marketData.primary") : t("marketData.fallback")}
                  {" · "}
                  {backend.url}
                </span>
              }
            >
              <span className="text-sm text-muted-foreground">{backend.health}</span>
            </SettingsRow>
          ))}
        </SettingsGroup>
      ) : null}

      <SettingsGroup
        title={t("marketData.krakenHeading")}
        footer={
          <>
            <a
              href={KRAKEN_OHLCVT_SUPPORT_URL}
              onClick={(event) => openMarketDataUrl(event, KRAKEN_OHLCVT_SUPPORT_URL)}
              className={linkClass}
            >
              {t("marketData.krakenGetArchive")}
            </a>
            {" · "}
            <a
              href={KRAKEN_MARKET_DATA_BLOG_URL}
              onClick={(event) => openMarketDataUrl(event, KRAKEN_MARKET_DATA_BLOG_URL)}
              className={linkClass}
            >
              {t("marketData.krakenBlog")}
            </a>
          </>
        }
      >
        <SettingsRow
          label={t("marketData.krakenMinuteHeading")}
          description={t("marketData.krakenMinuteDescription")}
        >
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void startKrakenImport("full")}
            disabled={isImportingKraken}
          >
            {pendingKrakenOperation === "full" && !pendingBundledKrakenImport
              ? t("marketData.importing")
              : t("marketData.fullMinuteHistory")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void startKrakenImport("incremental")}
            disabled={isImportingKraken}
          >
            {pendingKrakenOperation === "incremental"
              ? t("marketData.importing")
              : t("marketData.incrementalMinuteUpdate")}
          </Button>
        </SettingsRow>
        <div className="flex items-center gap-2 px-4 py-3">
          <Input
            className="h-8 min-w-0 flex-1"
            value={krakenArchivePath}
            onChange={(event) => setKrakenArchivePath(event.target.value)}
            placeholder={t("marketData.krakenPathPlaceholder")}
            aria-label={t("marketData.krakenPathAria")}
            disabled={isImportingKraken}
          />
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void chooseKrakenArchive()}
            disabled={!isFilePickerAvailable || isImportingKraken}
            title={
              isFilePickerAvailable
                ? t("marketData.chooseFileTitle")
                : t("marketData.filePickerUnavailable")
            }
          >
            {t("marketData.chooseFile")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void chooseKrakenDirectory()}
            disabled={!isFilePickerAvailable || isImportingKraken}
            title={
              isFilePickerAvailable
                ? t("marketData.chooseFolderTitle")
                : t("marketData.filePickerUnavailable")
            }
          >
            {t("marketData.chooseFolder")}
          </Button>
        </div>
        <SettingsRow
          label={t("marketData.bundledHeading")}
          description={t("marketData.bundledDescription")}
        >
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void startKrakenImport("full", { bundled: true })}
            disabled={isImportingKraken}
          >
            {pendingBundledKrakenImport
              ? t("marketData.importing")
              : t("marketData.importDailyValues")}
          </Button>
        </SettingsRow>
        {krakenImportError ? (
          <p role="alert" className="px-4 py-3 text-sm text-destructive">
            {krakenImportError}
          </p>
        ) : null}
        {krakenImportResult ? (
          <div className="space-y-1 px-4 py-3 text-sm">
            <p
              className={
                importedTotals?.pairs
                  ? "text-emerald-600 dark:text-emerald-400"
                  : "text-amber-600 dark:text-amber-400"
              }
            >
              {importedTotals?.pairs
                ? t("marketData.importedTotals", {
                    count: importedTotals.pairs,
                    rows: formatCount(importedTotals.samples),
                    pairs: formatCount(importedTotals.pairs),
                  })
                : t("marketData.noRowsImported")}
            </p>
            {importedPairs.map((row) => (
              <p key={row.pair} className="text-muted-foreground">
                {row.pair}
                {" · "}
                {formatKrakenRange(row)}
                {" · "}
                {t("marketData.importedRowGranularity", {
                  rows: formatCount(row.samples),
                  granularity: row.granularity ?? "",
                })}
              </p>
            ))}
            {importedTotals?.skipped_rows || importedTotals?.skipped_files ? (
              <p className="text-muted-foreground">
                {t("marketData.skipped", {
                  rows: formatCount(importedTotals.skipped_rows),
                  files: formatCount(importedTotals.skipped_files),
                })}
              </p>
            ) : null}
          </div>
        ) : null}
      </SettingsGroup>

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
          <p className="text-sm text-amber-700 dark:text-amber-300">
            <span className="font-medium">
              {t("marketData.rebuildDialogWarningTitle")}
            </span>{" "}
            {t("marketData.rebuildDialogWarningBody")}
          </p>
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
              disabled={isRebuildingRates || !liveMinuteRatesEnabled}
            >
              {isRebuildingRates
                ? t("marketData.rebuildDialogPending")
                : t("marketData.rebuildDialogSubmit")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
