import * as React from "react";
import { ArrowDownRight, ArrowUpRight, TriangleAlert } from "lucide-react";

import { cn } from "@/lib/utils";
import { useCountUp } from "./motion";

/**
 * Shared pieces of the assistant's visuals: count-up figures, stat tiles,
 * a segmented switch, the tooltip frame and a legend. Every value is text in
 * text tokens; colour only marks identity beside it.
 */

export function CountUp({
  value,
  format,
  delayMs,
  className,
  sensitive,
}: {
  value: number | null;
  format: (value: number) => string;
  delayMs?: number;
  className?: string;
  sensitive?: boolean;
}) {
  const shown = useCountUp(value, delayMs);
  return (
    <span className={cn(sensitive && "sensitive", className)}>
      {/* Screen readers get the final value, not every frame. */}
      <span aria-hidden="true">{shown === null ? "—" : format(shown)}</span>
      <span className="sr-only">{value === null ? "—" : format(value)}</span>
    </span>
  );
}

export type Tone = "positive" | "negative" | "neutral" | "warning";

export function DeltaPill({
  label,
  value,
  className,
  sensitive,
}: {
  label: string;
  value: number;
  className?: string;
  sensitive?: boolean;
}) {
  const Icon = value >= 0 ? ArrowUpRight : ArrowDownRight;
  return (
    <span
      className={cn(
        "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-2xs font-medium",
        value > 0
          ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : value < 0
            ? "bg-red-500/10 text-red-700 dark:text-red-300"
            : "bg-muted text-muted-foreground",
        className,
      )}
    >
      <Icon className="size-3" aria-hidden="true" />
      <span className={cn(sensitive && "sensitive")}>{label}</span>
    </span>
  );
}

export function StatTile({
  label,
  value,
  format,
  hint,
  delta,
  incomplete,
  incompleteLabel,
  delayMs,
  sensitive,
  className,
}: {
  label: string;
  value: number | null;
  format: (value: number) => string;
  hint?: React.ReactNode;
  delta?: { label: string; value: number } | null;
  incomplete?: boolean;
  incompleteLabel?: string;
  delayMs?: number;
  sensitive?: boolean;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "kb-visual-rise min-w-0 rounded-md border border-border/60 bg-background/40 px-3 py-2",
        className,
      )}
      style={{ animationDelay: `${delayMs ?? 0}ms` }}
    >
      <div className="truncate text-2xs text-muted-foreground">{label}</div>
      <div className="mt-0.5 flex flex-wrap items-baseline gap-x-1.5">
        <CountUp
          value={value}
          format={format}
          delayMs={delayMs}
          sensitive={sensitive}
          className={cn("text-base font-semibold", incomplete && "text-muted-foreground")}
        />
        {delta ? <DeltaPill label={delta.label} value={delta.value} sensitive={sensitive} /> : null}
      </div>
      {incomplete ? (
        <div className="mt-0.5 flex items-center gap-1 text-2xs text-amber-700 dark:text-amber-300">
          <TriangleAlert className="size-3" aria-hidden="true" />
          {incompleteLabel}
        </div>
      ) : hint ? (
        <div className="mt-0.5 truncate text-2xs text-muted-foreground">{hint}</div>
      ) : null}
    </div>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
  label: string;
}) {
  if (options.length < 2) return null;
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="inline-flex shrink-0 items-center gap-0.5 rounded-md border border-border/60 bg-background/40 p-0.5"
    >
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          onClick={() => onChange(option.value)}
          className={cn(
            "h-6 rounded-sm px-2 text-2xs font-medium transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
            option.value === value
              ? "bg-foreground/10 text-foreground"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function TooltipFrame({
  title,
  rows,
  sensitive,
}: {
  title: string;
  rows: Array<{ key: string; color?: string; label: string; value: string; dashed?: boolean }>;
  sensitive?: boolean;
}) {
  return (
    <div className="min-w-36 rounded-md border bg-popover/95 px-2.5 py-2 text-xs shadow-lg">
      <div className="mb-1 text-2xs text-muted-foreground">{title}</div>
      <div className="space-y-0.5">
        {rows.map((row) => (
          <div key={row.key} className="flex items-center gap-2">
            {row.color ? (
              <span
                className="h-0.5 w-3 shrink-0 rounded-full"
                style={{ backgroundColor: row.color }}
                aria-hidden="true"
              />
            ) : null}
            <span className={cn("font-medium tabular-nums", sensitive && "sensitive")}>{row.value}</span>
            <span className="truncate text-muted-foreground">{row.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function Legend({
  items,
  shape = "line",
}: {
  items: Array<{ key: string; label: string; color: string }>;
  shape?: "line" | "square";
}) {
  if (items.length < 2) return null;
  return (
    <ul className="flex flex-wrap items-center gap-x-3 gap-y-1 text-2xs text-muted-foreground">
      {items.map((item) => (
        <li key={item.key} className="flex items-center gap-1.5">
          <span
            className={shape === "line" ? "h-0.5 w-3 rounded-full" : "size-2 rounded-xs"}
            style={{ backgroundColor: item.color }}
            aria-hidden="true"
          />
          {item.label}
        </li>
      ))}
    </ul>
  );
}

/** A horizontal bar that grows from its baseline on mount. */
export function GrowBar({
  fraction,
  color,
  delayMs = 0,
  className,
  align = "start",
}: {
  fraction: number;
  color: string;
  delayMs?: number;
  className?: string;
  align?: "start" | "end";
}) {
  const width = `${Math.max(0, Math.min(1, fraction)) * 100}%`;
  return (
    <span
      className={cn(
        "kb-visual-grow-x block h-full rounded-full",
        align === "end" ? "ml-auto origin-right" : "origin-left",
        className,
      )}
      style={{ width, backgroundColor: color, animationDelay: `${delayMs}ms` }}
      aria-hidden="true"
    />
  );
}
