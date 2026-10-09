import { ShieldCheck } from "lucide-react";
import { useTranslation } from "react-i18next";

import { ConnectionsFields } from "../ConnectionsFields";
import { CheckRow } from "../fields";
import {
  OnboardingNote,
  OnboardingSection,
  OnboardingStepActions,
  OnboardingStepPage,
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
  const offline = form.backendSetupMode === "skip";
  return (
    <OnboardingStepPage
      eyebrow={t("frame.step", { current: currentStep + 1, total: totalSteps })}
      title={t("sync.title")}
      lead={t("sync.lead")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-8"
      >
        <ConnectionsFields form={form} update={update} />

        <OnboardingSection title={t("sync.updateChecksHeading")}>
          {/* Offline setup blocks update checks without forgetting the
              user's answer, so switching back online restores it. */}
          <CheckRow
            id="allow-update-checks"
            checked={form.updateChecksEnabled && !offline}
            disabled={offline}
            onCheckedChange={(checked) =>
              update("updateChecksEnabled", checked)
            }
            label={t("sync.updateChecks")}
            description={t("sync.updateChecksDescription")}
          />
        </OnboardingSection>

        <OnboardingNote icon={ShieldCheck}>{t("sync.footnote")}</OnboardingNote>

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
