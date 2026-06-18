import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

import { AiFields } from "../AiFields";
import { AiExplainer } from "../explainers";
import {
  OnboardingStepActions,
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { StepComponentProps } from "../types";

export const AiStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title={t("aiStep.title")}
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
          <AiFields form={form} update={update} />

          <OnboardingStepActions>
            <Button type="submit" className="w-full" disabled={!canContinue}>
              {t("common:actions.continue")}
            </Button>
          </OnboardingStepActions>
        </form>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <AiExplainer form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
