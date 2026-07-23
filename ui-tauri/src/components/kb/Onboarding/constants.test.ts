import { describe, expect, it } from "vitest";

import {
  DEFAULT_FORM,
  aiBaseUrlHint,
  backendEndpointHint,
  localAiBaseUrlHint,
  databasePassphraseHint,
  electrumEndpointUrl,
  GAINS_ALGORITHM_DEFAULTS,
  gainsAlgorithmsFor,
  parseTaxLongTermDays,
  taxLongTermDaysHint,
} from "./constants";

describe("onboarding update privacy", () => {
  it("records an explicit update-check choice in the setup form", () => {
    expect(DEFAULT_FORM.updateChecksEnabled).toBe(true);
  });
});

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
    expect(
      backendEndpointHint("esplora", "https://node.example/api"),
    ).toBeNull();
    expect(
      backendEndpointHint("liquid-esplora", "https://liquid.example/api"),
    ).toBeNull();
    expect(
      backendEndpointHint("electrum", "ssl://node.example:50002"),
    ).toBeNull();
    expect(backendEndpointHint("electrum", "node.example:50002")).toBeNull();
  });

  it("builds electrum endpoints from Sparrow-style host and port fields", () => {
    expect(
      electrumEndpointUrl({
        host: "index.bitcoin-austria.at",
        port: "50002",
        useSsl: true,
      }),
    ).toBe("ssl://index.bitcoin-austria.at:50002");
    expect(
      electrumEndpointUrl({
        host: "127.0.0.1",
        port: "50001",
        useSsl: false,
      }),
    ).toBe("tcp://127.0.0.1:50001");
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
      backendEndpointHint(
        "bitcoinrpc",
        "http://rpcuser:rpcpass@127.0.0.1:8332",
      ),
    ).toBe("Do not include usernames or passwords in the endpoint.");
    expect(
      backendEndpointHint("electrum", "ssl://user@node.example:50002"),
    ).toBe("Do not include usernames or passwords in the endpoint.");
  });

  it("validates OpenAI Responses API-compatible base URLs", () => {
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

  it("requires loopback endpoints for local AI mode", () => {
    expect(localAiBaseUrlHint("http://localhost:11434/v1")).toBeNull();
    expect(localAiBaseUrlHint("http://127.0.0.1:11434/v1")).toBeNull();
    expect(localAiBaseUrlHint("http://[::1]:11434/v1")).toBeNull();
    expect(localAiBaseUrlHint("https://api.example/v1")).toBe(
      "Local AI providers must use http://localhost, http://127.0.0.1, or http://[::1]. Use remote mode for off-device endpoints.",
    );
    expect(localAiBaseUrlHint("claude-cli://default")).toBe(
      "Local AI providers must use http://localhost, http://127.0.0.1, or http://[::1]. Use remote mode for off-device endpoints.",
    );
  });
});

describe("onboarding Austrian tax defaults", () => {
  it("offers every method for AT with the moving-average default listed first", () => {
    // Austrian books default to the moving-average method (gleitender
    // Durchschnittspreis) but may also use the generic methods, so the AT list
    // is the union with the moving-average default at index 0.
    expect(gainsAlgorithmsFor("at")).toEqual([
      "MOVING_AVERAGE_AT",
      "FIFO",
      "LIFO",
      "HIFO",
      "LOFO",
    ]);
    expect(gainsAlgorithmsFor("at")[0]).toBe(GAINS_ALGORITHM_DEFAULTS.at);
  });

  it("offers the lot methods plus plain moving-average for the generic region", () => {
    expect(gainsAlgorithmsFor("generic")).toEqual([
      "FIFO",
      "LIFO",
      "HIFO",
      "LOFO",
      "MOVING_AVERAGE",
    ]);
    // The Austrian list keeps the AT moving-average variant and the lot methods,
    // but NOT the plain generic moving-average.
    expect(gainsAlgorithmsFor("at")).not.toContain("MOVING_AVERAGE");
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
