import { describe, expect, it } from "vitest";

import { autoRemoteReason } from "./aiProviderFormModel";

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
