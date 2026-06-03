import {
  aiBaseUrlHint,
  backendEndpointHint,
  databasePassphraseHint,
  electrumEndpointUrl,
  parseTaxLongTermDays,
} from "./constants";
import type { OnboardingForm } from "./types";

/** Visible books label and hidden default book label are present. */
export const identityComplete = (form: OnboardingForm): boolean =>
  Boolean(form.workspace.trim() && form.profile.trim());

/** Tax defaults are valid: Austria needs nothing extra; generic needs a day count. */
export const accountingComplete = (form: OnboardingForm): boolean =>
  form.taxCountry === "at" ||
  parseTaxLongTermDays(form.taxLongTermDays) !== null;

/** The (optional) sync-backend choice is internally consistent. */
export const connectionsComplete = (form: OnboardingForm): boolean => {
  if (form.backendSetupMode === "skip") {
    return form.skipBackendsAcknowledged;
  }
  if (form.backendSetupMode === "custom") {
    const backendUrl =
      form.backendKind === "electrum"
        ? electrumEndpointUrl({
            host: form.backendHost,
            port: form.backendPort,
            useSsl: form.backendUseSsl,
          })
        : form.backendUrl;
    return Boolean(
      form.backendName.trim() &&
        backendEndpointHint(form.backendKind, backendUrl) === null,
    );
  }
  return true;
};

/** The (optional) AI choice is internally consistent. */
export const aiComplete = (form: OnboardingForm): boolean => {
  if (form.aiSetupMode !== "disabled") {
    if (aiBaseUrlHint(form.aiBaseUrl) !== null) return false;
  }
  if (form.aiSetupMode === "remote") {
    return Boolean(form.aiProviderName.trim() && form.aiRemoteAcknowledged);
  }
  return true;
};

/** Step "Your books" gate — books labels + valid tax defaults. */
export const essentialsStepComplete = (form: OnboardingForm): boolean =>
  identityComplete(form) && accountingComplete(form);

/** Step "Sync" gate — alias for the backend-consistency check. */
export const syncStepComplete = connectionsComplete;

/** Step "AI" gate — alias for the assistant-consistency check. */
export const aiStepComplete = aiComplete;

/** Step 2 ("Security") gate — encryption passphrase + required acknowledgements. */
export const securityStepComplete = (form: OnboardingForm): boolean =>
  form.databaseMode === "plaintext"
    ? form.plaintextAcknowledged
    : form.recoveryAcknowledged &&
      databasePassphraseHint(
        form.databasePassphrase,
        form.databasePassphraseConfirm,
      ) === null;

/** Final review is only actionable once all prior setup choices are valid. */
export const reviewStepComplete = (form: OnboardingForm): boolean =>
  essentialsStepComplete(form) &&
  syncStepComplete(form) &&
  aiStepComplete(form) &&
  securityStepComplete(form);
