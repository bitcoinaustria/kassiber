import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export type Range = "d" | "w" | "m" | "ytd" | "1y" | "5y" | "all";

// `id` is the stable range value; `labelKey` indexes `chrome:range.*`.
const RANGES: Array<{ id: Range; labelKey: string }> = [
  { id: "d", labelKey: "d" },
  { id: "w", labelKey: "w" },
  { id: "m", labelKey: "m" },
  { id: "ytd", labelKey: "ytd" },
  { id: "1y", labelKey: "1y" },
  { id: "5y", labelKey: "5y" },
  { id: "all", labelKey: "all" },
];

interface RangeTabsProps {
  value: Range;
  onChange: (range: Range) => void;
  className?: string;
}

export function RangeTabs({ value, onChange, className }: RangeTabsProps) {
  const { t } = useTranslation("chrome");
  return (
    <div className={cn("flex gap-0.5", className)}>
      {RANGES.map(({ id, labelKey }) => {
        const active = value === id;
        return (
          <button
            key={id}
            onClick={() => onChange(id)}
            className={cn(
              "cursor-pointer border px-1.5 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-[0.08em]",
              active
                ? "border-ink bg-ink text-paper"
                : "border-line bg-transparent text-ink-2",
            )}
          >
            {t(`range.${labelKey}` as never) /* dynamic key */}
          </button>
        );
      })}
    </div>
  );
}
