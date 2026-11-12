/*
 * Setup's progress track, laid out after T3 Code's wizard steps
 * (apps/web/src/components/ui/wizard.tsx, MIT, Copyright (c) 2026 T3 Tools
 * Inc.): a segmented track of numbered steps, the current one raised.
 */
import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export interface StepperEntry {
  /** Falls back to the position when absent. */
  label?: string;
  /** What the user chose on this step, read out once it is behind them. */
  summary?: string;
}

interface OnboardingStepperProps {
  steps: StepperEntry[];
  current: number;
  /** Jump back to an already-completed step. Forward steps stay locked. */
  onJump?: (index: number) => void;
}

/**
 * Completed steps are buttons back to that step; the current and upcoming
 * ones are not, since later steps may still be incomplete. Below `sm` only
 * the numbers show.
 */
export const OnboardingStepper = ({
  steps,
  current,
  onJump,
}: OnboardingStepperProps) => {
  const { t } = useTranslation("onboarding");
  return (
    <nav aria-label={t("stepper.ariaLabel")}>
      <ol className="grid list-none auto-cols-fr grid-flow-col gap-1 rounded-(--kb-radius-card) bg-background p-1 ring-1 ring-border/70">
        {steps.map((step, index) => {
          const text =
            step.label ?? t("stepper.stepFallback", { number: index + 1 });
          const done = index < current;
          const active = index === current;
          return (
            <li key={`${text}-${index}`} className="min-w-0">
              <button
                type="button"
                disabled={!done}
                aria-current={active ? "step" : undefined}
                // Named on every step: below `sm` the label is hidden and the
                // number is decorative, so this is the only name it has.
                aria-label={
                  done && step.summary ? `${text}: ${step.summary}` : text
                }
                onClick={() => done && onJump?.(index)}
                className={cn(
                  "flex w-full min-w-0 items-center gap-2 rounded-(--kb-radius-inset) px-2.5 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset max-sm:justify-center max-sm:px-2",
                  done ? "cursor-pointer hover:bg-card" : "cursor-default",
                  active && "bg-card shadow-xs ring-1 ring-border ring-inset",
                )}
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    "grid size-5 shrink-0 place-items-center rounded-full text-xs font-medium ring-1",
                    done
                      ? "bg-primary text-primary-foreground ring-primary"
                      : active
                        ? "bg-primary/10 text-ink ring-primary/30"
                        : "bg-card text-ink-3 ring-border",
                  )}
                >
                  {done ? <Check className="size-3.5" /> : index + 1}
                </span>
                <span
                  className={cn(
                    "min-w-0 truncate text-sm font-medium max-sm:hidden",
                    active ? "text-ink" : "text-ink-3",
                  )}
                >
                  {text}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
};
