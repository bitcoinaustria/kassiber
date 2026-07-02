import { cn } from "@/lib/utils";

import { blurClass } from "./model";

export function ChartStat({
  label,
  value,
  detail,
  prefix = "",
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
  detail?: string;
  prefix?: string;
  tone?: "good" | "bad" | "neutral";
  hidden: boolean;
}) {
  return (
    <div className="min-w-0 rounded-md bg-background/70 px-2.5 py-2">
      <div className="text-[10px] font-medium text-muted-foreground">
        {label}
      </div>
      <div
        className={cn(
          "mt-0.5 truncate text-sm font-semibold tabular-nums",
          tone === "good" && "text-emerald-600 dark:text-emerald-400",
          tone === "bad" && "text-[var(--kb-accent)]",
          blurClass(hidden),
        )}
      >
        {prefix}
        {value}
      </div>
      {detail && (
        <div className="mt-0.5 truncate text-[10px] text-muted-foreground">
          {detail}
        </div>
      )}
    </div>
  );
}
