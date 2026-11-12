import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

import { DEFAULT_FORM } from "../constants";
import { SecurityStep } from "./SecurityStep";

const renderStep = (passphrase: string, confirmation: string) =>
  renderToStaticMarkup(
    <SecurityStep
      form={{
        ...DEFAULT_FORM,
        databaseMode: "sqlcipher",
        databasePassphrase: passphrase,
        databasePassphraseConfirm: confirmation,
      }}
      update={vi.fn()}
      onSubmit={vi.fn()}
      currentStep={3}
      totalSteps={5}
      backendPreviewRows={[]}
    />,
  );

afterEach(async () => {
  await i18n.changeLanguage("en");
});

describe("SecurityStep", () => {
  it("shows the passphrase hint in the active language", async () => {
    expect(renderStep("short", "short")).toContain(
      "Use at least 12 characters.",
    );

    await i18n.changeLanguage("de");
    const html = renderStep("short", "short");
    expect(html).toContain("Verwende mindestens 12 Zeichen.");
    expect(html).not.toContain("Use at least");
  });

  it("asks the user to have stored the passphrase", () => {
    const html = renderStep("", "");

    expect(html).toContain(
      "Encryption protects your books on this device with a passphrase.",
    );
    expect(html).toContain(
      "I have stored this passphrase somewhere safe, such as a password manager.",
    );
    expect(html).not.toContain("SQLCipher");
  });

  it("notes a weak passphrase, not a strong one", () => {
    expect(renderStep("aaaaaaaaaaaa", "")).toContain("Easy to guess.");
    expect(renderStep("correct horse battery", "")).not.toContain(
      "Easy to guess.",
    );
  });
});
