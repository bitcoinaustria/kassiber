import { useTranslation } from "react-i18next";

import { AiFields } from "../AiFields";
import {
  OnboardingStepActions,
  OnboardingStepPage,
} from "../frame";
import type { StepComponentProps } from "../types";

export const AiStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <OnboardingStepPage
      title={t("aiStep.title")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        <AiFields form={form} update={update} />

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
