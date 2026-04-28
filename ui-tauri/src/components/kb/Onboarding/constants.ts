import type {
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

/**
 * Returns a user-facing validation hint for the long-term-days input,
 * or `null` when the value parses to a positive integer. Step gates use
 * `null` here as the green-light condition.
 */
export const taxLongTermDaysHint = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!trimmed) return "Required.";
  const days = Number.parseInt(trimmed, 10);
  if (!Number.isFinite(days) || String(days) !== trimmed) {
    return "Use a whole number of days.";
  }
  if (days < 1) return "Must be at least 1 day.";
  return null;
};
