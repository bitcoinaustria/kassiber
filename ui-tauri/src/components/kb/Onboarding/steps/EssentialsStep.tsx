import { useTranslation } from "react-i18next";

import { formatFiatAmount } from "@/lib/currency";

import {
  FIAT_CURRENCIES,
  GAINS_ALGORITHM_DEFAULTS,
  gainsAlgorithmsFor,
  taxLongTermDaysHint,
} from "../constants";
import { NumberField, SelectField, TextField } from "../fields";
import {
  OnboardingSection,
  OnboardingStepActions,
  OnboardingStepPage,
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
  const { t } = useTranslation(["onboarding", "common"]);
  const isAustrian = form.taxCountry === "at";
  const methodLabels = {
    MOVING_AVERAGE_AT: t("books.method.MOVING_AVERAGE_AT"),
    MOVING_AVERAGE: t("books.method.MOVING_AVERAGE"),
    FIFO: t("books.method.FIFO"),
    LIFO: t("books.method.LIFO"),
    HIFO: t("books.method.HIFO"),
    LOFO: t("books.method.LOFO"),
  };

  return (
    <OnboardingStepPage
      eyebrow={t("frame.step", { current: currentStep + 1, total: totalSteps })}
      title={t("essentials.title")}
      lead={t("essentials.lead")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-8"
      >
        <TextField
          label={t("essentials.booksName")}
          name="workspace"
          value={form.workspace}
          placeholder={t("essentials.booksNamePlaceholder")}
          autoFocus
          description={t("essentials.booksNameDescription")}
          onChange={(value) => update("workspace", value)}
        />

        <OnboardingSection title={t("essentials.taxHeading")}>
          <div className="space-y-4 rounded-(--kb-radius-card) border border-border bg-card/60 p-4">
            <SelectField
              label={t("essentials.taxJurisdiction")}
              value={form.taxCountry}
              options={["at", "generic"] as TaxCountry[]}
              optionLabels={{
                at: t("essentials.jurisdictionAustria"),
                generic: t("essentials.jurisdictionGeneric"),
              }}
              description={
                isAustrian
                  ? t("essentials.jurisdictionDescriptionAt")
                  : t("essentials.jurisdictionDescriptionGeneric")
              }
              onChange={(value) => {
                if (value === form.taxCountry) return;
                update("taxCountry", value);
                if (value === "at") {
                  // Austrian books start on the jurisdiction's defaults; both
                  // stay editable right below.
                  update("fiatCurrency", "EUR");
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.at);
                } else if (
                  !gainsAlgorithmsFor(value).includes(form.gainsAlgorithm)
                ) {
                  // Only replace a method the new region cannot use.
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS.generic);
                }
              }}
            />

            <div className="grid gap-4 sm:grid-cols-2">
              <SelectField
                label={t("essentials.fiatCurrency")}
                value={form.fiatCurrency}
                options={FIAT_CURRENCIES}
                description={t("essentials.fiatSample", {
                  sample: formatFiatAmount(1234.56, form.fiatCurrency),
                })}
                onChange={(value) => update("fiatCurrency", value)}
              />
              <SelectField
                label={
                  isAustrian
                    ? t("essentials.accountingMethod")
                    : t("essentials.lotSelection")
                }
                value={form.gainsAlgorithm}
                options={gainsAlgorithmsFor(form.taxCountry)}
                optionLabels={methodLabels}
                description={
                  isAustrian ? t("essentials.movingAverageNote") : undefined
                }
                onChange={(value) => update("gainsAlgorithm", value)}
              />
            </div>
            {!isAustrian && (
              <NumberField
                label={t("essentials.longTermDays")}
                name="taxLongTermDays"
                value={form.taxLongTermDays}
                placeholder={t("essentials.longTermDaysPlaceholder")}
                min={1}
                onChange={(value) => update("taxLongTermDays", value)}
                hint={taxLongTermDaysHint(form.taxLongTermDays)}
                description={t("essentials.longTermDaysDescription")}
              />
            )}
          </div>
        </OnboardingSection>

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
