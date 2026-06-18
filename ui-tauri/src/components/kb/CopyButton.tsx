import { type ComponentProps, useState } from "react";
import { Check, Copy } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { copyTextWithPolicy } from "@/lib/clipboard";

/**
 * Copy-to-clipboard icon button with a brief "copied" confirmation.
 * Honors the workspace clipboard-clear policy via copyTextWithPolicy.
 */
export function CopyButton({
  value,
  ariaLabel,
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
  const { t } = useTranslation("chrome");
  const copyLabel = ariaLabel ?? t("copyButton.copy");
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
      aria-label={copied ? t("copyButton.copied") : copyLabel}
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
