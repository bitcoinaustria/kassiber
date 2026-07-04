export interface ExplorerSettings {
  bitcoinBaseUrl: string;
  liquidBaseUrl: string;
  publicFallbacks: boolean;
}

export const DEFAULT_EXPLORER_SETTINGS: ExplorerSettings = {
  bitcoinBaseUrl: "",
  liquidBaseUrl: "",
  publicFallbacks: true,
};

export type ExplorerNetwork = "bitcoin" | "liquid";

export interface ExplorerTarget {
  label: string;
  url: string;
  configured: boolean;
}

const PUBLIC_EXPLORERS: Record<ExplorerNetwork, { label: string; baseUrl: string }> = {
  bitcoin: {
    label: "mempool.bitcoin-austria.at",
    baseUrl: "https://mempool.bitcoin-austria.at",
  },
  liquid: { label: "Liquid Network", baseUrl: "https://liquid.network" },
};

export function normalizeExplorerBaseUrl(baseUrl: string) {
  return baseUrl.trim().replace(/\/+$/, "").replace(/\/api$/i, "");
}

function labelForExplorerUrl(url: string, fallback: string) {
  try {
    return new URL(url).host || fallback;
  } catch {
    return fallback;
  }
}

function explorerUrl(baseUrl: string, segment: "address" | "tx", value: string) {
  const encoded = encodeURIComponent(value);
  const placeholder = segment === "tx" ? "{txid}" : "{address}";
  const url = baseUrl.includes(placeholder)
    ? baseUrl.replaceAll(placeholder, encoded)
    : `${normalizeExplorerBaseUrl(baseUrl)}/${segment}/${encoded}`;
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

function targetForExplorerValue({
  value,
  network,
  settings,
  segment,
}: {
  value: string | undefined;
  network: ExplorerNetwork;
  settings?: ExplorerSettings;
  segment: "address" | "tx";
}): ExplorerTarget | null {
  const id = value?.trim();
  if (!id) return null;

  const configuredBase =
    network === "liquid" ? settings?.liquidBaseUrl : settings?.bitcoinBaseUrl;
  const configured = Boolean(configuredBase?.trim());
  const fallback = PUBLIC_EXPLORERS[network];
  if (!configured && settings?.publicFallbacks === false) return null;
  const baseUrl = configured ? configuredBase?.trim() ?? "" : fallback.baseUrl;
  const url = explorerUrl(baseUrl, segment, id);
  if (!url) return null;

  return {
    label: configured ? labelForExplorerUrl(url, "Configured explorer") : fallback.label,
    url,
    configured,
  };
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
  return targetForExplorerValue({
    value: txid,
    network,
    settings,
    segment: "tx",
  });
}

export function explorerTargetForAddress({
  address,
  network,
  settings,
}: {
  address: string | undefined;
  network: ExplorerNetwork;
  settings?: ExplorerSettings;
}): ExplorerTarget | null {
  return targetForExplorerValue({
    value: address,
    network,
    settings,
    segment: "address",
  });
}
