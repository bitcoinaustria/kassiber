import { DaemonRequestError } from "@/daemon/client";

/** Exchanges whose history Kassiber reads through a read-only API key. */
export const EXCHANGE_KINDS = ["kraken", "coinbase", "binance", "bitpanda"] as const;
export type ExchangeKind = (typeof EXCHANGE_KINDS)[number];

export function isExchangeKind(kind: string | null | undefined): kind is ExchangeKind {
  return (EXCHANGE_KINDS as readonly string[]).includes((kind ?? "").trim().toLowerCase());
}

/** Bitpanda authenticates with the key alone; the others sign with a secret. */
export function exchangeNeedsSecret(kind: ExchangeKind) {
  return kind !== "bitpanda";
}

export const EXCHANGE_PROVIDER_NAMES: Record<ExchangeKind, string> = {
  kraken: "Kraken",
  coinbase: "Coinbase",
  binance: "Binance",
  bitpanda: "Bitpanda",
};

export interface ExchangeKeyGuide {
  /** Where the user creates the key, in the exchange's own account pages. */
  keyPageUrl: string;
  /** The exchange's permission names to turn on, as its key page shows them. */
  grant: readonly string[];
  /** Permissions to leave off. */
  deny: readonly string[];
}

/**
 * What each sync reads decides the scopes: Kraken's TradesHistory and Ledgers,
 * Coinbase's accounts and transactions, Binance's fiat payments, deposits,
 * withdrawals and dividends, Bitpanda's operations, assets and currencies
 * (docs/reference/rust-exchange-sync.md, docs/reference/bitpanda.md).
 */
export const EXCHANGE_KEY_GUIDES: Record<ExchangeKind, ExchangeKeyGuide> = {
  kraken: {
    keyPageUrl: "https://pro.kraken.com/app/settings/api",
    grant: ["Query closed orders & trades", "Query ledger entries"],
    deny: [
      "Deposit funds",
      "Withdraw funds",
      "Create & modify orders",
      "Cancel & close orders",
    ],
  },
  coinbase: {
    keyPageUrl: "https://www.coinbase.com/settings/api",
    grant: ["wallet:accounts:read", "wallet:transactions:read"],
    deny: [
      "wallet:transactions:send",
      "wallet:buys:create",
      "wallet:sells:create",
      "wallet:withdrawals:create",
    ],
  },
  binance: {
    keyPageUrl: "https://www.binance.com/en/my/settings/api-management",
    grant: ["Enable Reading"],
    deny: [
      "Enable Spot & Margin Trading",
      "Enable Withdrawals",
      "Enable Futures",
      "Permits Universal Transfer",
    ],
  },
  bitpanda: {
    keyPageUrl: "https://app.bitpanda.com/my-account/apikey",
    grant: ["Transaction", "Trade (Read)"],
    deny: ["Trade (Write)", "Earn (Write)", "Balances"],
  },
};

/** The pasted credentials, exactly as a check or a save sends them. */
export interface ExchangeCredentials {
  exchange: ExchangeKind;
  apiKey: string;
  apiSecret: string;
}

export function credentialArgs(credentials: ExchangeCredentials) {
  const args: Record<string, string> = {
    exchange: credentials.exchange,
    api_key: credentials.apiKey.trim(),
  };
  if (exchangeNeedsSecret(credentials.exchange)) {
    args.api_secret = credentials.apiSecret.trim();
  }
  return args;
}

/** True when two drafts would send the same credentials. */
export function sameCredentials(a: ExchangeCredentials, b: ExchangeCredentials | null) {
  if (!b) return false;
  const left = credentialArgs(a);
  const right = credentialArgs(b);
  return (
    left.exchange === right.exchange &&
    left.api_key === right.api_key &&
    left.api_secret === right.api_secret
  );
}

export type ExchangeCredentialIssue = "api_key" | "api_secret";

export function credentialIssue(
  credentials: ExchangeCredentials,
): ExchangeCredentialIssue | null {
  if (!credentials.apiKey.trim()) return "api_key";
  if (exchangeNeedsSecret(credentials.exchange) && !credentials.apiSecret.trim()) {
    return "api_secret";
  }
  return null;
}

/** What the setup panel hands the dialog. */
export interface ExchangeSetupDraft {
  credentials: ExchangeCredentials;
  /** The current credentials passed Test connection. */
  tested: boolean;
  issue: ExchangeCredentialIssue | "test_first" | null;
}

/** The daemon's answer to Test connection. */
export interface ExchangeTestResult {
  exchange: ExchangeKind;
  ok: boolean;
  /** True when the exchange confirmed the key cannot trade or withdraw;
   *  null when the exchange does not say (Kraken, Coinbase). */
  read_only: boolean | null;
}

export interface ExchangeCreateResult {
  exchange: ExchangeKind;
  wallet: { id: string; label: string; kind: ExchangeKind };
  backend: { name: string };
}

export interface ExchangeSyncResult {
  imported?: number;
  updated?: number;
  fetched?: number;
  skipped?: Record<string, number>;
  wallet?: string;
}

export interface ExchangeErrorInfo {
  code?: string;
  message?: string;
  hint?: string | null;
  retryable?: boolean;
  details?: Record<string, unknown> | null;
}

export interface ExchangeLastSync {
  status: "ok" | "error";
  at: string;
  imported?: number;
  updated?: number;
  fetched?: number;
  skipped?: Record<string, number>;
  error?: ExchangeErrorInfo | null;
}

export interface ExchangeConnectionStatus {
  wallet_id: string;
  wallet: string;
  exchange: ExchangeKind;
  provider: string;
  backend: string | null;
  backend_configured: boolean;
  has_api_key: boolean;
  has_api_secret: boolean | null;
  last_sync: ExchangeLastSync | null;
}

/** Error explanations by daemon error code; each is a key under
 *  `exchange.error` in the connections bundle. */
export type ExchangeErrorKind =
  | "auth"
  | "rateLimited"
  | "offline"
  | "consent"
  | "protocol"
  | "refused"
  | "config"
  | "timeout"
  | "network"
  | "http"
  | "conflict"
  | "needsRust"
  | "unknown";

const CODE_KINDS: Record<string, ExchangeErrorKind> = {
  auth_error: "auth",
  rate_limited: "rateLimited",
  network_egress_disabled: "offline",
  interaction_required: "consent",
  protocol_error: "protocol",
  validation: "refused",
  config_error: "config",
  backend_timeout: "timeout",
  network_error: "network",
  network_proxy_required: "network",
  network_proxy_unsupported: "network",
  app_error: "http",
  conflict: "conflict",
  // A daemon without these kinds: an older build.
  daemon_unavailable: "needsRust",
  unsupported_kind: "needsRust",
  kind_not_allowed: "needsRust",
  not_implemented: "needsRust",
};

export function exchangeErrorKind(code: string | undefined): ExchangeErrorKind {
  return (code && CODE_KINDS[code]) || "unknown";
}

/** The error a daemon request failed with, as the envelope carries it. */
export function exchangeErrorInfo(error: unknown): ExchangeErrorInfo {
  if (error instanceof DaemonRequestError) {
    const envelopeError = error.envelope.error;
    return {
      code: envelopeError?.code,
      message: envelopeError?.message ?? error.message,
      hint: envelopeError?.hint ?? null,
      retryable: envelopeError?.retryable,
      details:
        envelopeError?.details && typeof envelopeError.details === "object"
          ? (envelopeError.details as Record<string, unknown>)
          : null,
    };
  }
  return { message: error instanceof Error ? error.message : String(error) };
}

/** Permissions a refused Binance key still holds, when the daemon named them. */
export function refusedPermissions(info: ExchangeErrorInfo): string[] {
  const permissions = info.details?.permissions;
  return Array.isArray(permissions)
    ? permissions.filter((item): item is string => typeof item === "string")
    : [];
}
