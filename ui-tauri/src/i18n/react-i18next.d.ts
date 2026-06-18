/**
 * Type-safe translation keys. i18next reads `CustomTypeOptions` from its own
 * module to build `TypeOptions` → `TFunction`, so the augmentation must target
 * `i18next` (not `react-i18next`) for the English bundle to actually drive key
 * checking. With this in place `t("settings:appearance.theme.title")`
 * autocompletes and an unknown or misspelled literal key is a compile error.
 * English is the reference because `i18n.test.ts` guarantees every other
 * language matches its keys.
 */

import "i18next";

import type { AppResources } from "./resources";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "common";
    resources: AppResources;
  }
}
