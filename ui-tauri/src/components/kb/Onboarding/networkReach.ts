import { isLoopbackAiHttpUrl } from "@/lib/aiCapabilities";

/**
 * Where a choice can reach: out to the internet, a server on this device, or
 * nowhere. Setup shows this on every option and on Review, so the user can see
 * what Kassiber may contact before choosing it.
 */
export type NetworkReach = "internet" | "device" | "none";

export interface NetworkHost {
  host: string;
  /** What the server is, e.g. "Explorer API". */
  label?: string;
}

export interface NetworkSummary {
  reach: NetworkReach;
  /** Who and when, e.g. "Internet · when you sync". */
  label: string;
  /** The hosts it would contact, when known; one per line. */
  hosts?: NetworkHost[];
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

/**
 * A server on this machine stays on it; anything else goes out. Uses the
 * same loopback rules as the AI endpoint check (localhost, 127/8, ::1 and
 * IPv4-mapped 127/8), for any scheme a sync server can use.
 */
export function endpointReach(url: string): Exclude<NetworkReach, "none"> {
  const trimmed = url.trim();
  const asHttp = trimmed.includes("://")
    ? trimmed.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "http://")
    : `http://${trimmed}`;
  return isLoopbackAiHttpUrl(asHttp) ? "device" : "internet";
}
