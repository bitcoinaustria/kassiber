import { describe, expect, it } from "vitest";

import {
  AUDIT_PACKAGE_EXPORT_KIND,
  HANDOFF_EXPORT_MODES,
  NORMAL_HANDOFF_EXCLUSIONS,
  handoffModeById,
  normalHandoffModes,
  requiresSensitiveWalletMaterialConsent,
} from "./handoffExports";

describe("handoff export modes", () => {
  it("keeps normal handoff exports free of wallet surveillance material", () => {
    const normalModes = normalHandoffModes();

    expect(normalModes.map((mode) => mode.id)).toEqual([
      "tax_advisor_report",
      "audit_package",
    ]);
    for (const mode of normalModes) {
      expect(requiresSensitiveWalletMaterialConsent(mode)).toBe(false);
      for (const excludedLabel of NORMAL_HANDOFF_EXCLUSIONS) {
        expect(mode.excludes).toContain(excludedLabel);
      }
    }
  });

  it("keeps technical wallet evidence outside the normal export path", () => {
    const technical = handoffModeById("technical_wallet_evidence");

    expect(technical).toBeDefined();
    expect(technical?.availability).toBe("restricted");
    expect(technical?.walletMaterialPolicy).toBe("requires_explicit_consent");
    expect(
      technical ? requiresSensitiveWalletMaterialConsent(technical) : false,
    ).toBe(true);
    expect(normalHandoffModes()).not.toContain(technical);
  });

  it("marks audit package available only with its real daemon export kind", () => {
    const audit = handoffModeById("audit_package");

    expect(audit?.availability).toBe("available");
    expect(audit?.daemonKind).toBe(AUDIT_PACKAGE_EXPORT_KIND);
  });

  it("does not add another unrestricted export mode by accident", () => {
    expect(HANDOFF_EXPORT_MODES).toHaveLength(3);
    expect(
      HANDOFF_EXPORT_MODES.filter(
        (mode) => mode.walletMaterialPolicy !== "excluded",
      ).map((mode) => mode.id),
    ).toEqual(["technical_wallet_evidence"]);
  });
});
