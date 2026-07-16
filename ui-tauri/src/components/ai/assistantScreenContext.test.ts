import { describe, expect, it } from "vitest";

import { assistantScreenContextFor } from "./assistantScreenContext";

describe("assistantScreenContextFor", () => {
  it("includes the selected transaction and allowlisted display filters", () => {
    expect(
      assistantScreenContextFor(
        "/transactions",
        "?tx=tx-123&tab=pricing&quick=missing_price&wallet=ignored",
      ),
    ).toEqual({
      route: "/transactions",
      entityType: "transaction",
      entityId: "tx-123",
      filters: { tab: "pricing", quick: "missing_price" },
      capabilities: ["transactions"],
    });
  });

  it("never forwards path, URL, or arbitrary renderer query values", () => {
    const screenContext = assistantScreenContextFor(
      "/transactions",
      "?tx=https%3A%2F%2Fevil.example%2Ftx&tab=not-a-tab&file_path=%2FUsers%2Fme%2Fsecret&url=https%3A%2F%2Fevil.example",
    );

    expect(screenContext).toEqual({
      route: "/transactions",
      capabilities: ["transactions"],
      filters: undefined,
    });
    expect(JSON.stringify(screenContext)).not.toContain("evil.example");
    expect(JSON.stringify(screenContext)).not.toContain("/Users/");
  });

  it("maps specialist screens onto the matching capability packs", () => {
    expect(assistantScreenContextFor("/privacy-mirror")).toMatchObject({
      route: "/privacy-mirror",
      capabilities: ["privacy"],
    });
    expect(assistantScreenContextFor("/imports")).toMatchObject({
      route: "/imports",
      capabilities: ["wallets", "merchant", "transactions"],
    });
    expect(assistantScreenContextFor("/books")).toMatchObject({
      route: "/books",
      capabilities: ["wallets", "reports", "operations"],
    });
    expect(
      assistantScreenContextFor("/custody-gaps", "?gap=custody-gap%3A123"),
    ).toMatchObject({
      route: "/custody-gaps",
      entityType: "custody_gap",
      entityId: "custody-gap:123",
      capabilities: ["transfers", "transactions", "wallets"],
    });
    expect(
      assistantScreenContextFor(
        "/exit-tax",
        "?destination=eu_eea&departure_date=2026-07-10",
      ),
    ).toMatchObject({
      route: "/exit-tax",
      entityType: "report",
      entityId: "exit-tax",
      filters: {
        destination: "eu_eea",
        departure_date: "2026-07-10",
      },
      capabilities: ["reports"],
    });
  });

  it("canonicalizes safe connection detail ids without forwarding the route segment", () => {
    expect(assistantScreenContextFor("/connections/wallet-7")).toEqual({
      route: "/connections",
      entityType: "connection",
      entityId: "wallet-7",
      capabilities: ["wallets", "operations"],
    });
    expect(
      assistantScreenContextFor(
        "/connections/https%3A%2F%2Fevil.example%2Fwallet",
      ),
    ).toEqual({
      route: "/connections",
      capabilities: ["wallets", "operations"],
    });
  });

  it("bounds numeric report filters and transaction-focused swap context", () => {
    expect(assistantScreenContextFor("/reports", "?year=2025")).toEqual({
      route: "/reports",
      filters: { year: 2025 },
      capabilities: ["reports"],
    });
    expect(assistantScreenContextFor("/reports", "?year=99999")).toEqual({
      route: "/reports",
      filters: undefined,
      capabilities: ["reports"],
    });
    expect(
      assistantScreenContextFor(
        "/swaps",
        "?focus=tx-swap&method=ownership_graph",
      ),
    ).toEqual({
      route: "/swaps",
      entityType: "transaction",
      entityId: "tx-swap",
      filters: { method: "ownership_graph" },
      capabilities: ["transfers", "transactions"],
    });
  });
});
