import * as React from "react";
import { cn } from "@/lib/utils";

interface GutterStatProps {
  label: string;
  value: React.ReactNode;
  color?: string;
}

export function GutterStat({ label, value, color }: GutterStatProps) {
  return (
    <div className="min-w-0">
      <div className="font-mono text-[8px] uppercase tracking-[0.12em] text-ink-3">
        {label}
      </div>
      <div
        className={cn(
          "whitespace-nowrap font-sans text-[13px] font-medium leading-[1.15] tracking-[-0.005em]",
          color ?? "text-ink",
        )}
      >
        {value}
      </div>
    </div>
  );
}
