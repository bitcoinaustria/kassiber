import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { macTitleBarInset } from "@/lib/titleBarInset";
import { cn } from "@/lib/utils";

/**
 * The window's top row, always 28px tall.
 *
 * `muted` keeps the strip and drops the warning: same height, window background,
 * no text. The strip has to stay either way, because it is what the macOS
 * traffic lights sit on — without it they would land on the nav's own top row
 * and every control there would have to dodge them.
 */
export function PreAlphaBanner({
  className,
  muted = false,
}: {
  className?: string;
  muted?: boolean;
}) {
  const { t } = useTranslation("chrome");
  return (
    <section
      role={muted ? undefined : "status"}
      aria-label={muted ? undefined : t("preAlpha.label")}
      // The traffic lights land on this strip: pad both sides to keep the message
      // centred and clear of them, and let a drag on the strip move the window.
      data-tauri-drag-region={macTitleBarInset ? "" : undefined}
      className={cn(
        "flex h-[28px] w-full items-center justify-center gap-1.5 px-3 text-center text-xs font-medium",
        muted ? "bg-sidebar" : "bg-[#E3000F] text-white",
        macTitleBarInset && "px-[72px]",
        className,
      )}
    >
      {muted ? null : (
        <>
          <AlertTriangle className="size-3.5 shrink-0" aria-hidden="true" />
          <span>{t("preAlpha.message")}</span>
        </>
      )}
    </section>
  );
}
