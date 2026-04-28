import type {
  AiProviderKind,
  AustrianGainsAlgorithm,
  BackendKind,
  BackendPreviewRow,
  FiatCurrency,
  GainsAlgorithm,
  GenericGainsAlgorithm,
  OnboardingForm,
  TaxCountry,
} from "./types";

export const DEFAULT_BACKEND_NAME = "mempool";
export const DEFAULT_BACKEND_URL = "https://mempool.bitcoin-austria.at/api";
export const DEFAULT_AI_PROVIDER_NAME = "ollama";
export const DEFAULT_AI_BASE_URL = "http://localhost:11434/v1";

export const DEFAULT_FORM: OnboardingForm = {
  name: "",
  workspace: "Personal",
  profile: "main",
  taxCountry: "at",
  fiatCurrency: "EUR",
  taxLongTermDays: "365",
  gainsAlgorithm: "MOVING_AVERAGE_AT",
  databaseMode: "sqlcipher",
  recoveryAcknowledged: false,
  plaintextAcknowledged: false,
  migrateCredentials: true,
  backendSetupMode: "default",
  backendKind: "esplora",
  backendName: DEFAULT_BACKEND_NAME,
  backendUrl: DEFAULT_BACKEND_URL,
  skipBackendsAcknowledged: false,
  aiSetupMode: "local",
  aiProviderKind: "local",
  aiProviderName: DEFAULT_AI_PROVIDER_NAME,
  aiBaseUrl: DEFAULT_AI_BASE_URL,
  aiRemoteAcknowledged: false,
};

export const FIAT_CURRENCIES: FiatCurrency[] = ["EUR", "USD", "CHF", "GBP"];

export const GENERIC_GAINS_ALGORITHMS: GenericGainsAlgorithm[] = [
  "FIFO",
  "LIFO",
  "HIFO",
  "LOFO",
];

export const AUSTRIAN_GAINS_ALGORITHMS: AustrianGainsAlgorithm[] = [
  "MOVING_AVERAGE_AT",
  "MOVING_AVERAGE",
  "FIFO",
];

export const GAINS_ALGORITHM_DEFAULTS: Record<TaxCountry, GainsAlgorithm> = {
  at: "MOVING_AVERAGE_AT",
  generic: "FIFO",
};

export const gainsAlgorithmsFor = (country: TaxCountry): GainsAlgorithm[] =>
  country === "at" ? AUSTRIAN_GAINS_ALGORITHMS : GENERIC_GAINS_ALGORITHMS;

export const parseTaxLongTermDays = (raw: string): number | null => {
  const trimmed = raw.trim();
  if (!/^[1-9]\d*$/.test(trimmed)) return null;
  const days = Number(trimmed);
  return Number.isSafeInteger(days) ? days : null;
};

export const BACKEND_KINDS: BackendKind[] = [
  "esplora",
  "electrum",
  "bitcoinrpc",
  "btcpay",
  "liquid-esplora",
  "custom",
];

export const BACKEND_KIND_LABELS: Record<BackendKind, string> = {
  esplora: "Esplora",
  electrum: "Electrum",
  bitcoinrpc: "Bitcoin Core RPC",
  btcpay: "BTCPay",
  "liquid-esplora": "Liquid Esplora",
  custom: "Custom",
};

export const PUBLIC_BACKEND_DEFAULTS: readonly BackendPreviewRow[] = [
  { name: DEFAULT_BACKEND_NAME, kind: "Esplora", url: DEFAULT_BACKEND_URL },
  {
    name: "fulcrum",
    kind: "Electrum",
    url: "ssl://index.bitcoin-austria.at:50002",
  },
  { name: "liquid", kind: "Electrum", url: "ssl://les.bullbitcoin.com:995" },
];

export const AI_PROVIDER_KIND_LABELS: Record<AiProviderKind, string> = {
  local: "Local",
  remote: "Remote",
  tee: "TEE",
};

/**
 * Returns a user-facing validation hint for the long-term-days input,
 * or `null` when the value parses to a positive integer. Step gates use
 * `null` here as the green-light condition.
 */
export const taxLongTermDaysHint = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!trimmed) return "Required.";
  const days = parseTaxLongTermDays(trimmed);
  if (days !== null) return null;
  const integerDays = Number(trimmed);
  if (
    /^-?\d+$/.test(trimmed) &&
    Number.isSafeInteger(integerDays) &&
    integerDays < 1
  ) {
    return "Must be at least 1 day.";
  }
  return "Use a whole number of days.";
};
