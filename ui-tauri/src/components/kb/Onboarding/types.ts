import type { ComponentType } from "react";

import type { GainsAlgorithm, TaxCountry } from "@/lib/taxCountries";

export type { GainsAlgorithm, TaxCountry };
export type FiatCurrency = "EUR" | "USD" | "CHF" | "GBP";

export type DatabaseMode = "sqlcipher" | "plaintext";
export type BackendSetupMode = "default" | "custom" | "skip";
export type AiSetupMode = "local" | "remote" | "disabled";
export type AiProviderKind = "local" | "remote" | "tee";
export type BackendKind =
  | "esplora"
  | "electrum"
  | "bitcoinrpc"
  | "liquid-esplora";

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
  enableTouchId: boolean;
  recoveryAcknowledged: boolean;
  plaintextAcknowledged: boolean;
  migrateCredentials: boolean;
  updateChecksEnabled: boolean;
  /**
   * Unset until the user picks: setup does not choose a sync server for
   * them (privacy by default). The express path sets it explicitly.
   */
  backendSetupMode: BackendSetupMode | null;
  /** Built-in backends are Bitcoin mainchain; this adds the Liquid presets. */
  includeLiquidBackends: boolean;
  backendKind: BackendKind;
  backendName: string;
  backendUrl: string;
  backendHost: string;
  backendPort: string;
  backendUseSsl: boolean;
  backendTrustSsl: boolean;
  backendCertificate: string;
  backendUseProxy: boolean;
  backendProxyHost: string;
  backendProxyPort: string;
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
  onJump?: (index: number) => void;
  canContinue?: boolean;
  submitting?: boolean;
  currentStep: number;
  totalSteps: number;
  backendPreviewRows?: BackendPreviewRow[];
}

export interface OnboardingStep {
  component: ComponentType<StepComponentProps>;
  isComplete: (form: OnboardingForm) => boolean;
  /** Short label shown in the progress stepper (e.g. "Books", "Database"). */
  label?: string;
}

export interface BackendPreviewRow {
  name: string;
  kind: string;
  url: string;
  /** "bitcoin" or "liquid". */
  chain: string;
  /**
   * A Liquid preset setup adds only when asked. False for an
   * environment-defined backend, which stays active either way.
   */
  liquidOptIn: boolean;
}
