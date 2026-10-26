import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

import { useAssistantSession } from "./assistantSession";

/** The item pinned by "Ask about this", removable before or between turns. */
export function AssistantFocusChip({ className }: { className?: string }) {
  const { t } = useTranslation("assistant");
  const { focus, clearFocus } = useAssistantSession();
  if (!focus) return null;
  return (
    <div className={cn("flex items-center gap-1.5 px-1 text-xs", className)}>
      <span className="text-muted-foreground">{t("panel.focusLabel")}</span>
      <span
        className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/70 bg-muted/60 py-0.5 pr-0.5 pl-2 font-medium text-foreground"
        data-assistant-focus
      >
        <span className="truncate">{focus.label}</span>
        <button
          type="button"
          className="flex size-4 shrink-0 items-center justify-center rounded-full text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={t("panel.removeFocus", { label: focus.label })}
          onClick={clearFocus}
        >
          <X className="size-3" aria-hidden="true" />
        </button>
      </span>
    </div>
  );
}
