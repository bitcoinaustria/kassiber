import { WalletCards } from "lucide-react";

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
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Set up your local workspace"
        eyebrow="Identity"
        currentStep={currentStep}
        totalSteps={totalSteps}
      >
        <form
          onSubmit={(event) => event.preventDefault()}
          className="space-y-6 py-4"
        >
          <div className="space-y-4 border-b border-line pb-6">
            <TextField
              label="Your name"
              name="name"
              value={form.name}
              placeholder="Alice"
              onChange={(value) => update("name", value)}
            />
            <TextField
              label="Workspace name"
              name="workspace"
              value={form.workspace}
              placeholder="Personal"
              onChange={(value) => update("workspace", value)}
            />
            <TextField
              label="Profile"
              name="profile"
              value={form.profile}
              placeholder="main"
              onChange={(value) => update("profile", value)}
            />
          </div>

          <div className="flex items-start gap-3 rounded-lg border border-line bg-paper-2 p-3 text-xs leading-5 text-ink-2">
            <WalletCards className="mt-0.5 size-4 shrink-0 text-ink" />
            <p className="m-0">
              Creating a profile seeds the first wallet/reporting bucket named
              <span className="font-mono text-ink"> treasury</span>. This is a
              bucket, not a double-entry chart of accounts.
            </p>
          </div>

          <Button type="submit" onClick={onSubmit} className="mt-4 w-full">
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
