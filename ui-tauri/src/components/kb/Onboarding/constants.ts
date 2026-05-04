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
export const MIN_DATABASE_PASSPHRASE_CHARS = 12;

export const DEFAULT_FORM: OnboardingForm = {
  workspace: "My Books",
  profile: "Private",
  taxCountry: "at",
  fiatCurrency: "EUR",
  taxLongTermDays: "365",
  gainsAlgorithm: "MOVING_AVERAGE_AT",
  databaseMode: "sqlcipher",
  databasePassphrase: "",
  databasePassphraseConfirm: "",
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

const hasHttpUrl = (raw: string): boolean => {
  try {
    const parsed = new URL(raw);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
};

const hasInlineCredential = (raw: string): boolean => {
  const candidate = raw.includes("://") ? raw : `ssl://${raw}`;
  try {
    const parsed = new URL(candidate);
    return parsed.username.length > 0 || parsed.password.length > 0;
  } catch {
    return false;
  }
};

const hasSocketEndpoint = (raw: string): boolean => {
  const candidate = raw.includes("://") ? raw : `ssl://${raw}`;
  try {
    const parsed = new URL(candidate);
    return (
      (parsed.protocol === "ssl:" || parsed.protocol === "tcp:") &&
      parsed.hostname.length > 0 &&
      parsed.port.length > 0
    );
  } catch {
    return false;
  }
};

export const backendEndpointDescription = (kind: BackendKind): string => {
  if (kind === "electrum") {
    return "Use ssl://host:50002, tcp://host:50001, or host:port.";
  }
  if (kind === "bitcoinrpc") {
    return "Use the RPC URL only; add cookies or passwords after onboarding.";
  }
  if (kind === "custom") {
    return "Use the endpoint format expected by your custom adapter.";
  }
  return "Use an http(s) endpoint. Do not include API tokens or auth headers.";
};

export const backendEndpointHint = (
  kind: BackendKind,
  raw: string,
): string | null => {
  const trimmed = raw.trim();
  if (!trimmed) return "Endpoint is required.";
  if (hasInlineCredential(trimmed)) {
    return "Do not include usernames or passwords in the endpoint.";
  }
  if (kind === "electrum") {
    return hasSocketEndpoint(trimmed)
      ? null
      : "Use ssl://host:50002, tcp://host:50001, or host:port.";
  }
  if (kind === "custom") return null;
  return hasHttpUrl(trimmed) ? null : "Use an http:// or https:// URL.";
};

export const aiBaseUrlHint = (raw: string): string | null => {
  const trimmed = raw.trim();
  if (!trimmed) return "Base URL is required.";
  if (hasInlineCredential(trimmed)) {
    return "Do not include usernames or passwords in the endpoint.";
  }
  return hasHttpUrl(trimmed) ? null : "Use an http:// or https:// URL.";
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
 * Returns a user-facing validation hint for the generic long-term-days input,
 * or `null` when the value parses to a positive integer.
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

export const databasePassphraseHint = (
  passphrase: string,
  confirmation: string,
): string | null => {
  if (!passphrase) return "Enter a database passphrase.";
  if (passphrase.length < MIN_DATABASE_PASSPHRASE_CHARS) {
    return `Use at least ${MIN_DATABASE_PASSPHRASE_CHARS} characters.`;
  }
  if (!confirmation) return "Confirm the database passphrase.";
  if (passphrase !== confirmation) return "Passphrases do not match.";
  return null;
};
