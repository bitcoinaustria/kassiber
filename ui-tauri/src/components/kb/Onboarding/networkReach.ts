/**
 * Where a choice can reach: out to the internet, a server on this device, or
 * nowhere. Setup shows this on every option and on Review, so the user can see
 * what Kassiber may contact before choosing it.
 */
export type NetworkReach = "internet" | "device" | "none";

export interface NetworkSummary {
  reach: NetworkReach;
  /** Who and when, e.g. "Internet · when you sync". */
  label: string;
  /** The hosts it would contact, when known. */
  hosts?: string[];
  /** Said instead of hosts until they are known ("the server you enter"). */
  pending?: string;
}

/**
 * The host (and non-default port) of an endpoint URL of any scheme, or the
 * input when it doesn't parse as one.
 */
export function endpointHost(url: string): string {
  const trimmed = url.trim();
  try {
    return new URL(trimmed).host || trimmed;
  } catch {
    return trimmed;
  }
}

function endpointHostname(url: string): string {
  try {
    return new URL(url.trim()).hostname;
  } catch {
    return url.trim();
  }
}

/** Distinct hosts of several endpoints. */
export function endpointHosts(urls: string[]): string[] {
  return [...new Set(urls.map(endpointHost).filter(Boolean))];
}

const LOOPBACK = /^(localhost|127(?:\.\d{1,3}){3}|\[?::1\]?)$/i;

/** A server on this machine stays on it; anything else goes out. */
export function endpointReach(url: string): Exclude<NetworkReach, "none"> {
  return LOOPBACK.test(endpointHostname(url)) ? "device" : "internet";
}
