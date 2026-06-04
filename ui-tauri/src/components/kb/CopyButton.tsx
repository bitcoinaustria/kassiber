import { type ComponentProps, useState } from "react";
import { Check, Copy } from "lucide-react";

import { Button } from "@/components/ui/button";
import { copyTextWithPolicy } from "@/lib/clipboard";

/**
 * Copy-to-clipboard icon button with a brief "copied" confirmation.
 * Honors the workspace clipboard-clear policy via copyTextWithPolicy.
 */
export function CopyButton({
  value,
  ariaLabel = "Copy",
  className,
  variant = "outline",
  size = "icon-xs",
}: {
  value: string;
  ariaLabel?: string;
  className?: string;
  variant?: ComponentProps<typeof Button>["variant"];
  size?: ComponentProps<typeof Button>["size"];
}) {
  const [copied, setCopied] = useState(false);
  const onCopy = async () => {
    try {
      await copyTextWithPolicy(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1100);
    } catch {
      // Clipboard access is best-effort in browser preview.
    }
  };
  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      className={className}
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
