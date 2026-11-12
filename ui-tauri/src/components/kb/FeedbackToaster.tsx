import { AlertTriangle, CheckCircle2, CircleAlert, Info, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Toast,
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastTitle,
  ToastViewport,
} from "@/components/ui/toast";
import {
  dismissFeedback,
  useFeedbackStore,
  type FeedbackTone,
} from "@/lib/feedback";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { projectFreeText } from "./notificationPrivacy";

const TONE_ICON = {
  error: CircleAlert,
  warning: AlertTriangle,
  success: CheckCircle2,
  info: Info,
} satisfies Record<FeedbackTone, unknown>;

const TONE_ICON_CLASS: Record<FeedbackTone, string> = {
  error: "text-red-700 dark:text-red-300",
  warning: "text-amber-700 dark:text-amber-300",
  success: "text-emerald-700 dark:text-emerald-300",
  info: "text-muted-foreground",
};

/**
 * The toast layer for `showFeedback`. The shell mounts it only while
 * unlocked, so nothing shows over the lock screen.
 */
export function FeedbackToaster() {
  const { t } = useTranslation("chrome");
  const items = useFeedbackStore((s) => s.items);
  const privacyMode = useUiStore((s) => s.privacyMode);

  return (
    <ToastProvider swipeDirection="right" label={t("feedback.region")}>
      {items.map((item) => {
        const Icon = TONE_ICON[item.tone];
        const description =
          item.descriptionPrivacy === "safe"
            ? item.description
            : projectFreeText(item.description, privacyMode, t);
        return (
          <Toast
            // A coalesced receipt remounts, which restarts its timer.
            key={`${item.id}:${item.revision}`}
            type={item.tone === "error" ? "foreground" : "background"}
            duration={item.durationMs}
            onOpenChange={(open) => {
              if (!open) dismissFeedback(item.id);
            }}
            data-testid="feedback-toast"
          >
            <Icon
              className={cn("mt-0.5 size-4 shrink-0", TONE_ICON_CLASS[item.tone])}
              aria-hidden="true"
            />
            <div className="min-w-0 flex-1">
              <ToastTitle>
                {item.title}
                {item.count > 1 ? (
                  <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                    {t("notifications.repeatBadge", { count: item.count })}
                  </span>
                ) : null}
              </ToastTitle>
              {description ? (
                <ToastDescription>{description}</ToastDescription>
              ) : null}
            </div>
            <ToastClose asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                className="shrink-0 text-muted-foreground"
                aria-label={t("feedback.dismiss")}
              >
                <X aria-hidden="true" />
              </Button>
            </ToastClose>
          </Toast>
        );
      })}
      <ToastViewport />
    </ToastProvider>
  );
}
