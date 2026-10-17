import { Check, ChevronDown } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { currentUiLocale } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import { PERIOD_PRESETS, type ExportPeriod, type PeriodPreset, type ResolvedPeriod } from "./exportPeriod";

/**
 * Which transactions an export covers, by calendar period.
 *
 * The picker only names the period ("last month", or two days) and the
 * timezone. The core resolves the exact bounds in that calendar, by default
 * the book's own (Europe/Vienna for Austrian books), so a period means the same
 * in the desktop, the CLI and the reports; the resolved days come back with
 * the preview and are shown under the picker.
 */
function formatDay(value: string): string {
  const [year, month, day] = value.split("-").map(Number);
  if (!year || !month || !day) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), { dateStyle: "medium", timeZone: "UTC" }).format(
    new Date(Date.UTC(year, month - 1, day)),
  );
}

function viewerTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

export function ExportPeriodPicker({
  value,
  onChange,
  bookTimezone,
  resolved,
  disabled = false,
}: {
  value: ExportPeriod;
  onChange: (next: ExportPeriod) => void;
  /** The book's calendar, from the core. */
  bookTimezone: string;
  resolved: ResolvedPeriod | null;
  disabled?: boolean;
}) {
  const { t } = useTranslation("transactions");
  const [open, setOpen] = React.useState(false);
  const [from, setFrom] = React.useState(value.from ?? "");
  const [to, setTo] = React.useState(value.to ?? "");
  const [customOpen, setCustomOpen] = React.useState(value.preset === "custom");
  const timezones = Array.from(new Set([bookTimezone, "UTC", viewerTimezone()]));
  const timezone = value.timezone ?? bookTimezone;
  const presetLabel = (preset: PeriodPreset) => t(`dashboard.export.period.presets.${preset}`);
  const triggerLabel =
    value.preset === "custom" && value.from && value.to
      ? t("dashboard.export.period.span", { from: formatDay(value.from), to: formatDay(value.to) })
      : presetLabel(value.preset);
  const choose = (preset: PeriodPreset) => {
    onChange({ preset, timezone: value.timezone });
    setCustomOpen(false);
    setOpen(false);
  };
  const customValid = Boolean(from && to && from <= to);

  return (
    <div className="grid gap-1.5">
      <span id="export-period-label" className="text-sm font-medium">
        {t("dashboard.export.period.label")}
      </span>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (next) {
            setFrom(value.from ?? "");
            setTo(value.to ?? "");
            setCustomOpen(value.preset === "custom");
          }
        }}
      >
        <PopoverTrigger asChild>
          <button
            type="button"
            disabled={disabled}
            aria-labelledby="export-period-label"
            className="flex w-full min-w-0 items-center justify-between gap-2 rounded-md border bg-background/70 px-3 py-2 text-left text-sm font-medium transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
          >
            <span className="truncate">{triggerLabel}</span>
            <ChevronDown className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          </button>
        </PopoverTrigger>
        <PopoverContent align="start" className="w-[min(18rem,calc(100vw-2rem))] p-0">
          <div className="grid gap-1.5 border-b p-3">
            <span className="text-xs text-muted-foreground">{t("dashboard.export.period.timezone")}</span>
            <Select
              value={timezone}
              onValueChange={(zone) =>
                onChange({ ...value, timezone: zone === bookTimezone ? undefined : zone })
              }
            >
              <SelectTrigger className="h-8" aria-label={t("dashboard.export.period.timezone")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {timezones.map((zone) => (
                  <SelectItem key={zone} value={zone}>
                    {zone === bookTimezone
                      ? t("dashboard.export.period.timezoneDefault", { zone })
                      : zone}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div role="radiogroup" aria-label={t("dashboard.export.period.label")} className="grid p-1">
            {[...PERIOD_PRESETS, "custom" as const].map((preset) => {
              const active = value.preset === preset;
              return (
                <button
                  key={preset}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  onClick={() => (preset === "custom" ? setCustomOpen(true) : choose(preset))}
                  className={cn(
                    "flex items-center justify-between rounded-sm px-2.5 py-1.5 text-left text-sm transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                    active ? "bg-primary/10 font-medium" : "hover:bg-muted/60",
                  )}
                >
                  {presetLabel(preset)}
                  {active ? <Check className="size-3.5 text-primary" aria-hidden="true" /> : null}
                </button>
              );
            })}
          </div>
          {customOpen ? (
            <form
              className="grid gap-2 border-t p-3"
              onSubmit={(event) => {
                event.preventDefault();
                if (!customValid) return;
                onChange({ preset: "custom", from, to, timezone: value.timezone });
                setOpen(false);
              }}
            >
              <div className="grid grid-cols-2 gap-2">
                <label className="grid gap-1 text-xs text-muted-foreground">
                  {t("dashboard.export.period.from")}
                  <Input type="date" value={from} max={to || undefined} onChange={(e) => setFrom(e.target.value)} className="h-8" />
                </label>
                <label className="grid gap-1 text-xs text-muted-foreground">
                  {t("dashboard.export.period.to")}
                  <Input type="date" value={to} min={from || undefined} onChange={(e) => setTo(e.target.value)} className="h-8" />
                </label>
              </div>
              <Button type="submit" size="sm" disabled={!customValid}>
                {t("dashboard.export.period.apply")}
              </Button>
            </form>
          ) : null}
        </PopoverContent>
      </Popover>
      <p className="text-xs text-muted-foreground">
        {value.preset === "all"
          ? t("dashboard.export.period.everything")
          : resolved
            ? `${t("dashboard.export.period.span", {
                from: formatDay(resolved.first_day),
                to: formatDay(resolved.last_day),
              })} · ${resolved.timezone}`
            : timezone}
      </p>
    </div>
  );
}
