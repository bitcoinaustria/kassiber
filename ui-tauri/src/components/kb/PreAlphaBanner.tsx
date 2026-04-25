import { AlertTriangle } from "lucide-react";

import { cn } from "@/lib/utils";

export function PreAlphaBanner({ className }: { className?: string }) {
  return (
    <section
      role="status"
      aria-label="Pre-alpha software warning"
      className={cn(
        "flex h-9 w-full items-center justify-center gap-2 bg-[#E3000F] px-4 text-center text-sm font-medium text-white",
        className,
      )}
    >
      <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
      <span>
        Pre-alpha software: unreliable at best. Review everything before relying
        on reports or exports.
      </span>
    </section>
  );
}
