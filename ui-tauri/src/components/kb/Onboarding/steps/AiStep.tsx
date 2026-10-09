import { ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AiFields } from "../AiFields";
import {
  OnboardingNote,
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
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <OnboardingStepPage
      eyebrow={t("frame.step", { current: currentStep + 1, total: totalSteps })}
      title={t("aiStep.title")}
      lead={t("aiStep.lead")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-8"
      >
        <AiFields form={form} update={update} />

        {form.backendSetupMode !== "skip" && (
          <OnboardingNote icon={ShieldCheck}>
            {t("aiStep.footnote")}
          </OnboardingNote>
        )}

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
