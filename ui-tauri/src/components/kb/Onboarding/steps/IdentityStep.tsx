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
  goBack,
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Name your local ledger"
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
              placeholder="Personal"
              onChange={(value) => update("workspace", value)}
            />
            <details className="group rounded-md border border-line bg-paper-2 px-3 py-2">
              <summary className="cursor-pointer text-sm font-medium text-ink marker:text-ink-3">
                Advanced tax profile
              </summary>
              <div className="pt-4">
                <TextField
                  label="Profile label"
                  name="profile"
                  value={form.profile}
                  placeholder="main"
                  onChange={(value) => update("profile", value)}
                />
                <p className="m-0 mt-2 text-xs leading-5 text-ink-2">
                  Most ledgers only need the default profile. Add more profiles
                  later when a separate tax context is useful.
                </p>
              </div>
            </details>
          </div>

          <div className="flex items-start gap-3 rounded-lg border border-line bg-paper-2 p-3 text-xs leading-5 text-ink-2">
            <WalletCards className="mt-0.5 size-4 shrink-0 text-ink" />
            <p className="m-0">
              Opening a ledger creates the first wallet/reporting bucket named
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
