import { useTranslation } from "react-i18next";

import { formatFiatAmount } from "@/lib/currency";

import {
  TaxCountryLabel,
  TaxCountryPreviewNotice,
} from "@/components/kb/TaxCountryPreview";
import { requiredFiatCurrency, usesLongTermDays } from "@/lib/taxCountries";

import {
  FIAT_CURRENCIES,
  GAINS_ALGORITHM_DEFAULTS,
  TAX_COUNTRIES,
  gainsAlgorithmsFor,
  taxLongTermDaysHint,
} from "../constants";
import { onboardingRegionLabel } from "../regionLabel";
import { NumberField, SelectField, TextField } from "../fields";
import {
  OnboardingSection,
  OnboardingStepActions,
  OnboardingStepPage,
} from "../frame";
import type { FiatCurrency, StepComponentProps } from "../types";

export const EssentialsStep = ({
  form,
  update,
  onSubmit,
  goBack,
  canContinue = true,
}: StepComponentProps) => {
  const { t, i18n } = useTranslation(["onboarding", "common"]);
  const isAustrian = form.taxCountry === "at";
  const methodLabels = {
    MOVING_AVERAGE_AT: t("books.method.MOVING_AVERAGE_AT"),
    MOVING_AVERAGE: t("books.method.MOVING_AVERAGE"),
    MOVING_AVERAGE_FIFO_PERIOD: t("books.method.MOVING_AVERAGE_FIFO_PERIOD"),
    FIFO: t("books.method.FIFO"),
    LIFO: t("books.method.LIFO"),
    HIFO: t("books.method.HIFO"),
    LOFO: t("books.method.LOFO"),
    UK_SHARE_MATCHING: t("books.method.UK_SHARE_MATCHING"),
    JP_TOTAL_AVERAGE: t("books.method.JP_TOTAL_AVERAGE"),
    JP_MOVING_AVERAGE: t("books.method.JP_MOVING_AVERAGE"),
  };

  return (
    <OnboardingStepPage
      title={t("essentials.title")}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          onSubmit();
        }}
        className="space-y-6"
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
          <div className="space-y-4 p-4">
            <SelectField
              label={t("essentials.taxJurisdiction")}
              value={form.taxCountry}
              options={TAX_COUNTRIES}
              optionLabels={Object.fromEntries(
                TAX_COUNTRIES.map((country) => [
                  country,
                  <TaxCountryLabel
                    key={country}
                    country={country}
                    label={onboardingRegionLabel(country, t, i18n.language)}
                  />,
                ]),
              )}
              onChange={(value) => {
                if (value === form.taxCountry) return;
                update("taxCountry", value);
                const fiat = requiredFiatCurrency(value);
                if (fiat) {
                  // A country with its own currency starts on its defaults;
                  // both stay editable right below.
                  update("fiatCurrency", fiat as FiatCurrency);
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS[value]);
                } else if (
                  !gainsAlgorithmsFor(value).includes(form.gainsAlgorithm)
                ) {
                  // Only replace a method the new region cannot use.
                  update("gainsAlgorithm", GAINS_ALGORITHM_DEFAULTS[value]);
                }
              }}
            />

            <TaxCountryPreviewNotice country={form.taxCountry} />

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
                onChange={(value) => update("gainsAlgorithm", value)}
              />
            </div>
            {usesLongTermDays(form.taxCountry) && (
              <NumberField
                label={t("essentials.longTermDays")}
                name="taxLongTermDays"
                value={form.taxLongTermDays}
                placeholder={t("essentials.longTermDaysPlaceholder")}
                min={1}
                onChange={(value) => update("taxLongTermDays", value)}
                hint={taxLongTermDaysHint(form.taxLongTermDays)}
              />
            )}
          </div>
        </OnboardingSection>

        <OnboardingStepActions goBack={goBack} disabled={!canContinue} />
      </form>
    </OnboardingStepPage>
  );
};
