import { RefreshCw, X } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { formatCount } from "@/lib/localeFormat";

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
  serializeActivityMarkerMinimum,
  useActivityFlowColors,
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
      <div className="flex justify-between text-2xs text-muted-foreground">
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
  groupActivityDots: boolean;
  onGroupActivityDotsChange: (value: boolean) => void;
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

// Incoming and outgoing are the same control twice over, so they render from
// one row: same shape, same counts, only the flow colour differs.
function MarkerMinimumRow({
  id,
  label,
  sliderLabel,
  color,
  value,
  onChange,
  visibleCount,
  totalCount,
  hideSensitive,
}: {
  id: string;
  label: string;
  sliderLabel: string;
  color: string;
  value: number;
  onChange: (value: number) => void;
  visibleCount: number;
  totalCount: number;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("overview");
  return (
    <div>
      <div className="flex items-center justify-between gap-3 text-sm">
        <div className="min-w-0">
          <p className="flex items-center gap-2 text-xs font-medium text-foreground">
            <span
              className="size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: color }}
              aria-hidden="true"
            />
            <span className="truncate">{label}</span>
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {t("controls.dotsShownOfTotal", {
              visible: formatCount(visibleCount),
              total: formatCount(totalCount),
            })}
          </p>
        </div>
        <ActivityMarkerValueEditor
          value={value}
          onChange={onChange}
          hidden={hideSensitive}
        />
      </div>
      <ActivityMarkerSlider
        id={id}
        label={sliderLabel}
        value={value}
        color={color}
        onChange={onChange}
      />
    </div>
  );
}

export function ChartControlsSheet({
  open,
  onOpenChange,
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
  groupActivityDots,
  onGroupActivityDotsChange,
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
              <SheetDescription className="mt-1">
                {t("controls.dotsVisible", {
                  visible: formatCount(visibleMarkerCount),
                  total: formatCount(markerCount),
                })}
              </SheetDescription>
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
          {/* Series live in the legend row above the chart, the time range in
              the chart's own footer toolbar. This panel is the dots. */}
          <div className="space-y-5 p-4 sm:p-6">
            <label className="flex cursor-pointer items-start gap-3 rounded-md border p-3 text-sm transition-colors hover:bg-muted/35">
              <Checkbox
                checked={groupActivityDots}
                onCheckedChange={(checked) =>
                  onGroupActivityDotsChange(checked === true)
                }
                className="mt-0.5"
              />
              <span className="min-w-0">
                <span className="block text-xs font-medium text-foreground">
                  {t("controls.groupDotsLabel")}
                </span>
                <span className="mt-1 block text-xs text-muted-foreground">
                  {t("controls.mergedMarkersHint")}
                </span>
              </span>
            </label>

            <div className="space-y-4 rounded-md border p-3">
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
              <MarkerMinimumRow
                id="incoming-marker-minimum"
                label={t("controls.incomingPayments")}
                sliderLabel={t("controls.incomingSliderAria")}
                color={activityFlowColors.incoming}
                value={incomingMarkerMinimumBtc}
                onChange={onIncomingMarkerMinimumChange}
                visibleCount={visibleIncomingMarkerCount}
                totalCount={incomingMarkerCount}
                hideSensitive={hideSensitive}
              />
              <MarkerMinimumRow
                id="outgoing-marker-minimum"
                label={t("controls.outgoingActivity")}
                sliderLabel={t("controls.outgoingSliderAria")}
                color={activityFlowColors.outgoing}
                value={outgoingMarkerMinimumBtc}
                onChange={onOutgoingMarkerMinimumChange}
                visibleCount={visibleOutgoingMarkerCount}
                totalCount={outgoingMarkerCount}
                hideSensitive={hideSensitive}
              />
            </div>

            <ActivityFlowKey />
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
