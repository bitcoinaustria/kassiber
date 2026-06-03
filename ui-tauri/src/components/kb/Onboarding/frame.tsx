import type { ReactNode } from "react";
import { ChevronLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface OnboardingStepHeaderProps {
  title: string;
  stepIndex: number;
  totalSteps: number;
  goBack?: () => void;
  showProgress?: boolean;
}

export const OnboardingStepHeader = ({
  title,
  stepIndex,
  totalSteps,
  goBack,
  showProgress = false,
}: OnboardingStepHeaderProps) => {
  return (
    <div className="flex items-start gap-2">
      {goBack && (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={goBack}
          className="-ml-2 shrink-0 text-ink-2"
          aria-label="Go back"
        >
          <ChevronLeft className="size-4" />
        </Button>
      )}
      <div>
        {showProgress && (
          <p className="mb-2 font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-ink-3">
            Step {stepIndex + 1}/{totalSteps}
          </p>
        )}
        <h3 className="text-2xl font-semibold tracking-normal text-ink md:whitespace-nowrap">
          {title}
        </h3>
      </div>
    </div>
  );
};

/**
 * Pins the primary step action to the bottom of the viewport while the form
 * fields scroll, so "Continue" / "Open books" stays reachable on tall steps
 * (e.g. the custom Electrum backend or the encrypted-database step). A short
 * gradient mask fades scrolling content into the action bar.
 */
export const OnboardingStepActions = ({
  children,
}: {
  children: ReactNode;
}) => {
  return (
    <div className="sticky bottom-0 z-10 mt-2 bg-paper pt-4">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 -top-6 h-6 bg-gradient-to-t from-paper to-transparent"
      />
      {children}
    </div>
  );
};

export const OnboardingStepFrame = ({ children }: { children: ReactNode }) => {
  return (
    <div className="flex w-full flex-col-reverse gap-8 rounded-lg border border-line bg-paper md:min-h-[78dvh] md:flex-row lg:rounded-lg">
      {children}
    </div>
  );
};

/**
 * Focused single-column card for steps that don't need a side explainer
 * (naming the workspace, locking the database). Renders the standard step
 * header plus its content in a centered, comfortably narrow column.
 */
export const OnboardingSingleColumnFrame = ({
  title,
  currentStep,
  totalSteps,
  goBack,
  showProgress,
  className,
  children,
}: {
  title: string;
  currentStep: number;
  totalSteps: number;
  children: ReactNode;
  goBack?: () => void;
  showProgress?: boolean;
  className?: string;
}) => {
  return (
    <div
      className={cn(
        "mx-auto flex w-full max-w-xl flex-col gap-6 rounded-lg border border-line bg-paper px-6 py-8 sm:px-10 sm:py-10",
        className,
      )}
    >
      <OnboardingStepHeader
        title={title}
        stepIndex={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
        showProgress={showProgress}
      />
      {children}
    </div>
  );
};

export const OnboardingStepLeftWrapper = ({
  title,
  currentStep,
  totalSteps,
  goBack,
  showProgress,
  children,
}: {
  title: string;
  currentStep: number;
  totalSteps: number;
  children: ReactNode;
  goBack?: () => void;
  showProgress?: boolean;
}) => {
  return (
    <div className="flex flex-1/2 justify-center px-5 py-6 sm:px-10 sm:py-8 md:py-10 lg:justify-start lg:pl-20">
      <div className="flex h-full w-full max-w-md shrink-0 flex-col gap-6">
        <OnboardingStepHeader
          title={title}
          stepIndex={currentStep}
          totalSteps={totalSteps}
          goBack={goBack}
          showProgress={showProgress}
        />
        {children}
      </div>
    </div>
  );
};

export const OnboardingStepRightWrapper = ({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) => {
  return (
    <div
      className={cn(
        "hidden flex-1/2 overflow-hidden border-b border-line bg-paper-2 md:block md:border-b-0 md:border-l",
        className,
      )}
    >
      {children}
    </div>
  );
};
