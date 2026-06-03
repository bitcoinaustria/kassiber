import { describe, expect, it } from "vitest";

import { DEFAULT_FORM } from "./constants";
import {
  aiStepComplete,
  essentialsStepComplete,
  securityStepComplete,
  syncStepComplete,
} from "./gates";
import type { OnboardingForm } from "./types";

const form = (overrides: Partial<OnboardingForm>): OnboardingForm => ({
  ...DEFAULT_FORM,
  ...overrides,
});

describe("essentialsStepComplete", () => {
  it("accepts recommended defaults untouched", () => {
    expect(essentialsStepComplete(DEFAULT_FORM)).toBe(true);
  });

  it("requires the visible books label and hidden default book label", () => {
    expect(essentialsStepComplete(form({ workspace: "   " }))).toBe(false);
    expect(essentialsStepComplete(form({ profile: "" }))).toBe(false);
  });

  it("requires a valid long-term day count for generic books", () => {
    expect(
      essentialsStepComplete(
        form({ taxCountry: "generic", taxLongTermDays: "0" }),
      ),
    ).toBe(false);
    expect(
      essentialsStepComplete(
        form({ taxCountry: "generic", taxLongTermDays: "365" }),
      ),
    ).toBe(true);
  });
});

describe("syncStepComplete", () => {
  it("accepts the built-in default", () => {
    expect(syncStepComplete(DEFAULT_FORM)).toBe(true);
  });

  it("blocks a custom Electrum endpoint with no host", () => {
    expect(
      syncStepComplete(
        form({
          backendSetupMode: "custom",
          backendKind: "electrum",
          backendName: "home-node",
          backendHost: "",
        }),
      ),
    ).toBe(false);
  });

  it("requires the acknowledgement when skipping", () => {
    expect(
      syncStepComplete(
        form({ backendSetupMode: "skip", skipBackendsAcknowledged: false }),
      ),
    ).toBe(false);
    expect(
      syncStepComplete(
        form({ backendSetupMode: "skip", skipBackendsAcknowledged: true }),
      ),
    ).toBe(true);
  });
});

describe("aiStepComplete", () => {
  it("accepts the local default", () => {
    expect(aiStepComplete(DEFAULT_FORM)).toBe(true);
  });

  it("blocks an invalid remote AI base URL", () => {
    expect(
      aiStepComplete(
        form({
          aiSetupMode: "remote",
          aiProviderName: "openai",
          aiRemoteAcknowledged: true,
          aiBaseUrl: "not a url",
        }),
      ),
    ).toBe(false);
  });

  it("requires the acknowledgement for a remote provider", () => {
    expect(
      aiStepComplete(
        form({
          aiSetupMode: "remote",
          aiProviderName: "openai",
          aiBaseUrl: "https://api.example/v1",
          aiRemoteAcknowledged: false,
        }),
      ),
    ).toBe(false);
  });
});

describe("securityStepComplete", () => {
  it("requires the plaintext acknowledgement in plaintext mode", () => {
    expect(
      securityStepComplete(
        form({ databaseMode: "plaintext", plaintextAcknowledged: false }),
      ),
    ).toBe(false);
    expect(
      securityStepComplete(
        form({ databaseMode: "plaintext", plaintextAcknowledged: true }),
      ),
    ).toBe(true);
  });

  it("requires a confirmed passphrase and recovery ack when encrypted", () => {
    expect(securityStepComplete(form({ databaseMode: "sqlcipher" }))).toBe(
      false,
    );
    expect(
      securityStepComplete(
        form({
          databaseMode: "sqlcipher",
          databasePassphrase: "correct horse battery",
          databasePassphraseConfirm: "correct horse battery",
          recoveryAcknowledged: true,
        }),
      ),
    ).toBe(true);
  });
});
