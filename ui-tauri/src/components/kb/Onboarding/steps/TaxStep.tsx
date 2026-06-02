import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { formatFiatAmount } from "@/lib/currency";

import {
  FIAT_CURRENCIES,
  GAINS_ALGORITHM_DEFAULTS,
  gainsAlgorithmsFor,
  taxLongTermDaysHint,
} from "../constants";
import { DashboardIllustration } from "../DashboardIllustration";
import { NumberField, SelectField } from "../fields";
import {
  OnboardingStepActions,
  OnboardingStepFrame,
  OnboardingStepLeftWrapper,
  OnboardingStepRightWrapper,
} from "../frame";
import type { StepComponentProps, TaxCountry } from "../types";

export const TaxStep = ({
  form,
  update,
  currentStep,
  totalSteps,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const isAustrian = form.taxCountry === "at";

  return (
    <OnboardingStepFrame>
      <OnboardingStepLeftWrapper
        title="Choose tax defaults"
        eyebrow="Accounting"
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
          <div className="space-y-6">
            <SelectField
              label="Tax jurisdiction"
              value={form.taxCountry}
              options={["at", "generic"] as TaxCountry[]}
              optionLabels={{ at: "Austria", generic: "Other / generic" }}
              description={
                isAustrian
                  ? "Austrian crypto-tax workflow — EUR and moving-average rules apply."
                  : "Country-neutral books with FIFO, LIFO, HIFO, or LOFO lot selection."
              }
              onChange={(value) => {
                if (value === form.taxCountry) return;
                update("taxCountry", value);
                if (value === "at") {
                  update("fiatCurrency", "EUR");
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.at);
                } else {
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.generic);
                }
              }}
            />

            <div className="grid gap-4 sm:grid-cols-2">
              <SelectField
                label="Fiat currency"
                value={form.fiatCurrency}
                options={FIAT_CURRENCIES}
                description={`Sample: ${formatFiatAmount(1234.56, form.fiatCurrency)}`}
                onChange={(value) => update("fiatCurrency", value)}
              />
              {isAustrian ? (
                <div className="space-y-2">
                  <Label>Accounting method</Label>
                  <div className="flex h-9 items-center rounded-md border border-line bg-paper-2 px-3 text-sm text-ink">
                    Moving average
                  </div>
                  <p className="m-0 text-xs leading-5 text-ink-2">
                    Older holdings can be marked later per wallet.
                  </p>
                </div>
              ) : (
                <SelectField
                  label="Lot selection"
                  value={form.gainsAlgorithm}
                  options={gainsAlgorithmsFor(form.taxCountry)}
                  onChange={(value) => update("gainsAlgorithm", value)}
                />
              )}
            </div>
            {!isAustrian && (
              <NumberField
                label="Long-term holding days"
                name="taxLongTermDays"
                value={form.taxLongTermDays}
                placeholder="365"
                min={1}
                onChange={(value) => update("taxLongTermDays", value)}
                hint={taxLongTermDaysHint(form.taxLongTermDays)}
                description="Only used by generic books."
              />
            )}
          </div>

          <OnboardingStepActions>
            <Button type="submit" className="w-full" disabled={!canContinue}>
              Continue
            </Button>
          </OnboardingStepActions>
        </form>
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
