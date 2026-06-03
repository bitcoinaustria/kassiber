import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { formatFiatAmount } from "@/lib/currency";

import {
  FIAT_CURRENCIES,
  GAINS_ALGORITHM_DEFAULTS,
  gainsAlgorithmsFor,
  taxLongTermDaysHint,
} from "../constants";
import { NumberField, SelectField, TextField } from "../fields";
import {
  OnboardingSingleColumnFrame,
  OnboardingStepActions,
} from "../frame";
import type { StepComponentProps, TaxCountry } from "../types";

export const EssentialsStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
  currentStep,
  totalSteps,
}: StepComponentProps) => {
  const isAustrian = form.taxCountry === "at";

  return (
    <OnboardingSingleColumnFrame
      title="Set up your books"
      eyebrow="Your books"
      currentStep={currentStep}
      totalSteps={totalSteps}
      goBack={goBack}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
      >
        <TextField
          label="Workspace name"
          name="workspace"
          value={form.workspace}
          placeholder="My Books"
          autoFocus
          description="Groups all your books. Shown in the app header."
          onChange={(value) => update("workspace", value)}
        />

        <div className="space-y-4 border-y border-line py-6">
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

        <details className="group rounded-lg border border-line bg-paper px-4 py-3">
          <summary className="cursor-pointer list-none text-sm font-medium text-ink">
            More options
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
              A profile is one set of books inside this workspace (e.g. private
              or business) and carries its own tax defaults. Add more later from
              Settings.
            </p>
          </div>
        </details>

        <OnboardingStepActions>
          <Button type="submit" className="w-full" disabled={!canContinue}>
            Continue
          </Button>
        </OnboardingStepActions>
      </form>
    </OnboardingSingleColumnFrame>
  );
};
