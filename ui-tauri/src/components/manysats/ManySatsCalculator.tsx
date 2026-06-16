import * as React from "react";
import { Bitcoin, Copy, Equal, RefreshCw } from "lucide-react";

import { CopyButton } from "@/components/kb/CopyButton";
import {
  marketRateProviderLabel,
  type MaintenanceSettingsData,
  type RateLatestData,
} from "@/components/kb/settings/SettingsModel";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { useDaemon } from "@/daemon/client";
import { formatFiatAmount } from "@/lib/currency";
import { cn } from "@/lib/utils";
import {
  deriveBtc,
  formatBtcPlain,
  formatFiatPlain,
  formatSatsPlain,
  LIVE_FIATS,
  pairForFiat,
  parseFieldAmount,
  rateFromLatest,
  type ConversionField,
} from "./manySatsModel";

/** Seconds between automatic live-rate refreshes (provider-friendly cadence). */
const REFRESH_INTERVAL_SEC = 60;

/**
 * Count down to the next refresh and fire `onRefresh` at zero. Pauses while the
 * tab is hidden (so a backgrounded calculator never hammers the provider) and
 * resets whenever a fresh rate arrives (`resetKey` changes).
 */
function useCountdownRefresh(
  intervalSec: number,
  onRefresh: () => void,
  enabled: boolean,
  resetKey: unknown,
): number {
  const [remaining, setRemaining] = React.useState(intervalSec);
  const onRefreshRef = React.useRef(onRefresh);
  onRefreshRef.current = onRefresh;

  React.useEffect(() => {
    setRemaining(intervalSec);
  }, [resetKey, intervalSec]);

  React.useEffect(() => {
    if (!enabled) return;
    let hidden =
      typeof document !== "undefined" && document.visibilityState === "hidden";
    const onVisibility = () => {
      hidden = document.visibilityState === "hidden";
    };
    document.addEventListener("visibilitychange", onVisibility);
    const id = window.setInterval(() => {
      if (hidden) return;
      setRemaining((value) => {
        if (value <= 1) {
          onRefreshRef.current();
          return intervalSec;
        }
        return value - 1;
      });
    }, 1000);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [enabled, intervalSec]);

  return remaining;
}

function formatClock(timestamp: string | null): string | null {
  if (!timestamp) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** A pill-shaped unit badge (static), mirroring Boltz's asset chips. */
function UnitChip({
  icon,
  children,
}: {
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="inline-flex h-9 shrink-0 items-center gap-1.5 rounded-full border border-white/10 bg-muted px-3.5 font-sans text-sm font-semibold text-foreground">
      {icon}
      {children}
    </div>
  );
}

/** Decorative equals disc between amount panels (we convert, we don't swap). */
function Connector() {
  return (
    <div
      className="relative z-10 flex h-0 items-center justify-center"
      aria-hidden="true"
    >
      <span className="absolute flex size-8 items-center justify-center rounded-full border border-white/10 bg-card text-muted-foreground shadow-sm">
        <Equal className="size-3.5" />
      </span>
    </div>
  );
}

interface AmountPanelProps {
  fieldId: string;
  label: string;
  value: string;
  placeholder: string;
  inputMode: "decimal" | "numeric";
  disabled?: boolean;
  onChange: (raw: string) => void;
  /** Right-hand unit control: a currency Select (fiat) or a static UnitChip. */
  unit: React.ReactNode;
}

function AmountPanel({
  fieldId,
  label,
  value,
  placeholder,
  inputMode,
  disabled,
  onChange,
  unit,
}: AmountPanelProps) {
  return (
    <div className="rounded-xl border border-white/10 bg-background/60 px-4 py-3 transition-colors focus-within:border-[var(--color-accent)]/50">
      <div className="flex h-6 items-center justify-between">
        <label
          htmlFor={fieldId}
          className="font-sans text-[10px] font-semibold uppercase tracking-[0.14em] text-muted-foreground"
        >
          {label}
        </label>
        {value ? (
          <CopyButton
            value={value}
            ariaLabel={`Copy ${label}`}
            variant="ghost"
            size="icon-xs"
          />
        ) : (
          <span className="inline-flex size-6 items-center justify-center text-muted-foreground/40">
            <Copy className="size-3" aria-hidden="true" />
          </span>
        )}
      </div>
      <div className="mt-1 flex items-center gap-3">
        <Input
          id={fieldId}
          value={value}
          placeholder={placeholder}
          inputMode={inputMode}
          disabled={disabled}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => onChange(event.target.value)}
          className="h-auto min-w-0 flex-1 rounded-none border-0 bg-transparent p-0 font-mono text-2xl font-semibold tabular-nums shadow-none focus-visible:ring-0 disabled:opacity-50 sm:text-3xl dark:bg-transparent"
        />
        {unit}
      </div>
    </div>
  );
}

/**
 * ManySats — convert between fiat, BTC, and satoshis at the current live spot
 * price from the connected market-rate provider. Live-only (BTC-EUR / BTC-USD).
 * Layout takes design cues from the Boltz swap client (stacked amount panels
 * with asset chips and a connector disc) on the Bitcoin Austria theme.
 */
export function ManySatsCalculator() {
  // null = "use the profile's fiat currency" (discovered from a no-pair fetch).
  const [selectedFiat, setSelectedFiat] = React.useState<string | null>(null);
  const [active, setActive] = React.useState<{
    field: ConversionField;
    raw: string;
  }>({ field: "fiat", raw: "" });

  const settingsQuery = useDaemon<MaintenanceSettingsData>(
    "ui.maintenance.settings",
    undefined,
    { refetchOnWindowFocus: false },
  );
  const configuredProvider =
    settingsQuery.data?.data?.settings?.market_rate_provider ?? null;

  const latestArgs = selectedFiat ? { pair: pairForFiat(selectedFiat) } : undefined;
  const latestQuery = useDaemon<RateLatestData>("ui.rates.latest", latestArgs, {
    refetchOnWindowFocus: false,
  });

  // If the profile's fiat has no live spot, fall back to EUR rather than
  // sitting on the failed no-pair fetch.
  React.useEffect(() => {
    if (selectedFiat == null && latestQuery.isError) {
      setSelectedFiat("EUR");
    }
  }, [selectedFiat, latestQuery.isError]);

  const rate = rateFromLatest(latestQuery.data?.data);
  const displayFiat = (selectedFiat ?? rate?.fiatCurrency ?? "EUR").toUpperCase();
  const price = rate?.price ?? null;
  const hasPrice = price != null && price > 0;
  const rateLoading = latestQuery.isLoading;
  const isFetching = latestQuery.isFetching;

  const providerLabel = configuredProvider
    ? marketRateProviderLabel(configuredProvider)
    : rate?.source
      ? marketRateProviderLabel(rate.source)
      : null;
  const asOf = formatClock(rate?.fetchedAt ?? rate?.timestamp ?? null);

  const refresh = React.useCallback(() => {
    void latestQuery.refetch();
  }, [latestQuery]);
  const secondsLeft = useCountdownRefresh(
    REFRESH_INTERVAL_SEC,
    refresh,
    hasPrice,
    rate?.fetchedAt ?? rate?.timestamp ?? null,
  );

  const parsed = parseFieldAmount(active.raw, active.field);
  const btc = parsed == null ? null : deriveBtc(active.field, parsed, price);

  const valueFor = (field: ConversionField): string => {
    if (field === active.field) return active.raw;
    if (btc == null) return "";
    if (field === "btc") return formatBtcPlain(btc);
    if (field === "sats") return formatSatsPlain(btc);
    return price != null ? formatFiatPlain(btc * price, displayFiat) : "";
  };

  const onFieldChange = (field: ConversionField) => (raw: string) => {
    setActive({ field, raw });
  };

  return (
    <Card className="w-full gap-4 rounded-2xl border-white/10 bg-card p-5 shadow-xl sm:p-6">
      {/* Rate badge (left) + auto-refresh control (right) */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-h-[3.25rem] items-center rounded-full bg-[var(--color-accent)] px-5 py-2 text-center text-white shadow-sm">
          {hasPrice ? (
            <div className="leading-tight">
              <div className="font-mono text-lg font-bold tabular-nums">
                {formatFiatAmount(price, displayFiat)}
              </div>
              <div className="text-[11px] font-semibold uppercase tracking-[0.1em] text-white/85">
                {displayFiat} / BTC
              </div>
            </div>
          ) : rateLoading ? (
            <Skeleton className="h-7 w-28 bg-white/30" />
          ) : (
            <span className="text-sm font-semibold">Rate unavailable</span>
          )}
        </div>
        <Button
          type="button"
          variant="secondary"
          className="rounded-full"
          onClick={refresh}
          disabled={isFetching}
        >
          <RefreshCw
            className={cn("size-4", isFetching && "animate-spin")}
            aria-hidden="true"
          />
          {isFetching ? "Refreshing…" : `Refresh in ${secondsLeft}s`}
        </Button>
      </div>

      {/* Provider attribution */}
      <p className="text-xs text-muted-foreground">
        {hasPrice ? "Live rate" : "Live rate only"}
        {providerLabel ? <> · via {providerLabel}</> : null}
        {asOf ? <> · as of {asOf}</> : null}
      </p>

      {/* Stacked amount panels (Fiat = SAT = BTC), Boltz-style */}
      <div className="flex flex-col gap-1.5">
        <AmountPanel
          fieldId="manysats-fiat"
          label="Fiat"
          value={valueFor("fiat")}
          placeholder={hasPrice ? "0.00" : "—"}
          inputMode="decimal"
          disabled={!hasPrice}
          onChange={onFieldChange("fiat")}
          unit={
            <Select
              value={displayFiat}
              onValueChange={(value) => setSelectedFiat(value)}
            >
              <SelectTrigger
                aria-label="Fiat currency"
                className="h-9 gap-1.5 rounded-full border-white/10 bg-muted px-3.5 font-semibold"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LIVE_FIATS.map((code) => (
                  <SelectItem key={code} value={code}>
                    {code}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <Connector />
        <AmountPanel
          fieldId="manysats-sats"
          label="Satoshis"
          value={valueFor("sats")}
          placeholder="0"
          inputMode="numeric"
          onChange={onFieldChange("sats")}
          unit={<UnitChip>SAT</UnitChip>}
        />
        <Connector />
        <AmountPanel
          fieldId="manysats-btc"
          label="Bitcoin"
          value={valueFor("btc")}
          placeholder="0.00000000"
          inputMode="decimal"
          onChange={onFieldChange("btc")}
          unit={
            <UnitChip
              icon={<Bitcoin className="size-4 text-[#f7931a]" aria-hidden="true" />}
            >
              BTC
            </UnitChip>
          }
        />
      </div>
    </Card>
  );
}
