export interface ExplorerSettings {
  bitcoinBaseUrl: string;
  liquidBaseUrl: string;
}

export const DEFAULT_EXPLORER_SETTINGS: ExplorerSettings = {
  bitcoinBaseUrl: "",
  liquidBaseUrl: "",
};

export type ExplorerNetwork = "bitcoin" | "liquid";

export interface ExplorerTarget {
  label: string;
  url: string;
  configured: boolean;
}

const PUBLIC_EXPLORERS: Record<ExplorerNetwork, { label: string; baseUrl: string }> = {
  bitcoin: { label: "mempool.space", baseUrl: "https://mempool.space" },
  liquid: { label: "Liquid Network", baseUrl: "https://liquid.network" },
};

function normalizeExplorerBaseUrl(baseUrl: string) {
  return baseUrl.trim().replace(/\/+$/, "").replace(/\/api$/i, "");
}

function labelForExplorerUrl(url: string, fallback: string) {
  try {
    return new URL(url).host || fallback;
  } catch {
    return fallback;
  }
}

function transactionUrl(baseUrl: string, txid: string) {
  const encoded = encodeURIComponent(txid);
  const url = baseUrl.includes("{txid}")
    ? baseUrl.replaceAll("{txid}", encoded)
    : `${normalizeExplorerBaseUrl(baseUrl)}/tx/${encoded}`;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return null;
    }
    return parsed.toString();
  } catch {
    return null;
  }
}

export function explorerTargetForTransaction({
  txid,
  network,
  settings,
}: {
  txid: string | undefined;
  network: ExplorerNetwork;
  settings?: ExplorerSettings;
}): ExplorerTarget | null {
  const id = txid?.trim();
  if (!id) return null;

  const configuredBase =
    network === "liquid" ? settings?.liquidBaseUrl : settings?.bitcoinBaseUrl;
  const configured = Boolean(configuredBase?.trim());
  const fallback = PUBLIC_EXPLORERS[network];
  const baseUrl = configured ? configuredBase?.trim() ?? "" : fallback.baseUrl;
  const url = transactionUrl(baseUrl, id);
  if (!url) return null;

  return {
    label: configured ? labelForExplorerUrl(url, "Configured explorer") : fallback.label,
    url,
    configured,
  };
}
