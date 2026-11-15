import { describe, expect, it } from "vitest";

import {
  CERTIFICATE_ERROR_KEYS,
  canTrustCertificate,
  certificateErrorKey,
  certificateProblemFromCode,
  certificateProblemFromText,
} from "./certificateTrust";

describe("certificate trust availability", () => {
  it.each([
    ["electrum", "ssl://node.local:50002", true],
    ["electrum", "tcp://node.local:50001", false],
    ["esplora", "https://node.local/api", true],
    ["esplora", "http://node.local/api", false],
    ["bitcoinrpc", "HTTPS://node.local:8332", true],
    ["liquid-esplora", "https://node.local/api", true],
    ["mempool", "https://node.local/api", true],
    ["btcpay", "https://pay.example", false],
    ["lnd", "https://127.0.0.1:8080", false],
    [undefined, "https://node.local", false],
  ] as const)("%s %s → %s", (kind, url, expected) => {
    expect(canTrustCertificate(kind, url)).toBe(expected);
  });
});

describe("certificate problems", () => {
  it("reads the daemon's codes", () => {
    expect(certificateProblemFromCode("tls_certificate_untrusted")).toBe("untrusted");
    expect(certificateProblemFromCode("tls_certificate_changed")).toBe("changed");
    expect(certificateProblemFromCode("network_error")).toBeNull();
    expect(certificateProblemFromCode(undefined)).toBeNull();
  });

  it("reads the daemon's fixed texts in connection-test logs", () => {
    expect(
      certificateProblemFromText(
        "Opening Electrum connection to ssl://n:50002\nConnection failed: The server's TLS certificate is not signed by an authority Kassiber trusts",
      ),
    ).toBe("untrusted");
    expect(
      certificateProblemFromText(
        "< connection failed: This server's certificate changed: it no longer matches the certificate trusted for this backend",
      ),
    ).toBe("changed");
    expect(certificateProblemFromText("< connection failed: network I/O failed")).toBeNull();
    expect(certificateProblemFromText("")).toBeNull();
  });

  it("maps every refusal to an explained message", () => {
    expect(certificateErrorKey("backend_certificate_already_trusted")).toBe("alreadyTrusted");
    expect(certificateErrorKey("backend_certificate_mismatch")).toBe("mismatch");
    expect(certificateErrorKey("backend_certificate_config_changed")).toBe("changedSinceReview");
    expect(certificateErrorKey("backend_certificate_unpinnable")).toBe("unpinnable");
    expect(certificateErrorKey("network_egress_disabled")).toBe("offline");
    expect(certificateErrorKey("network_error")).toBe("connection");
    expect(certificateErrorKey("something_new")).toBe("unknown");
    expect(new Set(CERTIFICATE_ERROR_KEYS).size).toBe(CERTIFICATE_ERROR_KEYS.length);
  });
});
