import { ChevronDown, ChevronLeft, ChevronRight } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * One year out of a book's history, however long it is.
 *
 * The arrows step to the neighbouring year, which is what a year-end export
 * usually needs; the year itself opens a compact grid for jumping further.
 * `years` is newest first. With `allowAll`, `null` means every year.
 */
export function YearPicker({
  years,
  value,
  onChange,
  allowAll = false,
  className,
}: {
  years: number[];
  value: number | null;
  onChange: (year: number | null) => void;
  allowAll?: boolean;
  className?: string;
}) {
  const { t } = useTranslation("transactions");
  const [open, setOpen] = React.useState(false);
  const index = value === null ? -1 : years.indexOf(value);
  // Newest first: an older year is further down the list.
  const older = value === null ? years[0] : years[index + 1];
  const newer = index > 0 ? years[index - 1] : index === 0 && allowAll ? null : undefined;
  const allLabel = t("dashboard.export.yearPicker.all");
  const pick = (year: number | null) => {
    onChange(year);
    setOpen(false);
  };
  const arrow =
    "flex size-8 shrink-0 items-center justify-center text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground disabled:pointer-events-none disabled:opacity-35 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none";

  return (
    <div
      className={cn(
        "inline-flex h-8 items-stretch overflow-hidden rounded-md border bg-background/70",
        className,
      )}
    >
      <button
        type="button"
        className={cn(arrow, "border-r")}
        aria-label={t("dashboard.export.yearPicker.older")}
        disabled={older === undefined}
        onClick={() => older !== undefined && onChange(older)}
      >
        <ChevronLeft className="size-4" aria-hidden="true" />
      </button>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-label={t("dashboard.export.yearPicker.label", {
              year: value === null ? allLabel : value,
            })}
            className="flex min-w-24 flex-1 items-center justify-center gap-1.5 px-3 text-sm font-medium tabular-nums transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
          >
            {value === null ? allLabel : value}
            <ChevronDown className="size-3.5 text-muted-foreground" aria-hidden="true" />
          </button>
        </PopoverTrigger>
        <PopoverContent align="center" className="w-60 p-2">
          {allowAll ? (
            <button
              type="button"
              role="radio"
              aria-checked={value === null}
              onClick={() => pick(null)}
              className={cn(
                "mb-1.5 w-full rounded-md px-2 py-1.5 text-sm transition-colors",
                value === null ? "bg-primary text-primary-foreground" : "hover:bg-muted/60",
              )}
            >
              {allLabel}
            </button>
          ) : null}
          <div
            role="radiogroup"
            aria-label={t("dashboard.export.year")}
            className="grid max-h-64 grid-cols-4 gap-1 overflow-y-auto"
          >
            {years.map((year) => (
              <button
                key={year}
                type="button"
                role="radio"
                aria-checked={value === year}
                onClick={() => pick(year)}
                className={cn(
                  "rounded-md py-1.5 text-sm tabular-nums transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                  value === year ? "bg-primary text-primary-foreground" : "hover:bg-muted/60",
                )}
              >
                {year}
              </button>
            ))}
          </div>
        </PopoverContent>
      </Popover>
      <button
        type="button"
        className={cn(arrow, "border-l")}
        aria-label={t("dashboard.export.yearPicker.newer")}
        disabled={newer === undefined}
        onClick={() => newer !== undefined && onChange(newer)}
      >
        <ChevronRight className="size-4" aria-hidden="true" />
      </button>
    </div>
  );
}
