import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export function PreAlphaBanner({ className }: { className?: string }) {
  const { t } = useTranslation("chrome");
  return (
    <section
      role="status"
      aria-label={t("preAlpha.label")}
      className={cn(
        "flex h-6 w-full items-center justify-center gap-1.5 bg-[#E3000F] px-3 text-center text-xs font-medium text-white",
        className,
      )}
    >
      <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
      <span>{t("preAlpha.message")}</span>
    </section>
  );
}
