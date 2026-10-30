import { ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { yearPeriod, type PeriodYear, type YearPeriodKey } from "@/lib/period";
import { cn } from "@/lib/utils";

/**
 * One calendar year of the book, picked from the years that hold activity.
 * Thirty years take the room of three, so the ranges beside it stay one short
 * row. The caller styles the trigger to sit among its own range chips.
 */
export function PeriodYearMenu({
  years,
  value,
  onSelect,
  className,
}: {
  /** Newest first. */
  years: readonly PeriodYear[];
  /** The year on show, or null while a range is. */
  value: YearPeriodKey | null;
  onSelect: (year: YearPeriodKey) => void;
  className?: string;
}) {
  const { t } = useTranslation("common");
  if (!years.length && !value) return null;
  const thisYear = new Date().getFullYear();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={t("period.yearMenuAria")}
          aria-pressed={value !== null}
          className={cn("inline-flex shrink-0 items-center gap-1 tabular-nums", className)}
        >
          {value ?? t("period.year")}
          <ChevronDown className="size-3.5 opacity-70" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-72 min-w-44">
        <DropdownMenuRadioGroup
          value={value ?? ""}
          onValueChange={(next) => onSelect(next as YearPeriodKey)}
        >
          {years.map(({ year, count }) => (
            <DropdownMenuRadioItem
              key={year}
              value={yearPeriod(year)}
              className="justify-between gap-4"
            >
              <span className="tabular-nums">{year}</span>
              <span className="text-xs text-muted-foreground tabular-nums">
                {year === thisYear
                  ? t("period.yearCountSoFar", { count })
                  : t("period.yearCount", { count })}
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
