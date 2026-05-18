import { type ReactNode, useState } from "react";
import { Check, Copy } from "lucide-react";

import { Button } from "@/components/ui/button";
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

function CopyButton({
  value,
  ariaLabel = "Copy",
}: {
  value: string;
  ariaLabel?: string;
}) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1100);
    } catch {
      // Clipboard access is best-effort in browser preview.
    }
  };
  return (
    <Button
      type="button"
      variant="outline"
      size="icon-xs"
      aria-label={copied ? "Copied" : ariaLabel}
      onClick={onCopy}
    >
      {copied ? (
        <Check
          className="size-3 text-emerald-600 dark:text-emerald-400"
          aria-hidden="true"
        />
      ) : (
        <Copy className="size-3" aria-hidden="true" />
      )}
    </Button>
  );
}
