import { describe, expect, it } from "vitest";

import {
  backendPayload,
  backendRowToSettingsBackend,
  type Backend,
} from "./SettingsModel";

describe("backend settings model", () => {
  it("keeps the stable backend id separate from the editable display name", () => {
    const backend = backendRowToSettingsBackend({
      name: "liquid",
      display_name: "My Liquid node",
      kind: "liquid-esplora",
      chain: "liquid",
      network: "liquidv1",
      url: "https://liquid.network/api",
      has_url: true,
      wallet_refs: ["Main/Default/Treasury"],
    });

    expect(backend.id).toBe("liquid");
    expect(backend.name).toBe("My Liquid node");
    expect(backend.walletRefs).toEqual(["Main/Default/Treasury"]);
  });

  it("updates display_name without renaming the backend key", () => {
    const payload = backendPayload({
      id: "liquid",
      name: "Desk Liquid indexer",
      url: "https://liquid.network/api",
      net: "LIQUID",
      kind: "liquid-esplora",
      chain: "liquid",
      network: "liquidv1",
      health: "configured",
      on: true,
      auth: "none",
    } satisfies Backend);

    expect(payload.name).toBe("liquid");
    expect(payload.config).toMatchObject({
      display_name: "Desk Liquid indexer",
    });
  });
});
