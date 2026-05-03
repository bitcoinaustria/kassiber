import { Button } from "@/components/ui/button";

import { DashboardIllustration } from "../DashboardIllustration";
import { TextField } from "../fields";
import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { StepComponentProps } from "../types";

export const IdentityStep = ({
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
        title="Name this ledger"
        eyebrow="Ledger"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <form
          onSubmit={(event) => event.preventDefault()}
          className="space-y-6 py-4"
        >
          <div className="space-y-4 border-b border-line pb-6">
            <TextField
              label="Ledger name"
              name="workspace"
              value={form.workspace}
              placeholder="Personal ledger"
              description="This is the local ledger shown in the app header."
              onChange={(value) => update("workspace", value)}
            />
            <details className="group rounded-md border border-line bg-paper-2 px-3 py-2">
              <summary className="cursor-pointer text-sm font-medium text-ink marker:text-ink-3">
                More setup options
              </summary>
              <div className="pt-4">
                <TextField
                  label="Books label"
                  name="profile"
                  value={form.profile}
                  placeholder="main"
                  onChange={(value) => update("profile", value)}
                />
                <p className="m-0 mt-2 text-xs leading-5 text-ink-2">
                  Keep the default unless you plan to split this ledger into
                  separate private, business, or tax books later.
                </p>
              </div>
            </details>
          </div>

          <Button
            type="submit"
            onClick={onSubmit}
            className="mt-4 w-full"
            disabled={!canContinue}
          >
            Continue
          </Button>
        </form>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DashboardIllustration form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
