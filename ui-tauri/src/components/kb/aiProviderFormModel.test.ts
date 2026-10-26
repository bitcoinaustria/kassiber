import { describe, expect, it } from "vitest";

import { autoRemoteReason, httpOrigin } from "./aiProviderFormModel";

describe("provider form posture from the URL", () => {
  it("moves only a local provider, and only for an address off this device", () => {
    expect(autoRemoteReason("local", "https://openrouter.ai/api/v1")).toBe("url");
    expect(autoRemoteReason("local", "http://192.168.1.5:8000/v1")).toBe("url");
    expect(autoRemoteReason("local", "http://localhost:11434/v1")).toBeNull();
    expect(autoRemoteReason("local", "http://[::1]:8000/v1")).toBeNull();
    expect(autoRemoteReason("local", "codex-cli://default")).toBe("cli");
  });

  it("never moves remote or TEE, and never to local", () => {
    for (const kind of ["remote", "tee"] as const) {
      expect(autoRemoteReason(kind, "https://openrouter.ai/api/v1")).toBeNull();
      expect(autoRemoteReason(kind, "http://localhost:11434/v1")).toBeNull();
    }
  });

  it("waits for a host to judge", () => {
    for (const partial of ["", "h", "http", "http://", "https://", "openrouter.ai/api/v1"]) {
      expect(autoRemoteReason("local", partial)).toBeNull();
    }
  });
});

describe("the origin a typed key belongs to", () => {
  it("is the scheme, host and port, lowercased", () => {
    expect(httpOrigin("https://OpenRouter.ai/api/v1")).toBe("https://openrouter.ai");
    expect(httpOrigin(" https://openrouter.ai/api/v2 ")).toBe("https://openrouter.ai");
    expect(httpOrigin("http://127.0.0.1:8080/v1")).toBe("http://127.0.0.1:8080");
    expect(httpOrigin("https://api.openai.com:443/v1")).toBe("https://api.openai.com");
    expect(httpOrigin("http://api.openai.com/v1")).not.toBe(httpOrigin("https://api.openai.com/v1"));
    expect(httpOrigin("http://127.0.0.1:8080/v1")).not.toBe(httpOrigin("http://127.0.0.1:9090/v1"));
  });

  it("is null for anything that is not an HTTP URL", () => {
    for (const raw of ["", "openrouter.ai/api/v1", "codex-cli://default", "http://"]) {
      expect(httpOrigin(raw)).toBeNull();
    }
  });
});
