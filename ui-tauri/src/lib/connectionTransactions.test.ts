import { describe, expect, it } from "vitest";

import {
  accountLegs,
  accountMatchesLabel,
  transactionBelongsToConnection,
} from "./connectionTransactions";

describe("accountLegs", () => {
  it("returns a single leg for an ordinary account", () => {
    expect(accountLegs("Cold Storage")).toEqual(["Cold Storage"]);
  });

  it("splits transfer accounts on the arrow separator", () => {
    expect(accountLegs("Cold Storage → Vault")).toEqual([
      "Cold Storage",
      "Vault",
    ]);
  });

  it("handles ascii arrow and trims whitespace", () => {
    expect(accountLegs("  NWC · Alby ->Cashu · minibits ")).toEqual([
      "NWC · Alby",
      "Cashu · minibits",
    ]);
  });

  it("returns an empty array for blank input", () => {
    expect(accountLegs("   ")).toEqual([]);
    expect(accountLegs(null)).toEqual([]);
    expect(accountLegs(undefined)).toEqual([]);
  });
});

describe("accountMatchesLabel", () => {
  it("matches an exact account, case-insensitively", () => {
    expect(accountMatchesLabel("Cold Storage", "cold storage")).toBe(true);
  });

  it("matches a transfer leg", () => {
    expect(accountMatchesLabel("Cold Storage → Vault", "Cold Storage")).toBe(
      true,
    );
    expect(accountMatchesLabel("Cold Storage → Vault", "Vault")).toBe(true);
  });

  it("does NOT match on substrings (the old .includes bug)", () => {
    expect(accountMatchesLabel("Coldcard backup", "Cold")).toBe(false);
    expect(accountMatchesLabel("Cold Storage", "Cold")).toBe(false);
  });

  it("is false for blank label or account", () => {
    expect(accountMatchesLabel("Cold Storage", "")).toBe(false);
    expect(accountMatchesLabel("", "Cold Storage")).toBe(false);
    expect(accountMatchesLabel(null, "Cold Storage")).toBe(false);
  });
});

describe("transactionBelongsToConnection", () => {
  it("includes transfers that touch the connection", () => {
    expect(
      transactionBelongsToConnection(
        { account: "Cold Storage → Vault" },
        { label: "Cold Storage" },
      ),
    ).toBe(true);
  });

  it("excludes unrelated wallets with a shared prefix", () => {
    expect(
      transactionBelongsToConnection(
        { account: "Coldcard backup" },
        { label: "Cold Storage" },
      ),
    ).toBe(false);
  });
});
