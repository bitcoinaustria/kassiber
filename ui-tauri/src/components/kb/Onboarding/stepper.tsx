import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

interface OnboardingStepperProps {
  /** Ordered step labels; falls back to the position when a label is absent. */
  labels: Array<string | undefined>;
  current: number;
  /** Jump back to an already-completed step. Forward steps stay locked. */
  onJump?: (index: number) => void;
}

/**
 * Compact labeled progress indicator for the setup wizard. Completed steps are
 * clickable to go back; the active and upcoming steps are not, since later
 * steps may still be incomplete.
 */
export const OnboardingStepper = ({
  labels,
  current,
  onJump,
}: OnboardingStepperProps) => {
  const { t } = useTranslation("onboarding");
  return (
    <nav aria-label={t("stepper.ariaLabel")} className="w-full max-w-2xl">
      <ol className="flex items-center">
        {labels.map((label, index) => {
          const text = label ?? t("stepper.stepFallback", { number: index + 1 });
          const done = index < current;
          const active = index === current;
          const isLast = index === labels.length - 1;
          return (
            <li
              key={`${text}-${index}`}
              className={cn("flex items-center", !isLast && "flex-1")}
            >
              <button
                type="button"
                disabled={!done}
                aria-current={active ? "step" : undefined}
                onClick={() => done && onJump?.(index)}
                className={cn(
                  "flex shrink-0 items-center gap-2 rounded-md outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50",
                  done ? "cursor-pointer hover:opacity-80" : "cursor-default",
                )}
              >
                <span
                  className={cn(
                    "flex size-6 shrink-0 items-center justify-center rounded-full border text-[11px] font-semibold transition-colors",
                    done
                      ? "border-ink bg-ink text-paper"
                      : active
                        ? "border-ink text-ink"
                        : "border-line text-ink-3",
                  )}
                >
                  {done ? <Check className="size-3.5" /> : index + 1}
                </span>
                <span
                  className={cn(
                    "hidden text-xs font-medium sm:inline",
                    active
                      ? "text-ink"
                      : done
                        ? "text-ink-2"
                        : "text-ink-3",
                  )}
                >
                  {text}
                </span>
              </button>
              {!isLast && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "mx-2 h-px flex-1 transition-colors",
                    done ? "bg-ink" : "bg-line",
                  )}
                />
              )}
            </li>
          );
        })}
      </ol>
    </nav>
  );
};
