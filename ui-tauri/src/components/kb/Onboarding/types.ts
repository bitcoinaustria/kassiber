import type { ComponentType } from "react";

export type TaxCountry = "at" | "generic";
export type FiatCurrency = "EUR" | "USD" | "CHF" | "GBP";

// Algorithm tokens match `kassiber.tax_policy` / rp2 plugins.
// Generic country exposes FIFO/LIFO/HIFO/LOFO. New Austrian wallets expose
// only the current-rule moving-average default; Altbestand handling is
// wallet-specific rather than a profile-level holding-period setting.
export type GenericGainsAlgorithm = "FIFO" | "LIFO" | "HIFO" | "LOFO";
export type AustrianGainsAlgorithm = "MOVING_AVERAGE_AT";
export type GainsAlgorithm = GenericGainsAlgorithm | AustrianGainsAlgorithm;

export type DatabaseMode = "sqlcipher" | "plaintext";
export type BackendSetupMode = "default" | "custom" | "skip";
export type AiSetupMode = "local" | "remote" | "disabled";
export type AiProviderKind = "local" | "remote" | "tee";
export type BackendKind =
  | "esplora"
  | "electrum"
  | "bitcoinrpc"
  | "btcpay"
  | "liquid-esplora"
  | "custom";

export interface OnboardingForm {
  workspace: string;
  profile: string;
  taxCountry: TaxCountry;
  fiatCurrency: FiatCurrency;
  taxLongTermDays: string;
  gainsAlgorithm: GainsAlgorithm;
  databaseMode: DatabaseMode;
  databasePassphrase: string;
  databasePassphraseConfirm: string;
  recoveryAcknowledged: boolean;
  plaintextAcknowledged: boolean;
  migrateCredentials: boolean;
  backendSetupMode: BackendSetupMode;
  backendKind: BackendKind;
  backendName: string;
  backendUrl: string;
  skipBackendsAcknowledged: boolean;
  aiSetupMode: AiSetupMode;
  aiProviderKind: AiProviderKind;
  aiProviderName: string;
  aiBaseUrl: string;
  aiRemoteAcknowledged: boolean;
}

export interface StepComponentProps {
  form: OnboardingForm;
  update: <K extends keyof OnboardingForm>(
    key: K,
    value: OnboardingForm[K],
  ) => void;
  onSubmit: () => void;
  goBack?: () => void;
  currentStep: number;
  totalSteps: number;
}

export interface OnboardingStep {
  component: ComponentType<StepComponentProps>;
  isComplete: (form: OnboardingForm) => boolean;
}

export interface BackendPreviewRow {
  name: string;
  kind: string;
  url: string;
}
