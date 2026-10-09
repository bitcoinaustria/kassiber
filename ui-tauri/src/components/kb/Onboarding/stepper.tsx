import { Check } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

export interface StepperEntry {
  /** Falls back to the position when absent. */
  label?: string;
  /** What the user chose on this step, shown once the step is behind them. */
  summary?: string;
}

interface OnboardingStepperProps {
  steps: StepperEntry[];
  current: number;
  /** Jump back to an already-completed step. Forward steps stay locked. */
  onJump?: (index: number) => void;
}

/**
 * The setup rail's progress list. Each completed step shows the choice made on
 * it, so the rail doubles as a running receipt of the setup, and is clickable
 * to go back and change it. The active and upcoming steps are not clickable,
 * since later steps may still be incomplete.
 */
export const OnboardingStepper = ({
  steps,
  current,
  onJump,
}: OnboardingStepperProps) => {
  const { t } = useTranslation("onboarding");
  return (
    <nav aria-label={t("stepper.ariaLabel")}>
      <ol className="list-none space-y-1 p-0">
        {steps.map((step, index) => {
          const text =
            step.label ?? t("stepper.stepFallback", { number: index + 1 });
          const done = index < current;
          const active = index === current;
          const isLast = index === steps.length - 1;
          return (
            <li key={`${text}-${index}`} className="relative">
              {!isLast && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "absolute top-9 left-[1.375rem] h-[calc(100%-1.75rem)] w-px -translate-x-1/2 transition-colors",
                    done ? "bg-ink/50" : "bg-border",
                  )}
                />
              )}
              <button
                type="button"
                disabled={!done}
                aria-current={active ? "step" : undefined}
                onClick={() => done && onJump?.(index)}
                className={cn(
                  "relative flex w-full items-start gap-3 rounded-(--kb-radius-inset) px-2.5 py-2 text-left outline-none transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/50",
                  done && "cursor-pointer hover:bg-[var(--sidebar-row-hover)]",
                  active && "bg-[var(--sidebar-row-active)]",
                  !done && "cursor-default",
                )}
              >
                <span
                  className={cn(
                    "flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-semibold transition-colors",
                    done
                      ? "bg-ink text-paper"
                      : active
                        ? "bg-card text-ink ring-[1.5px] ring-ink"
                        : "bg-card text-ink-3 ring-1 ring-border",
                  )}
                >
                  {done ? (
                    <Check className="size-3.5" aria-hidden="true" />
                  ) : active ? (
                    <span className="size-2 rounded-full bg-[var(--kb-accent)]" />
                  ) : (
                    index + 1
                  )}
                </span>
                <span className="min-w-0 flex-1 pt-0.5">
                  <span
                    className={cn(
                      "block text-sm leading-5 font-medium",
                      active || done ? "text-ink" : "text-ink-3",
                    )}
                  >
                    {text}
                  </span>
                  {done && step.summary && (
                    <span className="mt-0.5 block truncate text-xs leading-4 text-ink-3">
                      {step.summary}
                    </span>
                  )}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </nav>
  );
};

/**
 * Compact progress for windows too narrow for the rail: the position, the
 * active step's name, and one segment per step.
 */
export const OnboardingProgressBar = ({
  steps,
  current,
}: {
  steps: StepperEntry[];
  current: number;
}) => {
  const { t } = useTranslation("onboarding");
  const active = steps[current];
  return (
    <div className="space-y-2">
      <p className="text-xs text-ink-2">
        <span className="font-mono text-2xs uppercase tracking-[0.14em] text-ink-3">
          {t("frame.step", { current: current + 1, total: steps.length })}
        </span>
        {active?.label && (
          <span className="ml-2 font-medium text-ink">{active.label}</span>
        )}
      </p>
      <div aria-hidden="true" className="flex gap-1">
        {steps.map((step, index) => (
          <span
            key={`${step.label ?? index}-${index}`}
            className={cn(
              "h-1 flex-1 rounded-full transition-colors",
              index < current
                ? "bg-ink"
                : index === current
                  ? "bg-[var(--kb-accent)]"
                  : "bg-border",
            )}
          />
        ))}
      </div>
    </div>
  );
};
