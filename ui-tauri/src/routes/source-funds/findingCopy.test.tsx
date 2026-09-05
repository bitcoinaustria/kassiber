import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";
import i18n from "@/i18n";
import { GateRow } from "./panels";
import { findingTranslationKeys } from "./findingCopy";
import type { SourceFundsFinding } from "./model";

const missing: SourceFundsFinding = {
  code: "missing_history", severity: "blocker",
  message: "The path stops at a transaction without a reviewed root source or missing-history attestation.",
  next_step: { headline: "Attach a root source or attest the gap", action: "open_source_creator" },
};
afterEach(async () => { await i18n.changeLanguage("en"); });
describe("canonical finding localization", () => {
  it("renders the visible missing-history finding and next action in German", async () => {
    await i18n.changeLanguage("de");
    const html = renderToStaticMarkup(<GateRow finding={missing} onAction={() => {}} />);
    expect(html).toContain("Fehlende Historie");
    expect(html).toContain("ohne geprüfte Ursprungsquelle");
    expect(html).toContain("Ursprungsquelle verknüpfen oder die Lücke bestätigen");
    expect(html).toContain("Diese Lücke dokumentieren");
    expect(html).not.toContain("The path stops"); expect(html).not.toContain("Attach a root source");
  });
  it("distinguishes a documented gap from a missing source", async () => {
    await i18n.changeLanguage("de");
    const attested = { ...missing, severity: "warning", message: "Reviewed missing-history gap included; it is not a real root source." } as SourceFundsFinding;
    expect(findingTranslationKeys(attested).message).not.toBe(findingTranslationKeys(missing).message);
    const html = renderToStaticMarkup(<GateRow finding={attested} />);
    expect(html).toContain("keine tatsächliche Ursprungsquelle");
    expect(html).not.toContain("ohne geprüfte Ursprungsquelle");
  });
  it("localizes a selected amount that cannot follow the exact reviewed ratio", async () => {
    await i18n.changeLanguage("de");
    const finding = { ...missing, code: "ambiguous_allocation", message: "The selected amount cannot follow the reviewed allocation ratio in exact millisatoshis." };
    expect(renderToStaticMarkup(<GateRow finding={finding} />)).toContain("in exakten Millisatoshis abbilden");
    expect(findingTranslationKeys(finding).message).toBe("findings.ambiguous_allocation.messages.selectedAmountNotExactlyRepresentable");
  });
  it("preserves unknown or changed backend details instead of replacing their meaning", async () => {
    await i18n.changeLanguage("de");
    for (const code of ["missing_history", "future_finding"]) {
      const finding = { ...missing, code, message: "A new precise diagnostic with ID 123", next_step: { headline: "A newly supplied instruction", action: "open_source_creator" } };
      expect(findingTranslationKeys(finding).message).toBeUndefined();
      expect(findingTranslationKeys(finding).nextStep).toBeUndefined();
      const html = renderToStaticMarkup(<GateRow finding={finding} />);
      expect(html).toContain(finding.message); expect(html).toContain(finding.next_step.headline);
    }
  });
});
