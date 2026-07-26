import { Settings } from "lucide-react";
import type * as React from "react";
import { useTranslation } from "react-i18next";

import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

import {
  periodKeys,
  periodLabelKeys,
  periodShortLabelKeys,
  type ResolvedTimePeriod,
  type TimePeriod,
} from "./model";

// Don't take focus on click: macOS Full Keyboard Access draws a native focus
// ring that ignores CSS. Keyboard tab focus still works.
const preventClickFocus = (event: React.MouseEvent) => event.preventDefault();

// TradingView-style chart footer: quick range presets on the left; scale-mode
// chips plus a small collapsible settings menu (TradingView's bottom-right
// scale menu) on the right.
export function ChartRangeToolbar({
  period,
  periodOptions = periodKeys,
  resolvedPeriod = null,
  onPeriodChange,
  yScaleLog,
  onYScaleLogChange,
  yAutoFit,
  onYAutoFitChange,
  showLastValue,
  onShowLastValueChange,
  onOpenMoreSettings,
}: {
  period: TimePeriod;
  periodOptions?: TimePeriod[];
  /** Window "auto" resolved to, named in the Auto chip's tooltip. */
  resolvedPeriod?: ResolvedTimePeriod | null;
  onPeriodChange: (period: TimePeriod) => void;
  yScaleLog: boolean;
  onYScaleLogChange: (value: boolean) => void;
  yAutoFit: boolean;
  onYAutoFitChange: (value: boolean) => void;
  showLastValue: boolean;
  onShowLastValueChange: (value: boolean) => void;
  onOpenMoreSettings: () => void;
}) {
  const { t } = useTranslation("overview");
  const chipClass = (active: boolean) =>
    cn(
      "rounded px-1.5 py-0.5 text-xs font-medium tabular-nums transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      active
        ? "bg-muted text-foreground"
        : "text-muted-foreground hover:bg-muted/45 hover:text-foreground",
    );
  return (
    <div className="mt-2 flex select-none flex-wrap items-center justify-between gap-x-3 gap-y-1 border-t pt-2">
      <div
        role="group"
        aria-label={t("controls.timeRange")}
        className="flex flex-wrap items-center gap-0.5"
      >
        {periodOptions.map((key) => {
          const label =
            key === "auto" && resolvedPeriod
              ? t("period.autoResolved", {
                  period: t(periodLabelKeys[resolvedPeriod]),
                })
              : t(periodLabelKeys[key]);
          return (
          <button
            key={key}
            type="button"
            aria-pressed={period === key}
            aria-label={label}
            title={label}
            className={chipClass(period === key)}
            onClick={() => onPeriodChange(key)}
            onMouseDown={preventClickFocus}
          >
            {t(periodShortLabelKeys[key])}
          </button>
          );
        })}
      </div>
      <div
        role="group"
        aria-label={t("controls.scale")}
        className="flex items-center gap-0.5"
      >
        <button
          type="button"
          aria-pressed={yScaleLog}
          title={t("controls.logScaleTitle")}
          className={chipClass(yScaleLog)}
          onClick={() => onYScaleLogChange(!yScaleLog)}
          onMouseDown={preventClickFocus}
        >
          {t("controls.logChip")}
        </button>
        <button
          type="button"
          aria-pressed={yAutoFit}
          title={t("controls.autoFitTitle")}
          className={chipClass(yAutoFit)}
          onClick={() => onYAutoFitChange(!yAutoFit)}
          onMouseDown={preventClickFocus}
        >
          {t("controls.autoChip")}
        </button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={t("controls.scaleMenuAria")}
              className={cn(
                chipClass(false),
                "inline-flex items-center data-[state=open]:bg-muted data-[state=open]:text-foreground",
              )}
            >
              <Settings className="size-3.5" aria-hidden="true" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent side="top" align="end" className="w-64">
            {/* Log and auto-fit are the chips to the left; only what has no
                chip of its own lives in here. */}
            <DropdownMenuCheckboxItem
              checked={showLastValue}
              onCheckedChange={onShowLastValueChange}
              onSelect={(event) => event.preventDefault()}
            >
              {t("controls.lastValueLabel")}
            </DropdownMenuCheckboxItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={onOpenMoreSettings}>
              <Settings className="size-4" aria-hidden="true" />
              {t("controls.moreSettings")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
