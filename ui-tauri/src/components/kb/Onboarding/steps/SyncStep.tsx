import { Button } from "@/components/ui/button";

import { ConnectionsFields } from "../ConnectionsFields";
import { SyncExplainer } from "../explainers";
import {
  OnboardingStepActions,
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { StepComponentProps } from "../types";

export const SyncStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose sync connections"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
          className="flex h-full flex-col justify-between gap-6 py-4"
        >
          <ConnectionsFields form={form} update={update} />

          <OnboardingStepActions>
            <Button type="submit" className="w-full" disabled={!canContinue}>
              Continue
            </Button>
          </OnboardingStepActions>
        </form>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <SyncExplainer form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
