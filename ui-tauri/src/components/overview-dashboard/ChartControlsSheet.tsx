import { RefreshCw, X } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { cn } from "@/lib/utils";

import {
  activityFlowKeys,
  activityFlowLabelKeys,
  ACTIVITY_MARKER_INPUT_STEP_BTC,
  ACTIVITY_MARKER_SLIDER_MARKS,
  activityMarkerSliderValue,
  blurClass,
  clampActivityMarkerMinimum,
  DEFAULT_INCOMING_MARKER_MIN_BTC,
  DEFAULT_OUTGOING_MARKER_MIN_BTC,
  formatEditableActivityMarkerMinimum,
  periodKeys,
  periodLabelKeys,
  serializeActivityMarkerMinimum,
  useActivityFlowColors,
  type TimePeriod,
  type TreasuryChartSeriesKey,
  type TreasuryLegendItem,
  type TreasurySeriesVisibility,
} from "./model";

export function ActivityMarkerSlider({
  id,
  label,
  value,
  color,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  color: string;
  onChange: (value: number) => void;
}) {
  const marksId = `${id}-marks`;
  return (
    <div className="mt-3 space-y-2">
      <input
        aria-label={label}
        className="h-2 w-full cursor-pointer"
        list={marksId}
        min={0}
        max={ACTIVITY_MARKER_SLIDER_MARKS.length - 1}
        step={1}
        type="range"
        value={activityMarkerSliderValue(value)}
        style={{ accentColor: color }}
        onChange={(event) =>
          onChange(ACTIVITY_MARKER_SLIDER_MARKS[Number(event.currentTarget.value)] ?? 0)
        }
      />
      <datalist id={marksId}>
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark, index) => (
          <option key={mark} value={index} label={serializeActivityMarkerMinimum(mark)} />
        ))}
      </datalist>
      <div className="flex justify-between text-[10px] text-muted-foreground">
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark) => (
          <span key={mark} className="tabular-nums">
            {serializeActivityMarkerMinimum(mark)}
          </span>
        ))}
      </div>
    </div>
  );
}

export type ChartControlsSheetProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  period: TimePeriod;
  onPeriodChange: (period: TimePeriod) => void;
  primaryColor: string;
  legendItems: TreasuryLegendItem[];
  seriesVisible: TreasurySeriesVisibility;
  onToggleSeries: (key: TreasuryChartSeriesKey) => void;
  activeSeries: TreasuryChartSeriesKey | null;
  onHoverSeries: (key: TreasuryChartSeriesKey | null) => void;
  markerCount: number;
  visibleMarkerCount: number;
  incomingMarkerCount: number;
  visibleIncomingMarkerCount: number;
  outgoingMarkerCount: number;
  visibleOutgoingMarkerCount: number;
  incomingMarkerMinimumBtc: number;
  onIncomingMarkerMinimumChange: (value: number) => void;
  outgoingMarkerMinimumBtc: number;
  onOutgoingMarkerMinimumChange: (value: number) => void;
  onResetMarkerMinimums: () => void;
  hideSensitive: boolean;
};

export type ActivityMarkerValueEditorProps = {
  value: number;
  onChange: (value: number) => void;
  className?: string;
  hidden: boolean;
};

export function ActivityFlowKey() {
  const { t } = useTranslation("overview");
  const activityFlowColors = useActivityFlowColors();
  return (
    <div className="rounded-md border p-3">
      <p className="text-xs font-medium text-muted-foreground">
        {t("controls.activityFlows")}
      </p>
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs text-muted-foreground">
        {activityFlowKeys.map((flow) => (
          <div key={flow} className="flex min-w-0 items-center gap-2">
            <span
              className="size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: activityFlowColors[flow] }}
              aria-hidden="true"
            />
            <span className="truncate">{t(activityFlowLabelKeys[flow])}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function ActivityLegendSwatch({ muted = false }: { muted?: boolean }) {
  const activityFlowColors = useActivityFlowColors();
  return (
    <span
      className={cn(
        "flex w-11 shrink-0 items-center gap-0.5",
        muted && "opacity-40",
      )}
      aria-hidden="true"
    >
      {activityFlowKeys.map((flow) => (
        <span
          key={flow}
          className="size-1.5 rounded-full"
          style={{ backgroundColor: activityFlowColors[flow] }}
        />
      ))}
    </span>
  );
}

export function ChartControlsSheet({
  open,
  onOpenChange,
  period,
  onPeriodChange,
  primaryColor,
  legendItems,
  seriesVisible,
  onToggleSeries,
  activeSeries,
  onHoverSeries,
  markerCount,
  visibleMarkerCount,
  incomingMarkerCount,
  visibleIncomingMarkerCount,
  outgoingMarkerCount,
  visibleOutgoingMarkerCount,
  incomingMarkerMinimumBtc,
  onIncomingMarkerMinimumChange,
  outgoingMarkerMinimumBtc,
  onOutgoingMarkerMinimumChange,
  onResetMarkerMinimums,
  hideSensitive,
}: ChartControlsSheetProps) {
  const { t } = useTranslation(["overview", "common"]);
  const activityFlowColors = useActivityFlowColors();
  const markerMinimumsAtDefault =
    incomingMarkerMinimumBtc === DEFAULT_INCOMING_MARKER_MIN_BTC &&
    outgoingMarkerMinimumBtc === DEFAULT_OUTGOING_MARKER_MIN_BTC;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        className="w-[min(100vw,420px)] overflow-hidden p-0 sm:max-w-none"
        showCloseButton={false}
      >
        <SheetHeader className="border-b p-0">
          <div className="flex items-start justify-between gap-4 px-4 py-4 sm:px-6">
            <div className="min-w-0">
              <SheetTitle className="truncate text-xl sm:text-2xl">
                {t("controls.title")}
              </SheetTitle>
              <SheetDescription className="mt-1 truncate">
                {t("controls.description")}
              </SheetDescription>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <span className="text-[10px] text-muted-foreground">
                  {t("controls.dotsVisible", {
                    visible: visibleMarkerCount.toLocaleString("en-US"),
                    total: markerCount.toLocaleString("en-US"),
                  })}
                </span>
              </div>
            </div>
            <Button
              type="button"
              size="icon"
              variant="ghost"
              aria-label={t("controls.close")}
              onClick={() => onOpenChange(false)}
            >
              <X className="size-4" aria-hidden="true" />
            </Button>
          </div>
        </SheetHeader>

        <div className="min-h-0 flex-1 overflow-y-auto">
          <div className="space-y-5 p-4 sm:p-6">
            <div className="rounded-md border p-3">
              <p className="text-xs font-medium text-muted-foreground">
                {t("controls.timeRange")}
              </p>
              <div className="mt-3 grid grid-cols-2 gap-2">
                {periodKeys.map((key) => (
                  <button
                    key={key}
                    type="button"
                    aria-pressed={period === key}
                    className={cn(
                      "rounded-md border px-2.5 py-2 text-left text-sm transition-colors",
                      period === key
                        ? "text-foreground"
                        : "border-transparent bg-muted/20 text-muted-foreground hover:bg-muted/45 hover:text-foreground",
                    )}
                    style={
                      period === key
                        ? {
                            backgroundColor: `${primaryColor}24`,
                            borderColor: primaryColor,
                          }
                        : undefined
                    }
                    onClick={() => onPeriodChange(key)}
                  >
                    {t(periodLabelKeys[key])}
                  </button>
                ))}
              </div>
            </div>

            <ActivityFlowKey />

            <div className="rounded-md border p-3">
              <p className="text-xs font-medium text-muted-foreground">
                {t("controls.series")}
              </p>
              <div className="mt-3 space-y-1">
                {legendItems.map((item) => (
                  <label
                    key={item.key}
                    className={cn(
                      "flex cursor-pointer items-center gap-2 rounded-md px-2 py-2 text-sm transition-colors hover:bg-muted/35",
                      !seriesVisible[item.key] && "text-muted-foreground",
                      activeSeries !== null &&
                        activeSeries !== item.key &&
                        "opacity-55",
                    )}
                    onMouseEnter={() => onHoverSeries(item.key)}
                    onMouseLeave={() => onHoverSeries(null)}
                  >
                    <Checkbox
                      checked={seriesVisible[item.key]}
                      onCheckedChange={() => onToggleSeries(item.key)}
                      aria-label={t("controls.showSeries", { label: item.label })}
                      className="data-[state=checked]:border-[var(--chart-control-accent)] data-[state=checked]:bg-[var(--chart-control-accent)] data-[state=checked]:text-background"
                      style={
                        {
                          "--chart-control-accent": item.color,
                        } as React.CSSProperties
                      }
                    />
                    {item.key === "events" ? (
                      <ActivityLegendSwatch muted={!seriesVisible.events} />
                    ) : (
                      <span
                        className={cn(
                          "h-0.5 w-6 shrink-0 rounded-full",
                          item.dashed && "border-t border-dashed bg-transparent",
                        )}
                        style={{
                          backgroundColor: item.dashed ? "transparent" : item.color,
                          borderColor: item.color,
                        }}
                      />
                    )}
                    <span className="truncate">{item.label}</span>
                  </label>
                ))}
              </div>
            </div>

            <div className="space-y-3 rounded-md border p-3">
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <p className="text-xs font-medium text-muted-foreground">
                    {t("controls.markerSize")}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("controls.minBtcSize")}
                  </p>
                </div>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="shrink-0 gap-2"
                  onClick={onResetMarkerMinimums}
                  disabled={markerMinimumsAtDefault}
                >
                  <RefreshCw className="size-3.5" aria-hidden="true" />
                  {t("common:actions.reset")}
                </Button>
              </div>
              <div className="flex items-center justify-between gap-3 text-sm">
                <div>
                  <p className="text-xs font-medium text-muted-foreground">
                    {t("controls.incomingPayments")}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("controls.minSizeWithCount", {
                      visible: visibleIncomingMarkerCount.toLocaleString("en-US"),
                      total: incomingMarkerCount.toLocaleString("en-US"),
                    })}
                  </p>
                </div>
                <ActivityMarkerValueEditor
                  value={incomingMarkerMinimumBtc}
                  onChange={onIncomingMarkerMinimumChange}
                  hidden={hideSensitive}
                />
              </div>
              <ActivityMarkerSlider
                id="incoming-marker-minimum"
                label={t("controls.incomingSliderAria")}
                value={incomingMarkerMinimumBtc}
                color={activityFlowColors.incoming}
                onChange={onIncomingMarkerMinimumChange}
              />
            </div>

            <div className="rounded-md border border-red-500/20 bg-red-500/5 p-3">
              <div className="flex items-center justify-between gap-3 text-sm">
                <div>
                  <p className="text-xs font-medium text-red-500 dark:text-red-400">
                    {t("controls.outgoingActivity")}
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t("controls.minSizeWithCount", {
                      visible: visibleOutgoingMarkerCount.toLocaleString("en-US"),
                      total: outgoingMarkerCount.toLocaleString("en-US"),
                    })}
                  </p>
                </div>
                <ActivityMarkerValueEditor
                  value={outgoingMarkerMinimumBtc}
                  onChange={onOutgoingMarkerMinimumChange}
                  className="text-red-500 dark:text-red-400"
                  hidden={hideSensitive}
                />
              </div>
              <ActivityMarkerSlider
                id="outgoing-marker-minimum"
                label={t("controls.outgoingSliderAria")}
                value={outgoingMarkerMinimumBtc}
                color={activityFlowColors.outgoing}
                onChange={onOutgoingMarkerMinimumChange}
              />
            </div>
          </div>
        </div>
      </SheetContent>
    </Sheet>
  );
}

export function ActivityMarkerValueEditor({
  value,
  onChange,
  className,
  hidden,
}: ActivityMarkerValueEditorProps) {
  const { t } = useTranslation("overview");
  const formattedValue = formatEditableActivityMarkerMinimum(value);
  const [draft, setDraft] = React.useState(formattedValue);
  const [editing, setEditing] = React.useState(false);

  React.useEffect(() => {
    if (!editing) setDraft(formattedValue);
  }, [editing, formattedValue]);

  const commitDraft = React.useCallback(
    (rawValue: string) => {
      const parsed = Number(rawValue);
      if (!rawValue.trim() || !Number.isFinite(parsed)) {
        setDraft(formatEditableActivityMarkerMinimum(value));
        return;
      }
      const nextValue = clampActivityMarkerMinimum(parsed);
      onChange(nextValue);
      setDraft(formatEditableActivityMarkerMinimum(nextValue));
    },
    [onChange, value],
  );

  return (
    <label
      className={cn(
        "group inline-flex h-8 items-center rounded-md border border-transparent bg-transparent transition-colors hover:border-border hover:bg-background focus-within:border-ring focus-within:bg-background focus-within:ring-2 focus-within:ring-ring/20",
        className,
        hidden && blurClass(true),
      )}
      title={t("controls.customMinTitle")}
    >
      <input
        aria-label={t("controls.customMinAria")}
        className="h-full w-[10ch] rounded-l-md bg-transparent px-2 text-right font-medium tabular-nums outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
        min={0}
        step={ACTIVITY_MARKER_INPUT_STEP_BTC}
        type="number"
        value={editing ? draft : formattedValue}
        onBlur={(event) => {
          commitDraft(event.currentTarget.value);
          setEditing(false);
        }}
        onChange={(event) => {
          const nextDraft = event.currentTarget.value;
          setDraft(nextDraft);
          const parsed = Number(nextDraft);
          if (nextDraft.trim() && Number.isFinite(parsed)) {
            onChange(clampActivityMarkerMinimum(parsed));
          }
        }}
        onFocus={() => setEditing(true)}
      />
      <span className="pr-2 text-xs">BTC</span>
    </label>
  );
}
