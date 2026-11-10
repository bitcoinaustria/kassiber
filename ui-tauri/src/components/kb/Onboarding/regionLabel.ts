import i18next from "i18next";

import { taxCountryName } from "@/lib/taxCountries";

type RegionKey = "essentials.jurisdictionAustria" | "essentials.jurisdictionGeneric";

// The onboarding name of a tax country: its own wording for Austria and the
// generic country, the registry's name in the UI language for the others.
export const onboardingRegionLabel = (
  country: string,
  t: (key: RegionKey) => string,
  language: string = i18next.language,
): string =>
  country === "at"
    ? t("essentials.jurisdictionAustria")
    : country === "generic"
      ? t("essentials.jurisdictionGeneric")
      : taxCountryName(country, language);
