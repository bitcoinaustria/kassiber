import { useTranslation } from "react-i18next";
import { KeyRound, RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { currentUiLocale, formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import { ExchangeErrorNotice } from "./ExchangeErrorNotice";
import {
  EXCHANGE_PROVIDER_NAMES,
  exchangeErrorKind,
  type ExchangeKind,
} from "./exchangeApiModel";
import type { useExchangeSync } from "./useExchangeSync";

function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function ExchangeSyncCard({
  exchange,
  state,
  disabled,
}: {
  exchange: ExchangeKind;
  state: ReturnType<typeof useExchangeSync>;
  disabled?: boolean;
}) {
  const { t } = useTranslation("connections");
  const provider = EXCHANGE_PROVIDER_NAMES[exchange];
  const { row, statusError, lastError } = state;
  const lastSync = row?.last_sync ?? null;
  const needsRust = statusError && exchangeErrorKind(statusError.code) === "needsRust";
  const error = lastError ?? (lastSync?.status === "error" ? lastSync.error ?? null : null);
  const keyStored = row ? row.backend_configured && row.has_api_key : null;
  const skipped = lastSync?.skipped
    ? Object.values(lastSync.skipped).reduce((total, value) => total + (value ?? 0), 0)
    : 0;

  return (
    <section
      aria-labelledby="exchange-sync-title"
      className="kb-surface space-y-3 p-4 text-sm"
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h2 id="exchange-sync-title" className="flex items-center gap-2 font-medium">
            <KeyRound className="size-4" aria-hidden="true" />
            {t("exchange.sync.title", { provider })}
          </h2>
          <p className="m-0 text-xs text-muted-foreground">
            {t("exchange.sync.body", { provider })}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="shrink-0"
          disabled={disabled || state.isPending || Boolean(needsRust) || keyStored === false}
          aria-busy={state.isPending}
          onClick={() => void state.sync()}
        >
          <RefreshCw
            className={cn("size-4", state.isPending && "animate-spin")}
            aria-hidden="true"
          />
          {state.isPending ? t("exchange.sync.running") : t("exchange.sync.button")}
        </Button>
      </div>

      {needsRust && statusError ? (
        <ExchangeErrorNotice provider={provider} error={statusError} />
      ) : (
        <dl className="grid grid-cols-1 gap-x-6 gap-y-1 text-xs sm:grid-cols-2">
          <div className="flex justify-between gap-3 border-b border-border/60 py-1.5">
            <dt className="text-muted-foreground">{t("exchange.sync.key")}</dt>
            <dd className="m-0 text-right font-medium">
              {keyStored === null
                ? "…"
                : keyStored
                  ? t("exchange.sync.keyStored", { backend: row?.backend ?? "" })
                  : t("exchange.sync.keyMissing")}
            </dd>
          </div>
          <div className="flex justify-between gap-3 border-b border-border/60 py-1.5">
            <dt className="text-muted-foreground">{t("exchange.sync.lastSync")}</dt>
            <dd className="m-0 text-right font-medium">
              {!lastSync
                ? t("exchange.sync.never")
                : lastSync.status === "ok"
                  ? t("exchange.sync.ok", {
                      time: formatTime(lastSync.at),
                      imported: formatCount(lastSync.imported ?? 0),
                      fetched: formatCount(lastSync.fetched ?? 0),
                    })
                  : t("exchange.sync.failed", { time: formatTime(lastSync.at) })}
            </dd>
          </div>
        </dl>
      )}
      {lastSync?.status === "ok" && skipped > 0 ? (
        <p className="m-0 text-xs text-muted-foreground">
          {t("exchange.sync.skipped", { count: skipped })}
        </p>
      ) : null}
      {error && !needsRust ? <ExchangeErrorNotice provider={provider} error={error} /> : null}
      <p className="m-0 text-2xs text-muted-foreground">{t("exchange.sync.sessionNote")}</p>
    </section>
  );
}
