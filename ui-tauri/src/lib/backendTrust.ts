// Privacy-posture read for a configured backend endpoint.
//
// Two axes matter for Bitcoin infra: *who operates the server* (first- vs
// third-party) and *whether your IP is exposed* (Tor/proxy). This module is
// intentionally conservative — anything that is not clearly on your own
// machine/LAN, marked as your own infrastructure, or Tor/proxy-shielded is
// treated as a third party that can observe your queries. No hardcoded service
// allowlists: the URL shape and the explicit ownership flag tell the story.
//
// Kept as a standalone, side-effect-free module so the posture logic can be
// unit-tested without rendering the (large) SettingsScreen component.

import { Network, ShieldCheck, ShieldOff, type LucideIcon } from "lucide-react";

export type InfrastructureOwnership = "self" | "third_party";

export type TrustPosture = "on-device" | "self-hosted" | "shielded" | "remote";

export interface TrustInfo {
  posture: TrustPosture;
  label: string;
  note: string;
  icon: LucideIcon;
  className: string;
}

const PROXY_AWARE_TRANSPORTS = new Set([
  "bitcoinrpc",
  "btcpay",
  "electrum",
  "esplora",
  "liquid-esplora",
  "mempool",
]);

const FIRST_PARTY_CLASS =
  "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
const SHIELDED_CLASS =
  "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300";
const THIRD_PARTY_CLASS =
  "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300";

/** Best-effort host extraction that tolerates non-HTTP schemes (ssl://, cln://). */
export function endpointHost(url: string): string {
  const raw = (url ?? "").trim();
  if (!raw) return "";
  try {
    const parsed = new URL(raw.includes("://") ? raw : `https://${raw}`);
    return parsed.hostname.toLowerCase().replace(/^\[|\]$/g, "");
  } catch {
    return raw.toLowerCase();
  }
}

export function isOnionHost(host: string): boolean {
  return host.toLowerCase().endsWith(".onion");
}

export function isOnionEndpoint(url: string): boolean {
  return isOnionHost(endpointHost(url));
}

/**
 * Loopback, RFC1918 private ranges, link-local, mDNS `.local`, and IPv6
 * ULA/link-local all indicate infrastructure on your own machine or LAN.
 */
export function isLocalOrPrivateHost(host: string): boolean {
  if (!host) return false;
  if (host === "localhost" || host === "0.0.0.0" || host === "::1") return true;
  if (host.endsWith(".local") || host.endsWith(".localhost")) return true;
  if (/^127\./.test(host)) return true; // loopback
  if (/^10\./.test(host)) return true; // RFC1918
  if (/^192\.168\./.test(host)) return true; // RFC1918
  if (/^172\.(1[6-9]|2\d|3[01])\./.test(host)) return true; // RFC1918 172.16-31
  if (/^169\.254\./.test(host)) return true; // IPv4 link-local
  if (/^f[cd][0-9a-f]*:/.test(host)) return true; // IPv6 ULA fc00::/7
  if (/^fe[89ab][0-9a-f]*:/.test(host)) return true; // IPv6 link-local fe80::/10
  return false;
}

/** Default ownership when the user has not set it explicitly. */
export function inferredInfrastructureOwnership(
  url: string,
): InfrastructureOwnership {
  return isLocalOrPrivateHost(endpointHost(url)) ? "self" : "third_party";
}

export function backendTrustFromEndpoint(
  url: string,
  hasProxy = false,
  transportKindOrOwnership?: string | InfrastructureOwnership,
  ownershipMaybe?: InfrastructureOwnership,
): TrustInfo {
  const ownership =
    ownershipMaybe ??
    (transportKindOrOwnership === "self" ||
    transportKindOrOwnership === "third_party"
      ? transportKindOrOwnership
      : undefined);
  const transportKind =
    ownershipMaybe === undefined &&
    (transportKindOrOwnership === "self" ||
      transportKindOrOwnership === "third_party")
      ? undefined
      : transportKindOrOwnership?.toLowerCase();
  const host = endpointHost(url);
  const isLocal = isLocalOrPrivateHost(host);
  const isOnion = isOnionHost(host);
  const proxyHonored =
    hasProxy &&
    (!transportKind || PROXY_AWARE_TRANSPORTS.has(transportKind));

  // Truly local wins regardless of the ownership flag — the queries never
  // leave your machine/LAN.
  if (isLocal) {
    return {
      posture: "on-device",
      label: "On device",
      note: "Runs on your machine or local network — address queries stay inside your own network.",
      icon: ShieldCheck,
      className: FIRST_PARTY_CLASS,
    };
  }

  // Remote, but you operate it: queries leave the device, yet only reach a
  // server inside your own trust boundary.
  if (ownership === "self") {
    return {
      posture: "self-hosted",
      label: "Your infrastructure",
      note: "Infrastructure you operate. Queries leave this device but only reach a server inside your trust boundary.",
      icon: ShieldCheck,
      className: FIRST_PARTY_CLASS,
    };
  }

  if (isOnion || proxyHonored) {
    return {
      posture: "shielded",
      label:
        ownership === "third_party"
          ? isOnion
            ? "Third-party via Tor"
            : "Third-party via proxy"
          : isOnion
            ? "Tor"
            : "Via proxy",
      note: isOnion
        ? "Reached over Tor — this server cannot tie your queries to your IP address, but still sees the queries themselves."
        : "Only this configured backend is routed through its proxy — your IP address stays hidden from this server, which still sees the queries themselves. Other backends keep their own routing.",
      icon: Network,
      className: SHIELDED_CLASS,
    };
  }

  return {
    posture: "remote",
    label: "Third-party server",
    note: "A third party operates this endpoint and can observe your queries. Use your own infrastructure or a proxy if that is not acceptable.",
    icon: ShieldOff,
    className: THIRD_PARTY_CLASS,
  };
}
