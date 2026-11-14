/**
 * Confirmed trust for a saved backend's TLS certificate
 * (docs/reference/backends.md#self-signed-certificates). The daemon fetches
 * and pins; this module only decides where the flow is offered and maps the
 * daemon's codes and fixed texts to the two problems the UI explains.
 */

/** Backend kinds whose `certificate` setting pins the server certificate. */
export const CERTIFICATE_TRUST_KINDS: readonly string[] = [
  "bitcoinrpc",
  "electrum",
  "esplora",
  "liquid-esplora",
  "mempool",
];

/** Whether a saved backend can use the trust flow: a supported kind over TLS. */
export function canTrustCertificate(
  kind: string | undefined,
  url: string | undefined,
): boolean {
  if (!kind || !CERTIFICATE_TRUST_KINDS.includes(kind)) return false;
  const scheme = (url ?? "").trim().toLowerCase();
  return scheme.startsWith("ssl://") || scheme.startsWith("https://");
}

export type CertificateProblem = "untrusted" | "changed";

export function certificateProblemFromCode(
  code: string | undefined | null,
): CertificateProblem | null {
  if (code === "tls_certificate_untrusted") return "untrusted";
  if (code === "tls_certificate_changed") return "changed";
  return null;
}

/**
 * Connection tests report failures as log lines carrying the daemon's fixed
 * English messages for these two errors (never server text), so matching
 * them is safe. Anything else is not a certificate-trust problem.
 */
export function certificateProblemFromText(
  text: string | undefined | null,
): CertificateProblem | null {
  if (!text) return null;
  if (/this server's certificate changed/i.test(text)) return "changed";
  if (/tls certificate is not signed by an authority kassiber trusts/i.test(text)) {
    return "untrusted";
  }
  return null;
}

/** `ui.backends.certificate.fetch` data. */
export interface PresentedCertificate {
  backend: string;
  kind: string;
  host: string;
  port: number;
  fingerprint_sha256: string;
  subject: string | null;
  issuer: string | null;
  not_before: string | null;
  not_after: string | null;
  self_signed: boolean;
  names: string[];
  replaces_fingerprint_sha256: string | null;
  clears_insecure: boolean;
}

/** `ui.backends.certificate.trust` data. */
export interface TrustedCertificate extends PresentedCertificate {
  trusted: true;
  /** The pin file the backend's `certificate` setting now names. */
  certificate: string;
}

/** Refusals and failures the dialog explains in its own words. */
export const CERTIFICATE_ERROR_KEYS = [
  "alreadyTrusted",
  "unpinnable",
  "plaintext",
  "unsupported",
  "mismatch",
  "offline",
  "connection",
  "writeFailed",
  "overridden",
  "unknown",
] as const;
export type CertificateErrorKey = (typeof CERTIFICATE_ERROR_KEYS)[number];

export function certificateErrorKey(code: string | undefined | null): CertificateErrorKey {
  switch (code) {
    case "backend_certificate_already_trusted":
      return "alreadyTrusted";
    case "backend_certificate_unpinnable":
      return "unpinnable";
    case "backend_certificate_plaintext":
      return "plaintext";
    case "backend_certificate_unsupported":
      return "unsupported";
    case "backend_certificate_mismatch":
      return "mismatch";
    case "network_egress_disabled":
      return "offline";
    case "network_error":
    case "backend_timeout":
    case "network_proxy_required":
    case "network_proxy_unsupported":
    case "protocol_error":
      return "connection";
    case "backend_certificate_write_failed":
      return "writeFailed";
    case "backend_certificate_overridden":
      return "overridden";
    default:
      return "unknown";
  }
}
