import { Button } from "@/components/ui/button";

import { DashboardIllustration } from "../DashboardIllustration";
import { TextField } from "../fields";
import {
  OnboardingStepActions,
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
        title="Name your workspace"
        eyebrow="Workspace"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <form
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
          className="space-y-6 py-4"
        >
          <div className="space-y-4 border-b border-line pb-6">
            <TextField
              label="Workspace name"
              name="workspace"
              value={form.workspace}
              placeholder="My Books"
              autoFocus
              description="Groups all your books. Shown in the app header."
              onChange={(value) => update("workspace", value)}
            />
            <details className="group rounded-md border border-line bg-paper-2 px-3 py-2">
              <summary className="cursor-pointer text-sm font-medium text-ink marker:text-ink-3">
                More setup options
              </summary>
              <div className="pt-4">
                <TextField
                  label="Profile name"
                  name="profile"
                  value={form.profile}
                  placeholder="Private"
                  onChange={(value) => update("profile", value)}
                />
                <p className="m-0 mt-2 text-xs leading-5 text-ink-2">
                  A profile is one set of books inside this workspace (e.g.
                  private or business) and carries its own tax defaults. Add
                  more later from Settings.
                </p>
              </div>
            </details>
          </div>

          <OnboardingStepActions>
            <Button type="submit" className="w-full" disabled={!canContinue}>
              Continue
            </Button>
          </OnboardingStepActions>
        </form>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DashboardIllustration form={form} />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
