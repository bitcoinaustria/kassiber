import { describe, expect, it } from "vitest";
import { CONNECTION_SOURCES } from "@/lib/connectionCatalog";
import { knownWalletImportSource, isHistoryImportSource, sourceForConnectionCategory } from "./connectionImportSource";

describe("known history import source", () => {
  it("selects the supported exact parser for a canonical Strike or River wallet", () => {
    expect(knownWalletImportSource("strike", CONNECTION_SOURCES)).toMatchObject({
      id: "strike", category: "exchanges", sourceFormat: "strike_csv", setupKind: "file-wallet",
    });
    expect(knownWalletImportSource("river", CONNECTION_SOURCES)?.sourceFormat).toBe("river_csv");
  });
  it("keeps the chooser for generic, unknown and unsupported wallet kinds", () => {
    for (const kind of [undefined, "", "custom", "address", "descriptor", "kraken", "unknown", "Strike"])
      expect(knownWalletImportSource(kind, CONNECTION_SOURCES)).toBeNull();
    // Even a reduced catalog cannot turn a generic custom wallet into evidence
    // that a particular CSV parser matches its export.
    expect(knownWalletImportSource("custom", CONNECTION_SOURCES.filter((source) => source.id === "csv"))).toBeNull();
  });
  it("declines ambiguous, planned, redirected and non-import matches", () => {
    const strike = CONNECTION_SOURCES.find((source) => source.id === "strike")!;
    expect(knownWalletImportSource("strike", [strike, { ...strike, id: "another", sourceFormat: "csv" }])).toBeNull();
    expect(knownWalletImportSource("strike", [{ ...strike, status: "planned" }])).toBeNull();
    expect(knownWalletImportSource("strike", [{ ...strike, forwardTo: "csv" }])).toBeNull();
    expect(knownWalletImportSource("strike", [{ ...strike, setupKind: "descriptor" }])).toBeNull();
  });
  it("never selects the hidden descriptor from an import category", () => {
    const descriptor = CONNECTION_SOURCES.find((source) => source.id === "descriptor")!;
    expect(descriptor.status).toBe("ready");
    expect(sourceForConnectionCategory(CONNECTION_SOURCES, "wallets", false)?.id).toBe("descriptor");
    const importedWalletSource = sourceForConnectionCategory(CONNECTION_SOURCES, "wallets", true);
    expect(importedWalletSource).not.toBeNull();
    expect(importedWalletSource?.id).not.toBe("descriptor");
    expect(isHistoryImportSource(importedWalletSource!)).toBe(true);
    expect(isHistoryImportSource(descriptor)).toBe(false);
    expect(sourceForConnectionCategory(CONNECTION_SOURCES, "nodes", true)).toBeNull();
    expect(sourceForConnectionCategory(CONNECTION_SOURCES, "files", true)?.sourceFormat).toBeTruthy();
  });

});
