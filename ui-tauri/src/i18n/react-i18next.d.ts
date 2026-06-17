/**
 * Type-safe translation keys. `CustomTypeOptions.resources` is wired to the
 * English bundles, so `t("settings:appearance.theme.title")` autocompletes and
 * an unknown or misspelled key is a compile error. English is the reference
 * because `i18n.test.ts` guarantees every other language matches its keys.
 */

import "react-i18next";

import type { AppResources } from "./resources";

declare module "react-i18next" {
  interface CustomTypeOptions {
    defaultNS: "common";
    resources: AppResources;
  }
}
