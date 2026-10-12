import { Check, ChevronsUpDown, Search } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

import {
  EXPORT_FORMAT_GROUPS,
  EXPORT_FORMATS,
  exportFormat,
  exportFormatLogo,
  type ExportFormatId,
  type ExportFormatOption,
} from "./exportFormats";

/** Above this many formats the list gets a search field. */
const SEARCH_THRESHOLD = 6;

export function ExportFormatMark({
  option,
  className,
}: {
  option: ExportFormatOption;
  className?: string;
}) {
  const logo = exportFormatLogo(option);
  if (logo) {
    return (
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-md border bg-white p-0.5",
          className,
        )}
      >
        <img src={logo} alt="" className="max-h-full max-w-full object-contain" />
      </span>
    );
  }
  const Icon = option.icon;
  return (
    <span
      className={cn(
        "flex size-7 shrink-0 items-center justify-center rounded-md",
        option.tone ?? "bg-muted text-muted-foreground",
        className,
      )}
    >
      {Icon ? <Icon className="size-4" aria-hidden="true" /> : null}
    </span>
  );
}

export function ExportFormatPicker({
  value,
  onChange,
}: {
  value: ExportFormatId;
  onChange: (id: ExportFormatId) => void;
}) {
  const { t } = useTranslation("transactions");
  const [open, setOpen] = React.useState(false);
  const [query, setQuery] = React.useState("");
  const selected = exportFormat(value);
  const needle = query.trim().toLocaleLowerCase();
  const name = (option: ExportFormatOption) => t(`dashboard.export.formats.${option.id}`);
  const hint = (option: ExportFormatOption) => t(`dashboard.export.formatHints.${option.id}`);
  const visible = needle
    ? EXPORT_FORMATS.filter((option) =>
        [name(option), hint(option), option.extension].some((text) =>
          String(text).toLocaleLowerCase().includes(needle),
        ),
      )
    : EXPORT_FORMATS;

  return (
    <div className="grid gap-1.5">
      <span id="export-format-label" className="text-sm font-medium">
        {t("dashboard.export.format")}
      </span>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          if (!next) setQuery("");
        }}
      >
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-labelledby="export-format-label"
            aria-haspopup="listbox"
            className="flex w-full min-w-0 items-center gap-2.5 rounded-md border bg-background/70 px-2.5 py-2 text-left text-sm transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ExportFormatMark option={selected} />
            <span className="min-w-0 flex-1">
              <span className="block truncate font-medium">{name(selected)}</span>
              <span className="block truncate text-xs text-muted-foreground">{hint(selected)}</span>
            </span>
            <ChevronsUpDown className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          className="w-[min(26rem,calc(100vw-2rem))] p-0"
          onOpenAutoFocus={(event) => {
            if (EXPORT_FORMATS.length <= SEARCH_THRESHOLD) event.preventDefault();
          }}
        >
          {EXPORT_FORMATS.length > SEARCH_THRESHOLD ? (
            <div className="relative border-b p-2">
              <Search
                className="pointer-events-none absolute top-1/2 left-4.5 size-3.5 -translate-y-1/2 text-muted-foreground"
                aria-hidden="true"
              />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("dashboard.export.filterFormats")}
                aria-label={t("dashboard.export.filterFormats")}
                className="h-8 pl-8 text-sm"
              />
            </div>
          ) : null}
          <div
            role="radiogroup"
            aria-labelledby="export-format-label"
            className="max-h-80 overflow-y-auto p-1"
          >
            {EXPORT_FORMAT_GROUPS.map((group) => {
              const options = visible.filter((option) => option.group === group);
              if (!options.length) return null;
              return (
                <div key={group} className="py-1">
                  <p className="px-2 pb-1 text-xs text-muted-foreground">
                    {t(`dashboard.export.formatGroups.${group}`)}
                  </p>
                  {options.map((option) => {
                    const active = option.id === selected.id;
                    return (
                      <button
                        key={option.id}
                        type="button"
                        role="radio"
                        aria-checked={active}
                        onClick={() => {
                          onChange(option.id);
                          setOpen(false);
                          setQuery("");
                        }}
                        className={cn(
                          "flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                          active ? "bg-primary/10" : "hover:bg-muted/60",
                        )}
                      >
                        <ExportFormatMark option={option} />
                        <span className="min-w-0 flex-1">
                          <span className="block font-medium">{name(option)}</span>
                          <span className="block truncate text-xs text-muted-foreground">
                            {hint(option)}
                          </span>
                        </span>
                        <span className="rounded-sm border px-1.5 py-px font-mono text-[10px] tracking-wide text-muted-foreground">
                          {option.extension}
                        </span>
                        <Check
                          className={cn("size-4 shrink-0 text-primary", active ? "" : "invisible")}
                          aria-hidden="true"
                        />
                      </button>
                    );
                  })}
                </div>
              );
            })}
            {!visible.length ? (
              <p className="p-3 text-xs text-muted-foreground">{t("dashboard.export.noFormatMatch")}</p>
            ) : null}
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
