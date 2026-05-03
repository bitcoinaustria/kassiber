import type { ReactNode } from "react";
import { ChevronLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface OnboardingStepHeaderProps {
  title: string;
  eyebrow: string;
  stepIndex: number;
  totalSteps: number;
  goBack?: () => void;
  showProgress?: boolean;
}

export const OnboardingStepHeader = ({
  title,
  eyebrow,
  stepIndex,
  totalSteps,
  goBack,
  showProgress = true,
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
        <p className="font-mono text-[10px] font-medium uppercase tracking-[0.14em] text-ink-3">
          {eyebrow}
          {showProgress ? ` · ${stepIndex + 1}/${totalSteps}` : null}
        </p>
        <h3 className="mt-2 text-2xl font-semibold tracking-normal text-ink md:whitespace-nowrap">
          {title}
        </h3>
      </div>
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

export const OnboardingStepLeftWrapper = ({
  title,
  eyebrow,
  currentStep,
  totalSteps,
  goBack,
  showProgress,
  children,
}: {
  title: string;
  eyebrow: string;
  currentStep: number;
  totalSteps: number;
  children: ReactNode;
  goBack?: () => void;
  showProgress?: boolean;
}) => {
  return (
    <div className="flex flex-1/2 justify-center px-5 py-6 sm:px-10 sm:py-10 md:py-16 lg:justify-start lg:pl-20">
      <div className="flex h-full w-full max-w-md shrink-0 flex-col gap-6">
        <OnboardingStepHeader
          title={title}
          eyebrow={eyebrow}
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
