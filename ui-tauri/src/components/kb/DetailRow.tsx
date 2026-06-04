import { type ReactNode } from "react";

import { CopyButton } from "@/components/kb/CopyButton";
import { cn } from "@/lib/utils";

interface DetailRowProps {
  label: string;
  value: ReactNode;
  mono?: boolean;
  copy?: boolean;
}

export function DetailRow({ label, value, mono, copy }: DetailRowProps) {
  return (
    <div className="flex min-w-0 items-center gap-3 text-sm">
      <span className="shrink-0 text-xs font-medium text-muted-foreground">
        {label}
      </span>
      <span
        className={cn(
          "ml-auto min-w-0 flex-1 truncate text-right",
          mono && "font-mono text-xs",
        )}
      >
        {value}
      </span>
      {copy && typeof value === "string" ? <CopyButton value={value} /> : null}
    </div>
  );
}
