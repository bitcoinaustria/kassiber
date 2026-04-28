import { Button } from "@/components/ui/button";

import {
  FIAT_CURRENCIES,
  GAINS_ALGORITHM_DEFAULTS,
  gainsAlgorithmsFor,
  taxLongTermDaysHint,
} from "../constants";
import { DashboardIllustration } from "../DashboardIllustration";
import { ChoiceCard, NumberField, SelectField } from "../fields";
import {
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { StepComponentProps } from "../types";

export const TaxStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
}: StepComponentProps) => {
  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose tax defaults"
        eyebrow="Accounting"
        currentStep={currentStep}
        totalSteps={totalSteps}
        goBack={goBack}
      >
        <div className="flex h-full flex-col justify-between gap-6 py-4">
          <div className="space-y-6">
            <div className="space-y-3">
              <ChoiceCard
                active={form.taxCountry === "at"}
                title="Austria"
                description="EUR defaults, section 27b buckets, and moving average semantics through rp2's AT plugin."
                onClick={() => {
                  if (form.taxCountry === "at") return;
                  update("taxCountry", "at");
                  update("fiatCurrency", "EUR");
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.at);
                }}
              />
              <ChoiceCard
                active={form.taxCountry === "generic"}
                title="Generic"
                description="Country-neutral FIFO/LIFO/HIFO/LOFO profile for non-Austrian workflows."
                onClick={() => {
                  if (form.taxCountry === "generic") return;
                  update("taxCountry", "generic");
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.generic);
                }}
              />
            </div>

            <div className="grid gap-4 sm:grid-cols-2">
              <SelectField
                label="Fiat currency"
                value={form.fiatCurrency}
                options={FIAT_CURRENCIES}
                onChange={(value) => update("fiatCurrency", value)}
              />
              <SelectField
                label="Lot selection"
                value={form.gainsAlgorithm}
                options={gainsAlgorithmsFor(form.taxCountry)}
                onChange={(value) => update("gainsAlgorithm", value)}
              />
            </div>
            <NumberField
              label="Long-term holding days"
              name="taxLongTermDays"
              value={form.taxLongTermDays}
              placeholder="365"
              min={1}
              onChange={(value) => update("taxLongTermDays", value)}
              hint={taxLongTermDaysHint(form.taxLongTermDays)}
            />
          </div>

          <Button onClick={onSubmit} className="w-full">
            Continue
          </Button>
        </div>
      </OnboardingStepLeftWrapper>
      <OnboardingStepRightWrapper className="px-8 py-10">
        <DashboardIllustration
          form={form}
          variant="zoomed-in"
          transformOrigin="10% 10%"
        />
      </OnboardingStepRightWrapper>
    </OnboardingStepFrame>
  );
};
