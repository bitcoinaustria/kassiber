import { cn } from "@/lib/utils";

export type Range = "d" | "w" | "m" | "ytd" | "1y" | "5y" | "all";

const RANGES: Array<[Range, string]> = [
  ["d", "D"],
  ["w", "W"],
  ["m", "M"],
  ["ytd", "YTD"],
  ["1y", "1Y"],
  ["5y", "5Y"],
  ["all", "ALL"],
];

interface RangeTabsProps {
  value: Range;
  onChange: (range: Range) => void;
  className?: string;
}

export function RangeTabs({ value, onChange, className }: RangeTabsProps) {
  return (
    <div className={cn("flex gap-0.5", className)}>
      {RANGES.map(([k, lbl]) => {
        const active = value === k;
        return (
          <button
            key={k}
            onClick={() => onChange(k)}
            className={cn(
              "cursor-pointer border px-1.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-[0.08em]",
              active
                ? "border-ink bg-ink text-paper"
                : "border-line bg-transparent text-ink-2",
            )}
          >
            {lbl}
          </button>
        );
      })}
    </div>
  );
}
