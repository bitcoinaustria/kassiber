import type * as React from "react";
import { useTranslation } from "react-i18next";

import { SENSITIVE_MASK, useSensitive, type SensitiveKind } from "@/lib/sensitive";
import { cn } from "@/lib/utils";

/**
 * Renders its children, or the mask when the privacy mode hides this kind of
 * value. The real value is not rendered at all, so it is absent from the DOM,
 * the accessibility tree, and find-in-page.
 */
export function Sensitive({
  kind = "amount",
  hidden,
  className,
  children,
}: {
  kind?: SensitiveKind;
  /** Overrides the store, for callers that already hold the flag. */
  hidden?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  const fromStore = useSensitive(kind);
  const { t } = useTranslation("common");
  if (!(hidden ?? fromStore)) return <>{children}</>;
  return (
    <span
      className={cn("sensitive", className)}
      aria-label={kind === "share" ? t("privacy.hiddenShare") : t("privacy.hiddenAmount")}
      data-sensitive-mask=""
    >
      {SENSITIVE_MASK}
    </span>
  );
}
