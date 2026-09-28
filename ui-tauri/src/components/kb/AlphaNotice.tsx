import * as React from "react";
import { AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

/**
 * The alpha warning, as a chip in the title bar.
 *
 * It used to be a full-width red strip under the title bar, which pushed every
 * screen down by a row and split the window chrome in two. The chip keeps the
 * warning in view on every screen without costing a row; the full wording and
 * the way to hide it sit one click away.
 */
export function AlphaNotice({ className }: { className?: string }) {
  const { t } = useTranslation("chrome");
  const setPreAlphaBannerVisible = useUiStore(
    (state) => state.setPreAlphaBannerVisible,
  );
  const [open, setOpen] = React.useState(false);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={t("preAlpha.label")}
          title={t("preAlpha.label")}
          className={cn(
            "inline-flex h-6 shrink-0 items-center gap-1 rounded-full border border-(--kb-accent)/35 bg-(--kb-accent)/10 px-2 text-xs font-medium text-(--kb-accent-text) transition-colors hover:bg-(--kb-accent)/15 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
            className,
          )}
        >
          <AlertTriangle className="size-3 shrink-0" aria-hidden="true" />
          {t("preAlpha.chip")}
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 space-y-3 text-sm">
        <div className="space-y-1">
          <p className="font-medium">{t("preAlpha.title")}</p>
          <p className="text-muted-foreground">{t("preAlpha.body")}</p>
        </div>
        <div className="flex items-end justify-between gap-3">
          <p className="text-xs text-muted-foreground">
            {t("preAlpha.hideHint")}
          </p>
          <Button
            type="button"
            variant="outline"
            size="xs"
            className="shrink-0"
            onClick={() => {
              setOpen(false);
              setPreAlphaBannerVisible(false);
            }}
          >
            {t("preAlpha.hide")}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
