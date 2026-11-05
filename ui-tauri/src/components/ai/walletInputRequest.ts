import type { AiChatToolCall } from "@/daemon/stream";

/** Connection sources the assistant may only hand to the desktop setup dialog. */
export const WALLET_INPUT_SOURCES = [
  "descriptor", "address-list", "silent-payment", "liquid-descriptor",
  "samourai", "core-ln", "lnd", "btcpay", "kraken", "coinbase", "binance",
] as const;
export type WalletInputSource = (typeof WALLET_INPUT_SOURCES)[number];

export interface WalletInputRequest {
  request_id: string;
  workspace_id: string;
  profile_id: string;
  source: WalletInputSource;
  explanation: string | null;
}

const record = (value: unknown): Record<string, unknown> | null =>
  value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;

/** Only a completed, well-formed server packet becomes a setup button. */
export function walletInputRequest(tool: AiChatToolCall): WalletInputRequest | null {
  if (tool.name !== "ui.wallets.request_input" || tool.status !== "done") return null;
  const envelope = record(tool.result);
  const data = record(envelope?.data);
  if (
    envelope?.kind !== "ui.wallets.request_input" || !data || data.schema_version !== 1
    || data.action !== "connect_wallet" || data.desktop_only !== true
    || typeof data.request_id !== "string" || !/^[a-f0-9]{64}$/.test(data.request_id)
    || typeof data.workspace_id !== "string" || !data.workspace_id
    || typeof data.profile_id !== "string" || !data.profile_id
    || !WALLET_INPUT_SOURCES.includes(data.source as WalletInputSource)
    || !(data.explanation === null || (typeof data.explanation === "string" && data.explanation.length <= 500))
  ) return null;
  return {
    request_id: data.request_id,
    workspace_id: data.workspace_id,
    profile_id: data.profile_id,
    source: data.source as WalletInputSource,
    explanation: data.explanation as string | null,
  };
}
