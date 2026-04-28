import { describe, expect, it } from "vitest";

import {
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
