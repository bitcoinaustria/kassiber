import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";

import { ConnectionsFields } from "../ConnectionsFields";
import { SyncExplainer } from "../explainers";
import { CheckRow } from "../fields";
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
  const { t } = useTranslation(["onboarding", "common"]);
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title={t("sync.title")}
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
          <div className="space-y-5">
            <ConnectionsFields form={form} update={update} />

            <section className="space-y-3 border-t border-line pt-5">
              <h2 className="m-0 text-sm font-semibold text-ink">
                {t("sync.updateChecksHeading")}
              </h2>
              <CheckRow
                id="allow-update-checks"
                checked={form.updateChecksEnabled}
                onCheckedChange={(checked) =>
                  update("updateChecksEnabled", checked)
                }
                label={t("sync.updateChecks")}
                description={t("sync.updateChecksDescription")}
              />
            </section>
          </div>

          <OnboardingStepActions>
            <Button type="submit" className="w-full" disabled={!canContinue}>
              {t("common:actions.continue")}
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
