import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { macTitleBarInset } from "@/lib/titleBarInset";
import { cn } from "@/lib/utils";

export function PreAlphaBanner({ className }: { className?: string }) {
  const { t } = useTranslation("chrome");
  return (
    <section
      role="status"
      aria-label={t("preAlpha.label")}
      // While the banner is shown it is the window's top row, so the traffic
      // lights land on it: pad both sides to keep the message centred and clear
      // of them, and let a drag on the strip move the window.
      data-tauri-drag-region={macTitleBarInset ? "" : undefined}
      className={cn(
        "flex h-[28px] w-full items-center justify-center gap-1.5 bg-[#E3000F] px-3 text-center text-xs font-medium text-white",
        macTitleBarInset && "px-[72px]",
        className,
      )}
    >
      <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
      <span>{t("preAlpha.message")}</span>
    </section>
  );
}
