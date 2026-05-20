import { describe, expect, it } from "vitest";

import {
  backendTrustFromEndpoint,
  endpointHost,
  inferredInfrastructureOwnership,
  isLocalOrPrivateHost,
} from "./backendTrust";

describe("endpointHost", () => {
  it("extracts the host across schemes", () => {
    expect(endpointHost("https://mempool.space/api")).toBe("mempool.space");
    expect(endpointHost("ssl://index.example.at:50002")).toBe(
      "index.example.at",
    );
    expect(endpointHost("http://127.0.0.1:3002")).toBe("127.0.0.1");
    expect(endpointHost("https://[::1]:8332")).toBe("::1");
    expect(endpointHost("")).toBe("");
  });
});

describe("isLocalOrPrivateHost", () => {
  it("treats loopback, mDNS, and RFC1918 ranges as local", () => {
    for (const host of [
      "localhost",
      "127.0.0.1",
      "node.local",
      "10.0.0.5",
      "192.168.1.20",
      "172.16.0.1",
      "172.31.255.254",
      "169.254.10.1",
      "fd00::1",
      "fe80::1",
    ]) {
      expect(isLocalOrPrivateHost(host)).toBe(true);
    }
  });

  it("treats public hosts and 172.32+ as non-local", () => {
    for (const host of [
      "mempool.space",
      "8.8.8.8",
      "172.32.0.1",
      "203.0.113.1",
      "",
    ]) {
      expect(isLocalOrPrivateHost(host)).toBe(false);
    }
  });
});

describe("inferredInfrastructureOwnership", () => {
  it("defaults local/LAN endpoints to self and public to third-party", () => {
    expect(inferredInfrastructureOwnership("http://192.168.1.5:3002")).toBe(
      "self",
    );
    expect(inferredInfrastructureOwnership("http://127.0.0.1:8332")).toBe(
      "self",
    );
    expect(inferredInfrastructureOwnership("https://mempool.space/api")).toBe(
      "third_party",
    );
  });
});

describe("backendTrustFromEndpoint", () => {
  it("classifies a local endpoint as on-device regardless of ownership", () => {
    expect(backendTrustFromEndpoint("http://127.0.0.1:3002").posture).toBe(
      "on-device",
    );
    expect(
      backendTrustFromEndpoint("http://192.168.1.5:50002", false, "third_party")
        .posture,
    ).toBe("on-device");
  });

  it("classifies a self-operated remote node as self-hosted, not on-device", () => {
    const trust = backendTrustFromEndpoint(
      "https://node.example.com:50002",
      false,
      "self",
    );
    expect(trust.posture).toBe("self-hosted");
    expect(trust.label).toBe("Your infrastructure");
  });

  it("classifies a public endpoint as a third-party server", () => {
    const trust = backendTrustFromEndpoint("https://mempool.space/api");
    expect(trust.posture).toBe("remote");
    expect(trust.label).toBe("Third-party server");
  });

  it("marks onion and proxied endpoints as shielded", () => {
    expect(backendTrustFromEndpoint("http://abc.onion").posture).toBe(
      "shielded",
    );
    expect(
      backendTrustFromEndpoint("https://mempool.space/api", true).posture,
    ).toBe("shielded");
  });

  it("does not downgrade a self-operated endpoint to third-party", () => {
    // Regression guard: a refactor must never silently turn first-party infra
    // into a third-party exposure in the privacy summary.
    expect(
      backendTrustFromEndpoint("https://my-node.example.com", false, "self")
        .posture,
    ).not.toBe("remote");
  });
});
