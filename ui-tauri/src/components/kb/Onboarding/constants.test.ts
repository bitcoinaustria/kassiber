import { describe, expect, it } from "vitest";

import {
  aiBaseUrlHint,
  backendEndpointHint,
  databasePassphraseHint,
  gainsAlgorithmsFor,
  parseTaxLongTermDays,
  taxLongTermDaysHint,
} from "./constants";

describe("onboarding tax long-term day parsing", () => {
  it("accepts positive whole days with surrounding whitespace", () => {
    expect(parseTaxLongTermDays("365")).toBe(365);
    expect(parseTaxLongTermDays("  730  ")).toBe(730);
    expect(taxLongTermDaysHint("365")).toBeNull();
  });

  it("rejects truncated or non-integer values", () => {
    for (const value of ["365.5", "1e2", "12abc", "0x10", ""]) {
      expect(parseTaxLongTermDays(value)).toBeNull();
      expect(taxLongTermDaysHint(value)).not.toBeNull();
    }
  });

  it("rejects zero, negative, and unsafe integer values", () => {
    for (const value of ["0", "-1", String(Number.MAX_SAFE_INTEGER + 1)]) {
      expect(parseTaxLongTermDays(value)).toBeNull();
      expect(taxLongTermDaysHint(value)).not.toBeNull();
    }
  });
});

describe("onboarding endpoint validation", () => {
  it("accepts backend endpoint formats by kind", () => {
    expect(backendEndpointHint("esplora", "https://node.example/api")).toBeNull();
    expect(backendEndpointHint("btcpay", "http://127.0.0.1:23000")).toBeNull();
    expect(backendEndpointHint("electrum", "ssl://node.example:50002")).toBeNull();
    expect(backendEndpointHint("electrum", "node.example:50002")).toBeNull();
  });

  it("rejects backend endpoint formats that will not work", () => {
    expect(backendEndpointHint("esplora", "ssl://node.example:50002")).toBe(
      "Use an http:// or https:// URL.",
    );
    expect(backendEndpointHint("electrum", "https://node.example/api")).toBe(
      "Use ssl://host:50002, tcp://host:50001, or host:port.",
    );
    expect(backendEndpointHint("bitcoinrpc", "")).toBe("Endpoint is required.");
    expect(
      backendEndpointHint("bitcoinrpc", "http://rpcuser:rpcpass@127.0.0.1:8332"),
    ).toBe("Do not include usernames or passwords in the endpoint.");
    expect(backendEndpointHint("electrum", "ssl://user@node.example:50002")).toBe(
      "Do not include usernames or passwords in the endpoint.",
    );
  });

  it("validates OpenAI-compatible base URLs", () => {
    expect(aiBaseUrlHint("http://localhost:11434/v1")).toBeNull();
    expect(aiBaseUrlHint("https://api.example/v1")).toBeNull();
    expect(aiBaseUrlHint("claude-cli://default")).toBeNull();
    expect(aiBaseUrlHint("codex-cli://default")).toBeNull();
    expect(aiBaseUrlHint("")).toBe("Base URL is required.");
    expect(aiBaseUrlHint("ollama.local/v1")).toBe(
      "Use an http:// or https:// URL, or claude-cli://default / codex-cli://default.",
    );
    expect(aiBaseUrlHint("https://sk-secret@example.test/v1")).toBe(
      "Do not include usernames or passwords in the endpoint.",
    );
  });
});

describe("onboarding Austrian tax defaults", () => {
  it("does not offer legacy lot or holding-period choices for new AT wallets", () => {
    expect(gainsAlgorithmsFor("at")).toEqual(["MOVING_AVERAGE_AT"]);
  });
});

describe("onboarding database passphrase validation", () => {
  it("requires a long passphrase and matching confirmation", () => {
    expect(databasePassphraseHint("", "")).toBe("Enter a database passphrase.");
    expect(databasePassphraseHint("short", "short")).toBe(
      "Use at least 12 characters.",
    );
    expect(databasePassphraseHint("long enough value", "")).toBe(
      "Confirm the database passphrase.",
    );
    expect(databasePassphraseHint("long enough value", "different value")).toBe(
      "Passphrases do not match.",
    );
    expect(
      databasePassphraseHint("long enough value", "long enough value"),
    ).toBeNull();
  });
});
