import * as React from "react";
import { Copy, RefreshCw } from "lucide-react";

import { CopyButton } from "@/components/kb/CopyButton";
import {
  marketRateProviderLabel,
  type MaintenanceSettingsData,
  type RateLatestData,
} from "@/components/kb/settings/SettingsModel";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

interface ConversionRowProps {
  fieldId: string;
  value: string;
  placeholder: string;
  inputMode: "decimal" | "numeric";
  disabled?: boolean;
  onChange: (raw: string) => void;
  /** Right-hand unit control: a currency Select (fiat) or a unit label. */
  unit: React.ReactNode;
}

function ConversionRow({
  fieldId,
  value,
  placeholder,
  inputMode,
  disabled,
  onChange,
  unit,
}: ConversionRowProps) {
  return (
    <div className="flex items-center gap-3">
      <Input
        id={fieldId}
        value={value}
        placeholder={placeholder}
        inputMode={inputMode}
        disabled={disabled}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => onChange(event.target.value)}
        className="h-12 flex-1 text-right font-mono text-lg tabular-nums md:text-xl"
      />
      <div className="flex w-[5.5rem] shrink-0 justify-center">{unit}</div>
      {value ? (
        <CopyButton value={value} ariaLabel="Copy amount" size="icon" />
      ) : (
        <Button
          type="button"
          variant="outline"
          size="icon"
          disabled
          aria-label="Nothing to copy"
        >
          <Copy className="size-3.5" aria-hidden="true" />
        </Button>
      )}
    </div>
  );
}

function UnitLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="font-sans text-sm font-semibold uppercase tracking-wide text-muted-foreground">
      {children}
    </span>
  );
}

/**
 * ManySats — convert between fiat, BTC, and satoshis at the current live spot
 * price from the connected market-rate provider. Live-only (BTC-EUR / BTC-USD).
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
    <Card className="w-full gap-0 overflow-hidden">
      <CardContent className="flex flex-col gap-5">
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

        {/* Converter: Fiat → SAT → BTC, matching the ManySats layout */}
        <div className="flex flex-col gap-4">
          <ConversionRow
            fieldId="manysats-fiat"
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
                <SelectTrigger aria-label="Fiat currency" className="h-9 w-full">
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
          <ConversionRow
            fieldId="manysats-sats"
            value={valueFor("sats")}
            placeholder="0"
            inputMode="numeric"
            onChange={onFieldChange("sats")}
            unit={<UnitLabel>SAT</UnitLabel>}
          />
          <ConversionRow
            fieldId="manysats-btc"
            value={valueFor("btc")}
            placeholder="0.00000000"
            inputMode="decimal"
            onChange={onFieldChange("btc")}
            unit={<UnitLabel>BTC</UnitLabel>}
          />
        </div>
      </CardContent>
    </Card>
  );
}
